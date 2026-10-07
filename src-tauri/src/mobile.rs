use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicBool, AtomicI64, Ordering}, Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState, Schedule};
use tauri_plugin_sitzfleisch_android::SitzfleischAndroidExt;

use crate::{alarm_store, alerts::{self, AppliedAlarm, StatusModel}, android_shutdown::HeartbeatShutdown,
    platform::{AppBlockRules, InstalledApps, SettingsRequest, SystemStatus}, Shared};

/// 同步失败后，在这段时间内只在状态真的变了（dirty）时才重试，不再每秒去敲原生接口。
const RETRY_SECONDS: i64 = 30;

struct SyncState {
    applied: Vec<AppliedAlarm>,
    status: Option<StatusModel>,
    /// 上一次成功推给原生屏蔽服务的规则；进程刚起来时为空，首轮同步一定推一次。
    block_rules: Option<AppBlockRules>,
    cold: bool,
    last_error: Option<String>,
    retry_after: i64,
}

/// 移动端状态独立；通知和闹钟共用一把专用锁，只由心跳执行原生同步。
pub struct MobileShared {
    dirty: AtomicBool,
    /// 回到前台时置位：常驻通知可能被用户划掉了，下一次同步要重新发一遍。
    status_stale: AtomicBool,
    suspended: AtomicBool,
    /// 心跳上一次真正运行的时刻。判断「这一轮到期的提醒是不是早该发过」只看它，
    /// 不看会被命令和回前台取快照推进的 last_tick。
    last_heartbeat: AtomicI64,
    heartbeat: Mutex<Option<Thread>>,
    shutdown: Arc<HeartbeatShutdown>,
    synchronizer: Mutex<SyncState>,
    alarms_path: PathBuf,
}

impl MobileShared {
    pub fn new(alarms_path: PathBuf) -> Self {
        remove_stale_temporaries(&alarms_path);
        let applied = match alarm_store::load(&alarms_path) {
            Ok(applied) => applied,
            Err(_) => {
                // 账本只是本应用私有目录里的缓存：读不了（旧格式、损坏）就挪到一边，
                // 从规则重建并继续记账；冷启动会按当前状态重新确认全部未来提醒。
                let aside = alarms_path.with_extension("json.bad");
                if fs::rename(&alarms_path, &aside).is_err() {
                    eprintln!("闹钟记录无法读取，也无法移到一旁；本次按当前状态重建排程。");
                }
                Vec::new()
            },
        };
        Self {
            dirty: AtomicBool::new(true),
            status_stale: AtomicBool::new(false),
            suspended: AtomicBool::new(false),
            last_heartbeat: AtomicI64::new(0),
            heartbeat: Mutex::new(None), alarms_path,
            shutdown: Arc::new(HeartbeatShutdown::new()),
            synchronizer: Mutex::new(SyncState { applied, status: None, block_rules: None, cold: true,
                last_error: None, retry_after: 0 }),
        }
    }
}

/// 进程在写临时文件和改名之间被杀时会留下 `.alarms.*.tmp`；启动时顺手清掉。
fn remove_stale_temporaries(alarms_path: &Path) {
    let Some(parent) = alarms_path.parent() else { return };
    let Ok(entries) = fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".alarms.") && name.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn plugin_call<T, E: std::fmt::Display>(call: impl FnOnce() -> Result<T, E>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(call))
        .map_err(|_| "Android 系统接口暂不可用，请回到前台后重试。".to_owned())?
        .map_err(|error| error.to_string())
}

pub fn set_heartbeat_thread(app: &AppHandle) {
    *app.state::<MobileShared>().heartbeat.lock().unwrap() = Some(thread::current());
}

pub fn register_heartbeat_worker(app: &AppHandle, worker: JoinHandle<()>) {
    app.state::<MobileShared>().shutdown.register_worker(worker);
}

pub fn stop_requested(app: &AppHandle) -> bool {
    app.state::<MobileShared>().shutdown.stop_requested()
}

pub fn prevent_exit_until_heartbeat_stops(app: &AppHandle) -> bool {
    app.state::<Shared>().exiting.store(true, Ordering::SeqCst);
    let handle = app.clone();
    app.state::<MobileShared>().shutdown.request_exit(move || handle.exit(0))
}

pub fn mark_dirty(app: &AppHandle) {
    let mobile = app.state::<MobileShared>();
    mobile.dirty.store(true, Ordering::SeqCst);
    let heartbeat = mobile.heartbeat.lock().unwrap().clone();
    if let Some(thread) = heartbeat { thread.unpark(); }
}

pub fn is_suspended(app: &AppHandle) -> bool {
    app.state::<MobileShared>().suspended.load(Ordering::SeqCst)
}

