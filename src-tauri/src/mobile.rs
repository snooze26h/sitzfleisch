use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState, Schedule};
use tauri_plugin_sitzfleisch_android::SitzfleischAndroidExt;

use crate::{alarm_store, alerts::{self, AppliedAlarm, StatusModel}, android_shutdown::HeartbeatShutdown, platform::{SettingsRequest, SystemStatus}, Shared};

struct SyncState {
    applied: Vec<AppliedAlarm>,
    status: Option<StatusModel>,
    cold: bool,
    persist_dirty: bool,
    persist_enabled: bool,
    last_error: Option<String>,
}

/// 移动端状态独立；通知和闹钟共用一把专用锁，只由心跳执行原生同步。
pub struct MobileShared {
    dirty: AtomicBool,
    suspended: AtomicBool,
    heartbeat: Mutex<Option<Thread>>,
    shutdown: Arc<HeartbeatShutdown>,
    synchronizer: Mutex<SyncState>,
    alarms_path: PathBuf,
}

impl MobileShared {
    pub fn new(alarms_path: PathBuf) -> Self {
        let (applied, persist_enabled) = match alarm_store::load(&alarms_path) {
            Ok(applied) => (applied, true),
            Err(_) => {
                // 不覆盖损坏或链接到其他位置的记录；仍可从规则重建本次内存排程。
                eprintln!("闹钟记录无法安全读取，保留原文件，本次仅在内存中重建排程。");
                (Vec::new(), false)
            },
        };
        Self {
            dirty: AtomicBool::new(true), suspended: AtomicBool::new(false),
            heartbeat: Mutex::new(None), alarms_path,
            shutdown: Arc::new(HeartbeatShutdown::new()),
            synchronizer: Mutex::new(SyncState { applied, status: None, cold: true,
                persist_dirty: false, persist_enabled, last_error: None }),
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

pub fn on_lifecycle(app: &AppHandle, suspended: bool) {
    app.state::<MobileShared>().suspended.store(suspended, Ordering::SeqCst);
    if suspended { crate::save(&app.state::<Shared>()); }
    mark_dirty(app);
}

fn wall_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs() as i64).unwrap_or(0)
}

fn persist(mobile: &MobileShared, state: &mut SyncState) -> Result<(), String> {
    if state.persist_dirty && state.persist_enabled {
        alarm_store::save(&mobile.alarms_path, &state.applied)
            .map_err(|_| "闹钟记录写入失败，将在下一轮重试。".to_owned())?;
        state.persist_dirty = false;
    }
    Ok(())
}

fn sync_alarms(app: &AppHandle, mobile: &MobileShared, state: &mut SyncState,
    desired: &[AppliedAlarm], now: i64) -> Result<(), String> {
    // 先退出已触发的本地项再补充 horizon 尾部，避免“170 个旧项 + 一个新项”超出存档上限。
    let before = state.applied.len();
    state.applied.retain(|saved| saved.alert.at > now.saturating_add(1));
    state.persist_dirty |= before != state.applied.len();
    persist(mobile, state)?;
    let mut delta = alerts::diff(&state.applied, desired, now);
    // force-stop 和重启清掉系统闹钟，却不清掉应用文件。冷启动保留旧账本用于精确取消，
    // 只重新确认未来项，不清空 ID，也不触碰已显示或一秒内即将显示的通知。
    if state.cold {
        for item in desired.iter().filter(|item| item.alert.at > now.saturating_add(1)) {
            if !delta.schedule.contains(item) { delta.schedule.push(item.clone()); }
        }
    }
    for item in delta.cancel {
        // 原生调用可能排队；再次检查边界，不能把这期间刚弹出的横幅撤掉。
        if item.alert.at > wall_seconds().saturating_add(1) {
            plugin_call(|| app.notification().cancel(vec![item.id]))?;
        }
        state.applied.retain(|saved| saved != &item);
        state.persist_dirty = true;
        persist(mobile, state)?;
    }
    if !delta.schedule.is_empty() {
        if plugin_call(|| app.notification().permission_state())? != PermissionState::Granted {
            return Err("通知未开启，暂不预排系统提醒。".into());
        }
        let system: SystemStatus = plugin_call(|| app.sitzfleisch_android().system_status())?;
        if !system.can_schedule_exact_alarms {
            return Err("精确闹钟未开启，暂不退化为可能延迟的非精确提醒。".into());
        }
    }
    for item in delta.schedule {
        if item.alert.at <= wall_seconds().saturating_add(1) { continue; }
        let date = alerts::schedule_date(item.alert.at).map_err(|_| "提醒时刻超出系统范围。".to_owned())?;
        plugin_call(|| app.notification().builder().id(item.id).channel_id(&item.channel)
            .title(&item.title).body(&item.body).icon("ic_stat_zuogong")
            .schedule(Schedule::At { date, repeating: false, allow_while_idle: true }).show())?;
        state.applied.retain(|saved| saved.id != item.id);
        state.applied.push(item);
        state.persist_dirty = true;
        persist(mobile, state)?;
    }
    let cutoff = wall_seconds().saturating_add(1);
    let before = state.applied.len();
    state.applied.retain(|saved| saved.alert.at > cutoff);
    state.persist_dirty |= before != state.applied.len();
    persist(mobile, state)?;
    state.cold = false;
    Ok(())
}

/// 所有参数在心跳的 take_due_* 之后计算；进入本函数前已释放 Shared 的全部锁。
pub fn sync(app: &AppHandle, desired: Vec<AppliedAlarm>, desired_status: StatusModel, now: i64) {
    let mobile = app.state::<MobileShared>();
    mobile.dirty.swap(false, Ordering::SeqCst);
    let mut state = mobile.synchronizer.lock().unwrap();
    let mut result = sync_alarms(app, &mobile, &mut state, &desired, now);
    if state.status.as_ref() != Some(&desired_status) {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|value| i64::try_from(value.as_millis()).unwrap_or(i64::MAX)).unwrap_or(0);
        let updated = plugin_call(|| app.sitzfleisch_android().update_status(desired_status.update(now_ms)));
        if updated.is_ok() { state.status = Some(desired_status); }
        if result.is_ok() { result = updated; }
    }
    match result {
        Ok(()) => state.last_error = None,
        Err(error) => {
            if state.last_error.as_ref() != Some(&error) { eprintln!("移动端提醒同步暂未完成：{error}"); }
            state.last_error = Some(error);
            mobile.dirty.store(true, Ordering::SeqCst);
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
        .title(title).body(body).icon("ic_stat_zuogong").show())
}
