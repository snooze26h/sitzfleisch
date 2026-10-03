use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};
use std::thread::{self, Thread};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tauri_plugin_sitzfleisch_android::SitzfleischAndroidExt;

use crate::{alerts::{status_model, StatusModel}, platform::{SettingsRequest, SystemStatus}, Shared};

/// 移动端缓存与唤醒句柄独立于 Shared，规则状态仍由原来的 Rust 状态锁管理。
pub struct MobileShared {
    dirty: AtomicBool,
    suspended: AtomicBool,
    heartbeat: Mutex<Option<Thread>>,
    status: Mutex<Option<StatusModel>>,
}

impl Default for MobileShared {
    fn default() -> Self {
        Self { dirty: AtomicBool::new(true), suspended: AtomicBool::new(false), heartbeat: Mutex::new(None), status: Mutex::new(None) }
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

/// 只由心跳调用；先释放 Shared 锁，再串行调用原生插件，失败不记为已应用。
pub fn sync_status(app: &AppHandle) {
    let mobile = app.state::<MobileShared>();
    mobile.dirty.swap(false, Ordering::SeqCst);
    let desired = {
        let shared = app.state::<Shared>();
        let state = shared.state.lock().unwrap();
        status_model(&state)
    };
    let mut applied = mobile.status.lock().unwrap();
    if applied.as_ref() == Some(&desired) { return; }
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)).unwrap_or(0);
    let result = plugin_call(|| app.sitzfleisch_android().update_status(desired.update(now_ms)));
    if result.is_ok() { *applied = Some(desired); }
    else { mobile.dirty.store(true, Ordering::SeqCst); }
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