/// 心跳每轮调用一次：返回距上一轮心跳的秒数，并记下这一轮。首轮返回一个很大的值。
pub fn heartbeat_gap(app: &AppHandle, now: i64) -> i64 {
    let previous = app.state::<MobileShared>().last_heartbeat.swap(now, Ordering::SeqCst);
    now.saturating_sub(previous)
}

/// 命令推进时间时用：只读，不算作一次心跳。
pub fn gap_since_heartbeat(app: &AppHandle, now: i64) -> i64 {
    now.saturating_sub(app.state::<MobileShared>().last_heartbeat.load(Ordering::SeqCst))
}

pub fn on_lifecycle(app: &AppHandle, suspended: bool) {
    let mobile = app.state::<MobileShared>();
    mobile.suspended.store(suspended, Ordering::SeqCst);
    if suspended {
        crate::save(&app.state::<Shared>());
    } else {
        mobile.status_stale.store(true, Ordering::SeqCst);
    }
    mark_dirty(app);
}

fn wall_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs() as i64).unwrap_or(0)
}

fn persist(mobile: &MobileShared, ledger: &[AppliedAlarm]) -> Result<(), String> {
    alarm_store::save(&mobile.alarms_path, ledger).map_err(|_| "闹钟记录写入失败，将稍后重试。".to_owned())
}

fn sync_alarms(app: &AppHandle, mobile: &MobileShared, state: &mut SyncState,
    desired: &[AppliedAlarm], now: i64) -> Result<(), String> {
    // 先退出已触发的本地项再补充 horizon 尾部，避免「170 个旧项 + 一个新项」超出存档上限。
    let before = state.applied.len();
    state.applied.retain(|saved| saved.alert.at > now.saturating_add(1));
    let pruned = before != state.applied.len();
    let mut delta = alerts::diff(&state.applied, desired, now);
    // force-stop 和重启清掉系统闹钟，却不清掉应用文件。冷启动保留旧账本用于精确取消，
    // 只重新确认未来项，不清空 ID，也不触碰已显示或一秒内即将显示的通知。
    if state.cold {
        for item in desired.iter().filter(|item| item.alert.at > now.saturating_add(1)) {
            if !delta.schedule.contains(item) { delta.schedule.push(item.clone()); }
        }
    }
    if delta.cancel.is_empty() && delta.schedule.is_empty() {
        if pruned { persist(mobile, &state.applied)?; }
        state.cold = false;
        return Ok(());
    }
    // 先记账再调原生：进程若在中途被杀，冷启动仍能凭账本取消多余的项、重新确认缺的项。
    // 整批只落两次盘，不再每排一项就重写一次文件。
    let mut ahead = state.applied.clone();
    for item in &delta.schedule {
        ahead.retain(|saved| saved.id != item.id);
        ahead.push(item.clone());
    }
    persist(mobile, &ahead)?;
    for item in delta.cancel {
        // 原生调用可能排队；再次检查边界，不能把这期间刚弹出的横幅撤掉。
        if item.alert.at > wall_seconds().saturating_add(1) {
            plugin_call(|| app.notification().cancel(vec![item.id]))?;
        }
        state.applied.retain(|saved| saved != &item);
    }
    let mut outcome = Ok(());
    if !delta.schedule.is_empty() {
        if plugin_call(|| app.notification().permission_state())? != PermissionState::Granted {
            outcome = Err("通知未开启，暂不预排系统提醒。".to_owned());
        } else {
            // 精确闹钟被关掉时照样排：通知插件会改用非精确的系统闹钟，可能晚到，但不会一条都不响。
            // 今天页和「设置 → 提醒」会提示用户去打开。
            for item in delta.schedule {
                if item.alert.at <= wall_seconds().saturating_add(1) { continue; }
                let Ok(date) = alerts::schedule_date(item.alert.at) else {
                    outcome = Err("提醒时刻超出系统范围。".to_owned());
                    break;
                };
                let shown = plugin_call(|| app.notification().builder().id(item.id).channel_id(&item.channel)
                    .title(&item.title).body(&item.body).icon("ic_stat_zuogong").auto_cancel()
                    .schedule(Schedule::At { date, repeating: false, allow_while_idle: true }).show());
                if let Err(error) = shown { outcome = Err(error); break; }
                state.applied.retain(|saved| saved.id != item.id);
                state.applied.push(item);
            }
        }
    }
    // 收尾：账本只留真正排上、而且还没到点的项。
    let cutoff = wall_seconds().saturating_add(1);
    state.applied.retain(|saved| saved.alert.at > cutoff);
    persist(mobile, &state.applied)?;
    if outcome.is_ok() { state.cold = false; }
    outcome
}

/// 所有参数在心跳的 take_due_* 之后计算；进入本函数前已释放 Shared 的全部锁。
/// `block_rules` 为 None 表示这次不推（保护模式下内存里的偏好不是用户的真实设置）。
pub fn sync(app: &AppHandle, desired: Vec<AppliedAlarm>, desired_status: StatusModel,
    block_rules: Option<AppBlockRules>, now: i64) {
    let mobile = app.state::<MobileShared>();
    let forced = mobile.dirty.swap(false, Ordering::SeqCst);
    let mut state = mobile.synchronizer.lock().unwrap();
    if mobile.status_stale.swap(false, Ordering::SeqCst) { state.status = None; }
    // 上次失败后先等一等；状态有变化（开格、回前台、刚授权、改了屏蔽名单）就立刻重试。
    if !forced && now < state.retry_after { return; }
    // 屏蔽规则先推：它不依赖通知权限，不能被提醒那边的失败拖住。
    let mut rules_result = Ok(());
    if let Some(rules) = block_rules.filter(|rules| state.block_rules.as_ref() != Some(rules)) {
        rules_result = plugin_call(|| app.sitzfleisch_android().set_block_rules(&rules));
        if rules_result.is_ok() { state.block_rules = Some(rules); }
    }
    let mut result = sync_alarms(app, &mobile, &mut state, &desired, now);
    if result.is_ok() { result = rules_result; }
    if state.status.as_ref() != Some(&desired_status) {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|value| i64::try_from(value.as_millis()).unwrap_or(i64::MAX)).unwrap_or(0);
        let updated = plugin_call(|| app.sitzfleisch_android().update_status(desired_status.update(now_ms)));
        if updated.is_ok() { state.status = Some(desired_status); }
        if result.is_ok() { result = updated; }
    }
    match result {
        Ok(()) => {
            state.last_error = None;
            state.retry_after = 0;
        },
        Err(error) => {
            if state.last_error.as_ref() != Some(&error) { eprintln!("移动端提醒同步暂未完成：{error}"); }
            state.last_error = Some(error);
            state.retry_after = now.saturating_add(RETRY_SECONDS);
        },
    }
}

pub fn system_status(app: &AppHandle) -> Result<SystemStatus, String> {
    plugin_call(|| app.sitzfleisch_android().system_status())
}

pub fn open_settings(app: &AppHandle, request: SettingsRequest) -> Result<(), String> {
    plugin_call(|| app.sitzfleisch_android().open_settings(request))
}

pub fn move_task_to_back(app: &AppHandle) -> Result<(), String> {
    plugin_call(|| app.sitzfleisch_android().move_task_to_back())
}

pub fn installed_apps(app: &AppHandle) -> Result<InstalledApps, String> {
    plugin_call(|| app.sitzfleisch_android().installed_apps::<InstalledApps>()).map(InstalledApps::sanitized)
}

pub fn request_app_list_permission(app: &AppHandle) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Answer { granted: bool }
    plugin_call(|| app.sitzfleisch_android().request_app_list_permission::<Answer>()).map(|answer| answer.granted)
}

/// 屏蔽服务刚把人送回坐功时记下的应用包名；取一次就清掉。
pub fn take_block_notice(app: &AppHandle) -> Result<Option<String>, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Notice { package_name: Option<String> }
    plugin_call(|| app.sitzfleisch_android().take_block_notice::<Notice>())
        .map(|notice| notice.package_name.filter(|name| sitzfleisch_core::valid_package_name(name)))
}

fn permission_name(permission: PermissionState) -> String {
    match permission {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        _ => "unknown",
    }.into()
}

pub fn notification_status(app: &AppHandle) -> Result<String, String> {
    plugin_call(|| app.notification().permission_state()).map(permission_name)
}

pub fn request_notification_permission(app: &AppHandle) -> Result<String, String> {
    let before = plugin_call(|| app.notification().permission_state())?;
    // 官方插件在 Android 13+ 已授权时再请求会一直等待；必须先查再决定是否请求。
    let permission = if before == PermissionState::Granted { before }
        else { plugin_call(|| app.notification().request_permission())? };
    mark_dirty(app);
    Ok(permission_name(permission))
}

pub fn test_notification(app: &AppHandle, water: bool) -> Result<(), String> {
    if plugin_call(|| app.notification().permission_state())? != PermissionState::Granted {
        return Err("通知未开启，请先允许坐功发送通知。".into());
    }
    let (id, channel, title, body) = if water {
        (9101, "water", "喝点水吧", "忙了一阵，喝几口水再继续。")
    } else {
        (9100, "timer", "坐功 · 试一条", "看到这条横幅，说明系统通知这条路是通的。")
    };
    plugin_call(|| app.notification().builder().id(id).channel_id(channel)
        .title(title).body(body).icon("ic_stat_zuogong").auto_cancel().show())
}
