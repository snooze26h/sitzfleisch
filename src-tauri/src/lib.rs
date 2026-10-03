use std::fs;
#[cfg(desktop)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(desktop)]
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use chrono::{Local, TimeZone, Timelike};
use sitzfleisch_core as core;
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
#[cfg(desktop)]
use tauri::Wry;
#[cfg(any(desktop, target_os = "android"))]
use tauri::WindowEvent;
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
#[cfg(desktop)]
use tauri_plugin_notification::NotificationExt;
#[cfg(desktop)]
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[cfg_attr(mobile, allow(dead_code))]
mod browser_blocking;
#[cfg_attr(mobile, allow(dead_code))]
mod browser_auth;
#[cfg(any(target_os = "windows", test))]
mod windows_hosts;
#[cfg(target_os = "macos")]
mod space_preview;
mod platform;
mod alerts;
mod alarm_store;
mod clock_policy;
use clock_policy::{advance_time, resume_time, show_due_banners, TimePolicy};
#[cfg(target_os = "android")]
mod mobile;

#[derive(Clone, Default, Serialize)]
struct BlockingStatus {
    active: bool,
    busy: bool,
    error: Option<String>,
    browser: browser_blocking::Status,
}

#[cfg_attr(mobile, allow(dead_code))]
struct Shared {
    state: Mutex<core::State>,
    /// 退出只改变运行时屏蔽状态，不结束学习日或清空用户配置。
    exiting: AtomicBool,
    blocking_released: AtomicBool,
    /// 隔离测试不能写入或清理用户真实的系统 hosts。
    isolated: bool,
    snapshot_revision: AtomicU64,
    /// 从取快照到原子替换必须串行，避免旧存档后写或共用暂存文件。
    save_lock: Mutex<()>,
    /// 屏蔽只能一次做一件：连着改列表会起好几个线程，各自读 hosts、各自提权，
    /// 后写的会盖掉先写的，因此整个读取、授权与回读过程都必须串行。
    blocking_lock: Mutex<()>,
    /// QA 启动参数（--qa-view today|history|settings，--qa-scroll <px>）：只影响首屏，平时为空。
    qa_view: Option<String>,
    qa_scroll: i64,
    /// 状态文件损坏或来自更高版本时进入保护模式：原文件一个字节都不动，本次运行不落盘。
    write_protected: Mutex<Option<String>>,
    blocking: Mutex<BlockingStatus>,
    browser_bridge: Mutex<browser_blocking::Bridge>,
    /// 上一次保存失败的系统原因，成功一次即清空。**叶子锁**：持有它时不再取任何锁、
    /// 不调用任何原生 API。
    save_error: Mutex<Option<String>>,
    /// 用户在退出前保存失败的框里点过「不保存退出」。退出钩子里那次兜底保存必须让路——
    /// 否则我们会把他刚拒绝写的东西替他写回去。
    declined_final_save: Mutex<bool>,
    path: PathBuf,
    /// 状态文件、hosts 暂存文件与 QA 报告都落在这个目录里。
    /// 平时是系统的应用数据目录，`SITZFLEISCH_DATA_DIR` 可以把整套挪到别处做隔离测试。
    data_dir: PathBuf,
    /// 上一次装到托盘上的菜单形状，形状没变就不重建。
    tray_shape: Mutex<Option<TrayMenuState>>,
    /// 标题、提示和暂停标记分别去重；文字相同时也不能漏掉暂停状态的切换。
    tray_display: Mutex<Option<TrayDisplay>>,
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(mobile, allow(dead_code))]
enum TrayMenuState {
    Day { line: String, paused: Option<bool>, totals: String },
    Idle { profile: Option<(String, String)> },
}

/// 托盘菜单主行的文字。菜单和 shape 都从这里取，两边不会分叉。
#[cfg(desktop)]
fn tray_menu_line(state: &core::State, day: &core::Day) -> String {
    let name_of = |id: &str| {
        day.categories
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match &day.timer {
        Some(timer) => {
            let name = name_of(&timer.category);
            if day.is_paused() { format!("{name}（已暂停）") } else { name }
        }
        // 没有格在走 = 暂停。这里只提示下一格做什么。
        None => match core::suggest(day, &state.preferences, state.last_tick) {
            Some(s) => format!("下一格：{} · {} 分钟", name_of(&s.category), s.minutes),
            None => "今天的配额已经满了".to_string(),
        },
    }
}

#[cfg(desktop)]
fn default_profile(state: &core::State) -> Option<&core::ProfileDef> {
    state
        .preferences
        .profiles
        .iter()
        .find(|p| p.id == state.preferences.default_profile_id)
        .or_else(|| state.preferences.profiles.first())
}

/// 菜单的「形状」。**里面绝不能放每秒都变的数字**——菜单一旦被重建，正打开的那份就会被
/// 系统收走，表现出来就是「点开秒缩」。倒计时只放在菜单栏标题上，以等宽数字随文本变化更新。
/// 它必须便宜：每秒都要算一次，用来决定要不要真的去造那一整套原生菜单项。
#[cfg(desktop)]
fn tray_shape(state: &core::State) -> TrayMenuState {
    match &state.day {
        Some(day) => TrayMenuState::Day {
            line: tray_menu_line(state, day),
            paused: day.timer.as_ref().map(|_| day.is_paused()),
            totals: format!("已学 {}", core::duration_text(day.net_seconds())),
        },
        None => TrayMenuState::Idle {
            profile: default_profile(state).map(|p| (p.id.clone(), p.name.clone())),
        },
    }
}

/// 菜单栏菜单：跟着状态走。只在 `tray_shape` 变化时才需要重建。
#[cfg(desktop)]
fn build_tray_menu(app: &AppHandle, state: &TrayMenuState) -> tauri::Result<Menu<Wry>> {
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<Wry>>> = Vec::new();
    let text = |app: &AppHandle, id: &str, text: &str| MenuItem::with_id(app, id, text, false, None::<&str>);
    let action = |app: &AppHandle, id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);

    items.push(Box::new(MenuItem::with_id(app, "show", "打开坐功", true, Some("CmdOrCtrl+O"))?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    match state {
        TrayMenuState::Day { line, paused, totals } => {
            items.push(Box::new(text(app, "info", line)?));
            if let Some(paused) = paused {
                items.push(Box::new(action(app, "toggle", if *paused { "继续" } else { "暂停" })?));
                items.push(Box::new(action(app, "extend", "延长时间…")?));
                items.push(Box::new(action(app, "finish", "结束这一格")?));
            } else {
                items.push(Box::new(action(app, "show", "去开一格")?));
            }
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
            // 已学时间只到分钟，一天里最多变几百次，不会打断菜单操作。
            items.push(Box::new(text(app, "totals", totals)?));
        }
        TrayMenuState::Idle { profile } => {
            items.push(Box::new(text(app, "info", "今天还没开始")?));
            if let Some((id, name)) = profile {
                // 计划只剩一份，按钮和开始页一样叫「开始今天」；拼计划名会拼出「开始今天日」。
                let _ = name;
                items.push(Box::new(action(app, &format!("start:{id}"), "开始今天")?));
            }
        }
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?));
    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i.as_ref()).collect();
    Menu::with_items(app, &refs)
}

#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    // 还原之前先问，问完再动手：下面那一轮补激活只在原本就最小化时才需要。
    let was_minimized = window.is_minimized().unwrap_or(false);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    if !was_minimized {
        return;
    }
    // tao 的 `Window::set_focus()` 在 `isMiniaturized()` 为真时**整段跳过**，
    // 连里面那句 `activateIgnoringOtherApps:` 都不发；而 `deminiaturize:` 的还原动画
    // 不是同步结束的，紧接着的那次 set_focus 多半正好撞在这个窗口期里。
    // 结果就是窗口在自己的 Space 里恢复了、用户却留在原地，只看到 app 不退出、找不到框。
    // 所以等它真的不再最小化，再激活一次。上限 1.5 秒，等不到就算了，别把线程留着。
    let window = window.clone();
    thread::spawn(move || {
        for _ in 0..30 {
            thread::sleep(Duration::from_millis(50));
            if !window.is_minimized().unwrap_or(false) {
                let _ = window.set_focus();
                return;
            }
        }
    });
}

#[cfg(desktop)]
fn handle_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main_window(app),
        "toggle" => {
            let _ = mutate(app, |s| s.toggle_pause(now_unix()));
        }
        "extend" => {
            show_main_window(app);
            let _ = app.emit("timer://extend", ());
        }
        "finish" => {
            let _ = mutate(app, |s| s.finish_block(now_unix()));
        }
        "quit" => quit_saving(app),
        other => {
            if let Some(profile) = other.strip_prefix("start:") {
                let profile = profile.to_string();
                let started = mutate(app, |s| s.start_day(&profile, now_unix())).is_ok();
                if started && blocking_wanted(&app.state::<Shared>()) {
                    spawn_apply_blocking(app);
                }
                show_main_window(app);
            }
        }
    }
}

/// 每秒都要推的那部分：偏好 + 今天。**不含历史**。
#[derive(Clone, Serialize)]
struct LiveState {
    schema: u32,
    last_tick: i64,
    preferences: core::Preferences,
    day: Option<core::Day>,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    revision: u64,
    state: LiveState,
    /// 历史一满 60 天就有 270 KB 上下，每秒推一遍是白烧电。
    /// 所以只有首次拉取和命令的返回值带上它，心跳推送里是 null，界面沿用上一次的。
    history: Option<Vec<core::ArchivedDay>>,
    write_protected: Option<String>,
    blocking: BlockingStatus,
    /// 上一次保存失败的系统原因：只走 IPC，不进 state.json。
    save_error: Option<String>,
    state_path: String,
    initial_view: Option<String>,
    initial_scroll: i64,
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn time_policy() -> TimePolicy {
    if cfg!(mobile) { TimePolicy::WallClock } else { TimePolicy::Heartbeat }
}

fn snapshot(shared: &Shared, with_history: bool) -> Snapshot {
    let state = shared.state.lock().unwrap();
    let mut blocking = shared.blocking.lock().unwrap().clone();
    blocking.browser = shared.browser_bridge.lock().unwrap()
        .status(&browser_blocking::Rules::from_state(&state, shared.exiting.load(Ordering::SeqCst)));
    Snapshot {
        // 与状态捕获一起排序；同一秒里的命令也有先后，不依赖时间戳。
        revision: shared.snapshot_revision.fetch_add(1, Ordering::Relaxed) + 1,
        state: LiveState {
            schema: state.schema,
            last_tick: state.last_tick,
            preferences: state.preferences.clone(),
            day: state.day.clone(),
        },
        history: with_history.then(|| state.history.clone()),
        write_protected: shared.write_protected.lock().unwrap().clone(),
        blocking,
        // 取锁顺序：state → write_protected → blocking → save_error，save_error 是最后一把。
        save_error: shared.save_error.lock().unwrap().clone(),
        state_path: shared.path.display().to_string(),
        initial_view: shared.qa_view.clone(),
        initial_scroll: shared.qa_scroll,
    }
}

/// 解析 `--qa-view <today|history|settings>` 与 `--qa-scroll <px>`；没有就都是空。
fn qa_launch_args() -> (Option<String>, i64) {
    parse_qa_args(&std::env::args().collect::<Vec<String>>())
}

fn parse_qa_args(args: &[String]) -> (Option<String>, i64) {
    let mut view = None;
    let mut scroll = 0;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--qa-view" if i + 1 < args.len() => {
                let v = args[i + 1].as_str();
                if matches!(v, "today" | "history" | "settings") {
                    view = Some(v.to_string());
                }
                i += 1;
            }
            "--qa-scroll" if i + 1 < args.len() => {
                scroll = args[i + 1].parse().unwrap_or(0);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    (view, scroll)
}

fn save(shared: &Shared) {
    let _serial = shared.save_lock.lock().unwrap();
    if shared.write_protected.lock().unwrap().is_some() {
        return;
    }
    let json = core::to_json(&shared.state.lock().unwrap());
    let tmp = shared.path.with_extension("json.tmp");
    // 先把临时文件真正写到盘上再改名：断电或系统崩溃时，宁可留下上一份完整的存档，
    // 也不要一个改了名、内容却没落盘的空文件——那样下次启动只能进保护模式。
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &shared.path)
    })();
    // 结果发布在 save_lock 之内：两笔排队的保存，后写的那份结果才是最终结论。
    *shared.save_error.lock().unwrap() = result.err().map(|e| e.to_string());
}

/// 短名放得下多宽：4 个汉字，或 8 个拉丁字符。
#[cfg(desktop)]
const SHORT_NAME_WIDTH: usize = 8;

/// 一个字顶两个拉丁字符宽的那类字：汉字、假名、谚文、全角标点。
#[cfg(desktop)]
fn is_wide(c: char) -> bool {
    matches!(u32::from(c),
        0x1100..=0x115F      // 谚文字母
        | 0x2E80..=0x303E    // CJK 部首、符号与标点
        | 0x3041..=0x33FF    // 假名、注音、兼容字符
        | 0x3400..=0x4DBF    // 扩展 A
        | 0x4E00..=0x9FFF    // 基本汉字
        | 0xA000..=0xA4CF    // 彝文
        | 0xAC00..=0xD7A3    // 谚文音节
        | 0xF900..=0xFAFF    // 兼容汉字
        | 0xFE30..=0xFE6F    // 竖排与小写变体
        | 0xFF00..=0xFF60    // 全角
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FA1F  // 扩展 B–F
    )
}

#[cfg(desktop)]
fn display_width(text: &str) -> usize {
    text.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

/// `short_name` 为空时的自动回退。**与 `src/format.ts::shortNameFrom` 必须一模一样**。
/// 取的是**前**两个字不是后两个：「深度工作」截成「工作」会丢掉是哪一种。
#[cfg(desktop)]
fn short_name_from(name: &str) -> String {
    let trimmed = name.trim();
    if display_width(trimmed) <= SHORT_NAME_WIDTH {
        return trimmed.to_string();
    }
    if let Some(first) = trimmed.split_whitespace().next() {
        if display_width(first) <= SHORT_NAME_WIDTH {
            return first.to_string();
        }
    }
    trimmed.chars().take(2).collect()
}

/// 显示名：学习日进行中一律读当天**冻结**的名字，改名从下一个学习日起才生效。
/// 前端的 `nameOf()` 就是这条规则，托盘必须同源，否则两处会喊出不同的名字。
#[cfg(desktop)]
fn display_name(state: &core::State, id: &str) -> String {
    if let Some(day) = &state.day {
        if let Some(c) = day.categories.iter().find(|c| c.id == id) {
            return c.name.clone();
        }
    }
    match state.preferences.categories.iter().find(|c| c.id == id) {
        Some(def) => def.name.clone(),
        None => id.to_string(),
    }
}

#[cfg(desktop)]
fn short_name(state: &core::State, id: &str) -> String {
    // 短名是显示偏好，改完立刻生效，不跟着学习日冻结——和图标一个待遇。
    if let Some(def) = state.preferences.categories.iter().find(|c| c.id == id) {
        if !def.short_name.trim().is_empty() {
            return def.short_name.trim().to_string();
        }
    }
    short_name_from(&display_name(state, id))
}

#[cfg(desktop)]
fn clock_text(seconds: i64) -> String {
    let safe = seconds.max(0);
    if safe >= 3600 {
        format!("{}:{:02}:{:02}", safe / 3600, (safe % 3600) / 60, safe % 60)
    } else {
        format!("{:02}:{:02}", safe / 60, safe % 60)
    }
}

/// 菜单栏的完整状态文字，供悬浮提示使用；紧凑标题由 `tray_display` 生成。
#[cfg(desktop)]
fn tray_text(state: &core::State) -> String {
    let Some(day) = &state.day else { return "坐功".into() };
    let now = state.last_tick;
    // 有格在走：跑就报剩余，停就报停了多久。
    if let Some(timer) = &day.timer {
        if day.is_paused() {
            // 秒表**故意**留着：看得见的暂停才会被结束，把数字藏起来，五分钟的休息就是这么
            // 变成一下午的。PRODUCT.md 原来写的是「never a pause stopwatch」，2026-09-06 按
            // 这个理由改了契约，不是代码在违约。
            // 名字也要留着：光一个「暂停 01:16」看不出是哪一格停了，侧栏那处一直写着名字。
            return format!("{} 暂停 {}", short_name(state, &timer.category), clock_text(day.current_pause_seconds(now)));
        }
        return format!("{} {}", short_name(state, &timer.category), clock_text(timer.remaining_seconds()));
    }
    if day.resting(now) {
        return format!("休息 {}", clock_text(day.break_remaining(now)));
    }
    // 没有格在走：报「该做什么」比报「摸了多久」有用。
    // 标题和托盘菜单必须用同一个调度器，否则两处会给出不同的「下一格」。
    match core::suggest(day, &state.preferences, now) {
        Some(s) => format!("下一格 {}", short_name(state, &s.category)),
        None => "今日达成".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(mobile, allow(dead_code))]
struct TrayDisplay {
    title: String,
    tooltip: String,
    paused: bool,
}

/// 菜单栏只占一小段：跨小时后省略秒数，完整时长仍在悬浮提示中。
#[cfg(desktop)]
fn compact_tray_clock(seconds: i64) -> String {
    let safe = seconds.max(0);
    match safe {
        0..3_600 => clock_text(safe),
        3_600..36_000 => format!("{}h{:02}", safe / 3_600, (safe % 3_600) / 60),
        36_000..360_000 => format!("{}h", safe / 3_600),
        _ => "99h+".into(),
    }
}

#[cfg(desktop)]
fn tray_display(state: &core::State) -> TrayDisplay {
    let text = tray_text(state);
    let mut display = TrayDisplay { title: text.clone(), tooltip: format!("坐功 · {text}"), paused: false };
    let Some(day) = &state.day else { return display };
    if let Some(timer) = &day.timer {
        display.paused = day.is_paused();
        let seconds = if display.paused { day.current_pause_seconds(state.last_tick) } else { timer.remaining_seconds() };
        // 暂停放进原图标的角标，不向标题插入额外文字，避免暂停时整项变宽被系统挤掉。
        display.title = format!("{} {}", short_name(state, &timer.category), compact_tray_clock(seconds));
    } else if day.resting(state.last_tick) {
        display.title = format!("休息 {}", compact_tray_clock(day.break_remaining(state.last_tick)));
    }
    display
}

/// 在原图标内部画暂停角标，保持画布尺寸与宽高比，菜单栏占位不会随暂停增加。
#[cfg(desktop)]
fn tray_icon(icon: &tauri::image::Image<'_>, paused: bool) -> tauri::image::Image<'static> {
    if !paused {
        return icon.clone().to_owned();
    }
    let (width, height) = (icon.width() as usize, icon.height() as usize);
    let side = width.min(height);
    let mut rgba = icon.rgba().to_vec();
    if side >= 16 && width.checked_mul(height).and_then(|n| n.checked_mul(4)) == Some(rgba.len()) {
        let badge = side / 2;
        let edge = (side / 32).max(1);
        let left = width - badge;
        let top = height - badge;
        for y in 0..badge {
            for x in 0..badge {
                let border = x < edge || y < edge || x >= badge - edge || y >= badge - edge;
                let bar = y >= badge / 4 && y < badge * 3 / 4
                    && ((x >= badge / 4 && x < badge * 7 / 16) || (x >= badge * 9 / 16 && x < badge * 3 / 4));
                let color = if border { [25, 24, 21, 255] } else if bar { [235, 229, 215, 255] } else { [168, 73, 48, 255] };
                let index = ((top + y) * width + left + x) * 4;
                rgba[index..index + 4].copy_from_slice(&color);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, icon.width(), icon.height())
}

#[cfg(target_os = "macos")]
fn configure_tray_digits(tray: &tauri::tray::TrayIcon) -> tauri::Result<()> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSFont, NSFontWeightRegular};

    tray.with_inner_tray_icon(|inner| {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(item) = inner.ns_status_item() else { return };
        let Some(button) = item.button(mtm) else { return };
        let size = button.font().map(|font| font.pointSize()).unwrap_or_else(NSFont::systemFontSize);
        // 数字等宽，保留系统字号与中文显示；避免秒数变化反复改变菜单栏占位。
        // 此常量由已链接的 AppKit 提供，在整个进程生命周期内有效。
        let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, unsafe { NSFontWeightRegular });
        button.setFont(Some(&font));
    })
}

fn broadcast(app: &AppHandle) {
    #[cfg(target_os = "android")]
    if mobile::is_suspended(app) { return; }
    let shared = app.state::<Shared>();
    // 心跳推送不带历史：历史只在命令返回和首次拉取时随快照走一遍。
    let snap = snapshot(&shared, false);
    let _ = app.emit("state://update", &snap);
    // 原生菜单会同步等待主线程；带着应用锁调用会与主线程的命令互相等。
    // 到主线程后再取当前形状，排队的更新也不会把旧菜单装回去。
    #[cfg(desktop)]
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let shared = handle.state::<Shared>();
            let (display, shape) = {
                let state = shared.state.lock().unwrap();
                (tray_display(&state), tray_shape(&state))
            };
            let Some(tray) = handle.tray_by_id("main") else { return };
            let previous = shared.tray_display.lock().unwrap().clone();
            if previous.as_ref() != Some(&display) {
                // 原生调用之前释放缓存锁；失败时不记为已应用，下次推送继续重试。
                #[cfg(target_os = "macos")]
                let title_applied = previous.as_ref().is_some_and(|p| p.title == display.title)
                    || tray.set_title(Some(&display.title)).is_ok();
                #[cfg(not(target_os = "macos"))]
                let title_applied = true;
                let tooltip_applied = previous.as_ref().is_some_and(|p| p.tooltip == display.tooltip)
                    || tray.set_tooltip(Some(&display.tooltip)).is_ok();
                let icon_applied = previous.as_ref().is_some_and(|p| p.paused == display.paused)
                    || handle.default_window_icon().is_some_and(|icon| tray.set_icon(Some(tray_icon(icon, display.paused))).is_ok());
                if title_applied && tooltip_applied && icon_applied {
                    *shared.tray_display.lock().unwrap() = Some(display);
                }
            }
            // shape 便宜，先算它：形状没变就不去造那一整套原生菜单项（这是每秒都会走的路径）。
            let changed = shared.tray_shape.lock().unwrap().as_ref() != Some(&shape);
            if changed {
                if let Ok(menu) = build_tray_menu(&handle, &shape) {
                    if tray.set_menu(Some(menu)).is_ok() {
                        *shared.tray_shape.lock().unwrap() = Some(shape);
                    }
                }
            }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlertSound {
    Standard,
    Water,
}

impl AlertSound {
    #[cfg(target_os = "macos")]
    fn file(self) -> &'static str {
        match self {
            Self::Standard => "/System/Library/Sounds/Glass.aiff",
            Self::Water => "/System/Library/Sounds/Pop.aiff",
        }
    }

    #[cfg(target_os = "windows")]
    fn command(self) -> &'static str {
        match self {
            Self::Standard => "[console]::beep(880,180); [console]::beep(1320,180)",
            Self::Water => "[console]::beep(587,120); [console]::beep(784,120)",
        }
    }
}

fn local_reminder_seconds(state: &core::State) -> u32 {
    if cfg!(mobile) && !state.preferences.water_reminder_enabled { return 3600; }
    let local = Local.timestamp_opt(state.last_tick, 0).single();
    local.map(|time| time.minute() * 60 + time.second()).unwrap_or(3600)
}

#[cfg(any(not(target_os = "android"), test))]
fn take_due_reminder_notifications(state: &mut core::State) -> Vec<(&'static str, String, AlertSound)> {
    let local_seconds = local_reminder_seconds(state);
    take_due_reminder_notifications_at(state, local_seconds)
}

fn take_due_reminder_notifications_at(state: &mut core::State, local_seconds: u32) -> Vec<(&'static str, String, AlertSound)> {
    let water_due = state.take_due_water_reminder(local_seconds);
    let (_, stretch_due, idle_due) = state.take_due_reminders();
    let mut notifications = Vec::new();
    for (due, kind) in [(water_due, core::AlertKind::Water), (stretch_due, core::AlertKind::Stretch), (idle_due, core::AlertKind::Idle)] {
        if !due { continue; }
        let pause_started_at = state.day.as_ref().filter(|day| day.is_paused())
            .and_then(|day| day.pauses.last()).map(|pause| pause.started_at);
        let alert = core::PlannedAlert { kind, at: state.last_tick, pause_started_at };
        if cfg!(mobile) && !alerts::mobile_alert_allowed(state, &alert) { continue; }
        let (title, body, _) = alerts::alert_copy(kind, state, &alert);
        let sound = if kind == core::AlertKind::Water { AlertSound::Water } else { AlertSound::Standard };
        notifications.push((title, body, sound));
    }
    notifications
}

/// 系统横幅显不显示由每个 App 的通知设置说了算，App 自己既读不到也改不了。
/// 所以提醒一次走四条路，任何一条都能让人察觉：
/// 系统通知 · 界面提示条 · 一声响 · Dock 图标跳一下。
fn notify(app: &AppHandle, title: &str, body: &str, sound: AlertSound) {
    #[cfg(target_os = "android")]
    if mobile::is_suspended(app) { return; }
    #[cfg(desktop)]
    let delivered = app.notification().builder().title(title).body(body).show().is_ok();
    // Android 系统通知由后续的移动端排程负责；A1 只保留应用内提示，避免同步插件阻塞主线程。
    #[cfg(mobile)]
    let delivered = false;
    let _ = app.emit("reminder://show", serde_json::json!({
        "title": title,
        "body": body,
        "system": delivered,
    }));
    #[cfg(desktop)]
    {
        let sound_on = app.state::<Shared>().state.lock().unwrap().preferences.sound_enabled;
        if sound_on { play_alert_sound(sound); }
    }
    #[cfg(mobile)]
    let _ = sound;
    // 人多半在别的 App 里，Dock 上跳一下比什么都直接。
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// 响一声。走系统自带的声音，不需要任何权限，窗口关着也听得见。
#[cfg(desktop)]
fn play_alert_sound(sound: AlertSound) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/afplay")
        .arg(sound.file())
        .spawn();
    #[cfg(target_os = "windows")]
    if let Ok(mut command) = windows_hosts::powershell_process() {
        let _ = command.args(["-NoProfile", "-NonInteractive", "-Command", sound.command()]).spawn();
    }
}

#[cfg(all(mobile, not(target_os = "android")))]
fn play_alert_sound(_sound: AlertSound) {}

/// 推进一次时间，并报告这一下是不是把格自然走完了（台账多了一条，格也不在了）。
/// 心跳和命令都会推进时间：格恰好在某个命令那一下走完时，通知也得发，不能只看心跳。
fn tick_reporting_finish(state: &mut core::State, now: i64, policy: TimePolicy) -> bool {
    let had_timer = state.day.as_ref().is_some_and(|d| d.timer.is_some());
    let ledger_before = state.day.as_ref().map_or(0, |d| d.ledger.len());
    advance_time(state, now, policy);
    had_timer && state.day.as_ref().is_some_and(|d| d.ledger.len() > ledger_before && d.timer.is_none())
}

fn notify_block_finished(app: &AppHandle) {
    let (title, body, _) = {
        let shared = app.state::<Shared>();
        let state = shared.state.lock().unwrap();
        let alert = core::PlannedAlert { kind: core::AlertKind::BlockFinished, at: state.last_tick, pause_started_at: None };
        alerts::alert_copy(alert.kind, &state, &alert)
    };
    notify(app, title, &body, AlertSound::Standard);
}

struct HeartbeatStep {
    focus_done: bool,
    break_done: bool,
    reminders: Vec<(&'static str, String, AlertSound)>,
    show_banners: bool,
    #[cfg(target_os = "android")]
    desired: Vec<alerts::AppliedAlarm>,
    #[cfg(target_os = "android")]
    status: alerts::StatusModel,
    #[cfg(target_os = "android")]
    now: i64,
}

fn advance_heartbeat(state: &mut core::State, now: i64) -> HeartbeatStep {
    let show_banners = show_due_banners(time_policy(), now.saturating_sub(state.last_tick));
    let focus_done = tick_reporting_finish(state, now, time_policy());
    #[cfg(target_os = "android")]
    let local_seconds = local_reminder_seconds(state);
    #[cfg(target_os = "android")]
    let reminders = take_due_reminder_notifications_at(state, local_seconds);
    #[cfg(not(target_os = "android"))]
    let reminders = take_due_reminder_notifications(state);
    let break_done = state.take_due_break(now);
    HeartbeatStep {
        focus_done, break_done, reminders, show_banners,
        #[cfg(target_os = "android")]
        desired: alerts::desired_alarms(state, local_seconds),
        #[cfg(target_os = "android")]
        status: alerts::status_model(state),
        #[cfg(target_os = "android")]
        now: state.last_tick,
    }
}

fn mutate(
    app: &AppHandle,
    op: impl FnOnce(&mut core::State) -> core::RuleResult,
) -> Result<Snapshot, String> {
    let shared = app.state::<Shared>();
    if shared.exiting.load(Ordering::SeqCst) {
        return Err("正在退出坐功，请等待网站屏蔽解除。".into());
    }
    let (outcome, finished, show_banners) = {
        let mut state = shared.state.lock().unwrap();
        let now = now_unix();
        let show_banners = show_due_banners(time_policy(), now.saturating_sub(state.last_tick));
        let finished = tick_reporting_finish(&mut state, now, time_policy());
        (op(&mut state), finished, show_banners)
    };
    if finished && show_banners {
        notify_block_finished(app);
    }
    outcome.map_err(|e| e.to_string())?;
    save(&shared);
    #[cfg(target_os = "android")]
    mobile::mark_dirty(app);
    broadcast(app);
    Ok(snapshot(&shared, true))
}

// ---------- 网站屏蔽（hosts；变换在 core，落盘与提权在这里） ----------

/// 写入与退出清理共用同一条验证路径；只修改坐功托管段。
#[cfg(desktop)]
fn sync_hosts_file(
    path: &Path,
    hosts: &[String],
    enable: bool,
    install: impl FnOnce(&str) -> Result<(), String>,
) -> Result<bool, String> {
    let current = fs::read_to_string(path).map_err(|e| format!("读不到系统 hosts：{e}"))?;
    // 丢失结束标记时直接剥离会吞掉后面的用户条目；先确认每段边界完整。
    let mut inside = false;
    for line in current.lines().map(str::trim) {
        if line == core::HOSTS_BEGIN {
            if inside { return Err("系统 hosts 的坐功托管标记异常，原文件已保留。".into()); }
            inside = true;
        } else if line == core::HOSTS_END {
            if !inside { return Err("系统 hosts 的坐功托管标记异常，原文件已保留。".into()); }
            inside = false;
        }
    }
    if inside { return Err("系统 hosts 的坐功托管标记不完整，原文件已保留。".into()); }
    let desired = core::render_hosts(&current, hosts, enable);
    if desired == current {
        return Ok(core::hosts_section_present(&current));
    }
    if desired.is_empty() || desired.len() > 65536 {
        return Err("hosts 内容为空或超过 65536 字节，原文件已保留。".into());
    }
    // 授权只接收这次计算出的内存内容，不能重新读取一个可被替换的用户文件。
    install(&desired)?;
    let after = fs::read_to_string(path).map_err(|e| format!("回读系统 hosts 失败：{e}"))?;
    if after != desired {
        return Err("回读核验不一致，规则可能没有生效。".into());
    }
    Ok(core::hosts_section_present(&after))
}

#[cfg(desktop)]
fn record_blocking_result(shared: &Shared, result: &Result<bool, String>) {
    let mut blocking = shared.blocking.lock().unwrap();
    blocking.busy = false;
    match result {
        Ok(present) => { blocking.active = *present; blocking.error = None; }
        Err(message) => {
            blocking.error = Some(message.clone());
            blocking.active = read_system_hosts()
                .map(|c| core::hosts_section_present(&c)).unwrap_or(blocking.active);
        }
    }
}

#[cfg(desktop)]
fn release_system_blocking(shared: &Shared) -> Result<(), String> {
    if shared.isolated { return Ok(()); }
    // 必须等已在写 hosts 的线程完成；后续排队写入也会看到 exiting，只能清理。
    let _serial = shared.blocking_lock.lock().unwrap_or_else(|e| e.into_inner());
    shared.blocking.lock().unwrap().busy = true;
    let result = hosts_path().and_then(|path| sync_hosts_file(&path, &[], false, |content| privileged_install(content, &shared.data_dir)));
    record_blocking_result(shared, &result);
    result.map(|_| ())
}

#[cfg(desktop)]
fn hosts_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        return Ok(windows_hosts::system_directory()?.join("drivers/etc/hosts"));
    }
    #[cfg(not(target_os = "windows"))]
    Ok(PathBuf::from("/etc/hosts"))
}

#[cfg(desktop)]
fn read_system_hosts() -> Result<String, String> {
    fs::read_to_string(hosts_path()?).map_err(|e| format!("读不到系统 hosts：{e}"))
}

/// macOS 授权脚本使用的标准 Base64（带 = 补位）。
#[cfg(target_os = "macos")]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// AppleScript 字符串字面量的转义。**反斜杠必须先换**：反过来的话 `"` → `\"` 里
/// 新加的那个反斜杠会被第二遍再转一次，变成 `\\"`，字符串当场断掉。
#[cfg(target_os = "macos")]
fn applescript_string(raw: &str) -> String {
    raw.replace('\\', r"\\").replace('"', "\\\"")
}

/// hosts 内容 → 完整的 AppleScript。抽出来是为了能测转义而不真的去提权。
///
/// 内容以 Base64 嵌进脚本，由 root 解码到 /etc 里自己新建的临时文件，再原地覆盖 /etc/hosts。
/// 以前是让 root 去 cp 应用数据目录里的暂存文件：授权框可以停留任意久，同一账号下的程序能趁机
/// 把暂存文件换成指向 root 专属文件的符号链接，cp 照着读出来，就写进了所有人可读的 /etc/hosts。
/// Base64 只有字母、数字和 + / =，放进单引号不需要任何转义，也就没有注入的余地。
#[cfg(target_os = "macos")]
fn install_script(content: &str) -> String {
    // `killall mDNSResponder` 而不是 `-HUP`：SIGHUP 是苹果文档里的路子，但它**不保证**丢掉
    // 已经缓存下来的 hosts 派生记录。实测（macOS 26.6）解除屏蔽后 hosts 明明清干净了，
    // 被解除的域名仍然解析到 127.0.0.1 至少两分钟，手动再跑一次 flushcache 也没用，
    // 约 25 分钟后才自己恢复——用户看到的就是「收工了但站还打不开」。
    // 整个进程收掉、由 launchd 重新拉起，新实例带着空缓存重读 hosts，这一下是确定的。
    // 不用 `launchctl kickstart`：它要 launchd 的标签，而那个标签跟版本走
    // （macOS 26 上是 com.apple.mDNSResponder.reloaded，不是通用的 com.apple.mDNSResponder），
    // 按**进程名**杀更耐得住系统升级。
    // 刷新是尽力而为，不能让它把「写成功了」报成失败：`do shell script` 拿最后一条命令的
    // 退出码当整条脚本的结果，而 `killall` 在进程名对不上时返回非零。所以写入失败才 exit 1，
    // 后面两条各自吞掉错误，最后显式 exit 0。
    // cp 打开已有的 /etc/hosts 截断重写，属主和权限保持原样。
    let shell = [
        "umask 022".to_string(),
        "t=$(/usr/bin/mktemp /etc/.sitzfleisch-hosts.XXXXXX) || exit 1".into(),
        format!("/usr/bin/printf '%s' '{}' | /usr/bin/base64 -D > \"$t\" || {{ /bin/rm -f \"$t\"; exit 1; }}", base64_encode(content.as_bytes())),
        "/bin/cp -f \"$t\" /etc/hosts; s=$?; /bin/rm -f \"$t\"; [ $s -eq 0 ] || exit 1".into(),
        "/usr/bin/dscacheutil -flushcache >/dev/null 2>&1".into(),
        "/usr/bin/killall mDNSResponder >/dev/null 2>&1".into(),
        "exit 0".into(),
    ]
    .join("; ");
    format!(
        r#"do shell script "{}" with administrator privileges with prompt "坐功需要管理员权限来更新学习日的网站屏蔽规则。""#,
        applescript_string(&shell)
    )
}

/// 免密助手的固定位置。装上它，开工与收工就不再弹授权框；没装则一切照旧。
/// 内容走标准输入而不是参数，助手自己拒收未知参数，安装脚本再把 sudoers
/// 规则收紧到「不带参数」——三道都指向同一件事：这条免密路径只能干这一件事。
#[cfg(target_os = "macos")]
const HOSTS_HELPER: &str = "/usr/local/libexec/sitzfleisch-hosts-install";

/// 这一版的助手脚本原文。装在系统里的那份必须与它逐字相同才用：
/// 安装脚本是原样拷过去的，对不上就说明是旧版（早期版本没有锁定 PATH，
/// 借着 sudo 免密就能以 root 跑调用者 PATH 里的同名程序）。
#[cfg(target_os = "macos")]
const HOSTS_HELPER_SOURCE: &str = include_str!("../../scripts/hosts-helper.sh");

/// 先试免密助手。`sudo -n` 在没有免密规则时**直接失败而不弹任何窗**，
/// 所以这条路要么静默成功，要么无声让开，不会在授权框之前多出一次打扰。
/// 任何失败都返回 None：调用方回到 osascript 授权，行为与没装助手时完全一致。
#[cfg(target_os = "macos")]
fn helper_install(content: &str) -> Option<()> {
    // 对不上就当没装：退回系统授权框，不替一个过时的免密入口背书。重新运行安装脚本即可换成新版。
    if fs::read_to_string(HOSTS_HELPER).ok()? != HOSTS_HELPER_SOURCE {
        return None;
    }
    let mut child = Command::new("/usr/bin/sudo")
        .env_clear().env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin").current_dir("/")
        .args(["-n", HOSTS_HELPER])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    use std::io::Write;
    let written = child.stdin.take()?.write_all(content.as_bytes()).is_ok();
    let status = child.wait().ok()?;
    (written && status.success()).then_some(())
}

#[cfg(target_os = "macos")]
fn privileged_install(content: &str, _data_dir: &Path) -> Result<(), String> {
    if helper_install(content).is_some() {
        return Ok(());
    }
    let script = install_script(content);
    let output = Command::new("/usr/bin/osascript")
        .env_clear().env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin").current_dir("/")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("无法调用系统授权：{e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    if err.contains("-128") || err.to_lowercase().contains("cancel") {
        Err("授权被取消，屏蔽规则未写入。".into())
    } else {
        Err("系统授权或 hosts 写入失败，请重试。".into())
    }
}

#[cfg(target_os = "windows")]
fn privileged_install(content: &str, data_dir: &Path) -> Result<(), String> {
    windows_hosts::install(content, data_dir)
}

#[cfg(all(desktop, not(any(target_os = "macos", target_os = "windows"))))]
fn privileged_install(_content: &str, _data_dir: &Path) -> Result<(), String> {
    Err("此平台暂不支持网站屏蔽。".into())
}

/// 在独立线程里应用/解除屏蔽；结果通过 blocking 状态广播回界面。
#[cfg(desktop)]
fn spawn_apply_blocking(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let shared = app.state::<Shared>();
        if shared.isolated { return; }
        // 暂存文件跟着状态文件走同一个目录：隔离测试时不能落回真实目录。
        let data_dir = shared.data_dir.clone();
        apply_latest_blocking(&shared, |hosts, enable| {
            let result = hosts_path().and_then(|path| sync_hosts_file(&path, hosts, enable, |content| privileged_install(content, &data_dir)));
            record_blocking_result(&shared, &result);
        });
        // 退出钩子可能在主线程等 blocking_lock；原生界面调用放到解锁之后。
        broadcast(&app);
    });
}

#[cfg(desktop)]
fn apply_latest_blocking(shared: &Shared, apply: impl FnOnce(&[String], bool)) {
    let _serial = shared.blocking_lock.lock().unwrap_or_else(|e| e.into_inner());
    // 先标忙再取状态，取完后即使最后一个域名被删，也会安排后续清理。
    {
        let mut blocking = shared.blocking.lock().unwrap();
        blocking.busy = true;
        blocking.error = None;
    }
    // 等待授权的间隙可能已经收工或重新开日；开关和域名必须一起从当前状态取。
    let (hosts, enable) = {
        let state = shared.state.lock().unwrap();
        (state.preferences.blocked_hosts.clone(), state.day.is_some() && !shared.exiting.load(Ordering::SeqCst))
    };
    apply(&hosts, enable);
}

#[cfg(desktop)]
fn blocking_wanted(shared: &Shared) -> bool {
    let state = shared.state.lock().unwrap();
    state.day.is_some() && !state.preferences.blocked_hosts.is_empty() && !shared.exiting.load(Ordering::SeqCst)
}

#[cfg(desktop)]
fn blocking_needs_sync(shared: &Shared) -> bool {
    let active_or_busy = {
        let blocking = shared.blocking.lock().unwrap();
        blocking.active || blocking.busy
    };
    active_or_busy || blocking_wanted(shared)
}

// 手机不接管系统 hosts，也不启动桌面扩展桥；保留共用调用点与快照结构。
#[cfg(mobile)]
fn spawn_apply_blocking(_app: &AppHandle) {}

#[cfg(mobile)]
fn release_system_blocking(_shared: &Shared) -> Result<(), String> { Ok(()) }

#[cfg(mobile)]
fn blocking_wanted(_shared: &Shared) -> bool { false }

#[cfg(mobile)]
fn blocking_needs_sync(_shared: &Shared) -> bool { false }

#[cfg(desktop)]
fn refresh_hosts_status(blocking: &mut BlockingStatus, current: &str, hosts: &[String], enable: bool) {
    // active 仍记录系统残留；是否与设置一致要逐条核对，不能用一个 BEGIN 标记代替。
    blocking.active = core::hosts_section_present(current);
    blocking.error = if core::hosts_rules_match(current, hosts, enable) {
        None
    } else if !enable || hosts.is_empty() {
        Some("系统里仍残留整站屏蔽规则，请到「设置 · 网站屏蔽」点击「重新应用整站规则」解除。".into())
    } else {
        Some("系统整站规则与当前设置不一致，可能缺少规则或仍有旧规则。请点击「重新应用整站规则」修复。".into())
    };
}

// ---------- 命令 ----------

#[tauri::command]
fn get_snapshot(shared: State<'_, Shared>) -> Snapshot {
    snapshot(&shared, true)
}

// 凭据只通过桌面 IPC 在明确点击时返回，不能混进每秒广播的普通状态快照。
#[tauri::command]
fn browser_pairing_code(shared: State<'_, Shared>) -> Result<String, String> {
    browser_blocking::pairing_code(&shared, false)
}

#[tauri::command]
fn reset_browser_pairing(app: AppHandle) -> Result<String, String> {
    let code = browser_blocking::pairing_code(&app.state::<Shared>(), true)?;
    broadcast(&app);
    Ok(code)
}

#[tauri::command]
fn start_day(profile_id: String, app: AppHandle) -> Result<Snapshot, String> {
    let snap = mutate(&app, |s| s.start_day(&profile_id, now_unix()))?;
    if blocking_wanted(&app.state::<Shared>()) {
        spawn_apply_blocking(&app);
    }
    Ok(snap)
}

#[tauri::command]
fn start_block(
    category_id: String,
    minutes: i64,
    tasks: Vec<core::TaskItem>,
    break_minutes: Option<i64>,
    app: AppHandle,
) -> Result<Snapshot, String> {
    mutate(&app, |s| match break_minutes {
        Some(rest) => s.start_block_with_break(&category_id, minutes, tasks.clone(), rest),
        None => s.start_block(&category_id, minutes, tasks.clone()),
    })
}

/// 换一份计划：进度与台账保留，只按新计划重铺配额。界面上没有入口，留给数据里存着多份的老档。
#[tauri::command]
fn switch_profile(profile_id: String, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.switch_profile(&profile_id))
}

#[tauri::command]
fn toggle_pause(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.toggle_pause(now_unix()))
}

#[tauri::command]
fn extend_block(minutes: i64, timer_started_at: Option<i64>, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| {
        if timer_started_at.is_some_and(|expected| s.day.as_ref().and_then(|d| d.timer.as_ref()).is_none_or(|t| t.started_at != expected)) {
            return Err("原来的计时已结束，请重新选择延长时间");
        }
        s.extend_block(minutes)
    })
}

#[tauri::command]
fn finish_block(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.finish_block(now_unix()))
}

#[tauri::command]
fn set_completion_note(day_started_at: i64, entry_index: usize, ended_at: i64, note: String, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.set_completion_note(day_started_at, entry_index, ended_at, &note))
}

#[tauri::command]
fn end_break(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.end_break())
}

#[tauri::command]
fn abandon_block(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.abandon_block(now_unix()))
}

#[tauri::command]
fn end_day(app: AppHandle) -> Result<Snapshot, String> {
    let was_relevant = blocking_needs_sync(&app.state::<Shared>());
    let snap = mutate(&app, |s| s.end_day(now_unix()))?;
    if was_relevant {
        spawn_apply_blocking(&app);
    }
    Ok(snap)
}

#[tauri::command]
fn abandon_day(app: AppHandle) -> Result<Snapshot, String> {
    let was_relevant = blocking_needs_sync(&app.state::<Shared>());
    let snap = mutate(&app, |s| s.abandon_day())?;
    if was_relevant {
        spawn_apply_blocking(&app);
    }
    Ok(snap)
}

#[tauri::command]
fn update_preferences(prefs: core::Preferences, app: AppHandle) -> Result<Snapshot, String> {
    let hosts_changed = app.state::<Shared>().state.lock().unwrap().preferences.blocked_hosts != prefs.blocked_hosts;
    let snap = mutate(&app, |s| s.update_preferences(prefs))?;
    let shared = app.state::<Shared>();
    let day_active = shared.state.lock().unwrap().day.is_some();
    // 只有学习日里整站列表真的变了才同步系统规则。之前取消过一次授权的话，
    // 不能让之后每点一下配额、改一个提醒都再弹一次授权框；要重试有「核对整站规则」。
    if day_active && hosts_changed && blocking_needs_sync(&shared) {
        spawn_apply_blocking(&app);
    }
    Ok(snap)
}

#[tauri::command]
fn delete_history_day(started_at: i64, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.delete_history_day(started_at))
}

#[tauri::command]
fn normalize_host(host: String) -> Result<String, String> {
    core::validate_host(&host).map_err(|e| e.to_string())
}

#[tauri::command]
fn normalize_url(url: String) -> Result<String, String> {
    core::validate_url(&url).map_err(|e| e.to_string())
}

/// 展示随应用附带的扩展目录，安装动作由用户在浏览器扩展页完成。
#[cfg(desktop)]
#[tauri::command]
fn reveal_browser_extension(app: AppHandle) -> Result<(), String> {
    let path = app.path().resource_dir().map_err(|_| "找不到应用资源目录。")?
        .join("browser-extension");
    #[cfg(debug_assertions)]
    let path = if path.join("manifest.json").is_file() { path } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension")
    };
    if !path.join("manifest.json").is_file() {
        return Err("此安装包没有浏览器扩展，请更新坐功后重试。".into());
    }
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(&path).status();
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer").arg(&path).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = Command::new("xdg-open").arg(&path).status();
    match result {
        Ok(status) if status.success() => Ok(()),
        _ => Err("无法打开扩展目录，请从安装目录中打开 browser-extension。".into()),
    }
}

/// 立即再存一次。成不成都随快照带出去：成功清空横条，失败换上最新的原因。
#[tauri::command]
fn retry_save(app: AppHandle) -> Snapshot {
    let shared = app.state::<Shared>();
    save(&shared);
    broadcast(&app);
    snapshot(&shared, true)
}

/// 只读核对：重新读系统 hosts，比对当前学习日应有的完整托管规则。不弹授权。
#[cfg(desktop)]
#[tauri::command]
fn check_blocking(app: AppHandle) -> Snapshot {
    {
        let shared = app.state::<Shared>();
        let (hosts, enable) = {
            let state = shared.state.lock().unwrap();
            (state.preferences.blocked_hosts.clone(), state.day.is_some())
        };
        let mut blocking = shared.blocking.lock().unwrap();
        match read_system_hosts() {
            Ok(current) => refresh_hosts_status(&mut blocking, &current, &hosts, enable),
            Err(e) => blocking.error = Some(e),
        }
    }
    broadcast(&app);
    snapshot(&app.state::<Shared>(), true)
}

/// 通知权限：granted / denied / unknown。
#[cfg(desktop)]
#[tauri::command]
async fn notification_status(app: AppHandle) -> String {
    use tauri_plugin_notification::PermissionState;
    match app.notification().permission_state() {
        Ok(PermissionState::Granted) => "granted".into(),
        Ok(PermissionState::Denied) => "denied".into(),
        Ok(_) => "unknown".into(),
        Err(_) => "unknown".into(),
    }
}

#[cfg(desktop)]
#[tauri::command]
async fn request_notification_permission(app: AppHandle) -> String {
    use tauri_plugin_notification::PermissionState;
    match app.notification().request_permission() {
        Ok(PermissionState::Granted) => "granted".into(),
        Ok(PermissionState::Denied) => "denied".into(),
        _ => "unknown".into(),
    }
}

/// 打开系统的通知设置页。
/// 设置页的「试一条」：立刻按完整链路发一条，看得见就说明这条路通。
#[cfg(not(target_os = "android"))]
#[tauri::command]
async fn test_notification(app: AppHandle) {
    notify(&app, "坐功 · 试一条", "看到这条横幅，说明系统通知这条路是通的。", AlertSound::Standard);
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
async fn test_water_sound(app: AppHandle) {
    if app.state::<Shared>().state.lock().unwrap().preferences.sound_enabled {
        play_alert_sound(AlertSound::Water);
    }
}

#[cfg(desktop)]
#[tauri::command]
async fn open_notification_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let ok = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        .status();
    #[cfg(target_os = "windows")]
    let ok = Command::new("cmd").args(["/c", "start", "ms-settings:notifications"]).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let ok: std::io::Result<std::process::ExitStatus> = Err(std::io::Error::other("unsupported"));
    ok.map(|_| ()).map_err(|e| e.to_string())
}

/// 手动重新同步屏蔽：有学习日则按列表应用，没有则解除残留。
#[cfg(desktop)]
#[tauri::command]
fn reapply_blocking(app: AppHandle) -> Snapshot {
    spawn_apply_blocking(&app);
    snapshot(&app.state::<Shared>(), true)
}

/// 在 Finder / 资源管理器中显示状态文件。
#[cfg(desktop)]
#[tauri::command]
fn reveal_state_file(shared: State<'_, Shared>) -> Result<(), String> {
    let path = shared.path.clone();
    #[cfg(target_os = "macos")]
    let ok = Command::new("open").arg("-R").arg(&path).status();
    #[cfg(target_os = "windows")]
    let ok = Command::new("explorer").arg(format!("/select,{}", path.display())).status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let ok: std::io::Result<std::process::ExitStatus> =
        Err(std::io::Error::other("unsupported"));
    ok.map(|_| ()).map_err(|e| e.to_string())
}

/// 打包时写进 Cargo.toml 的版本号，与 tauri.conf.json 同源。
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(desktop)]
#[tauri::command]
fn autostart_status(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[cfg(desktop)]
#[tauri::command]
fn set_autostart(enabled: bool, app: AppHandle) -> Result<bool, String> {
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }
        .map_err(|e| e.to_string())?;
    Ok(launcher.is_enabled().unwrap_or(false))
}

// IPC 名称保持一致；桌面能力在手机上明确拒绝，能力入口由 A3 隐藏。
#[cfg(mobile)]
#[tauri::command]
fn reveal_browser_extension(_app: AppHandle) -> Result<(), String> {
    Err("移动端不支持安装桌面浏览器扩展。".into())
}

#[cfg(mobile)]
fn unsupported_blocking_snapshot(app: &AppHandle) -> Snapshot {
    app.state::<Shared>().blocking.lock().unwrap().error = Some("移动端不支持网站屏蔽。".into());
    broadcast(app);
    snapshot(&app.state::<Shared>(), true)
}

#[cfg(mobile)]
#[tauri::command]
fn check_blocking(app: AppHandle) -> Snapshot { unsupported_blocking_snapshot(&app) }

#[cfg(mobile)]
#[tauri::command]
fn reapply_blocking(app: AppHandle) -> Snapshot { unsupported_blocking_snapshot(&app) }

#[cfg(all(mobile, not(target_os = "android")))]
#[tauri::command]
async fn notification_status(_app: AppHandle) -> String { "unknown".into() }

#[cfg(all(mobile, not(target_os = "android")))]
#[tauri::command]
async fn request_notification_permission(_app: AppHandle) -> Result<String, String> {
    Err("移动端暂不支持请求通知权限。".into())
}

#[cfg(all(mobile, not(target_os = "android")))]
#[tauri::command]
async fn open_notification_settings() -> Result<(), String> {
    Err("移动端暂不支持打开通知设置。".into())
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn notification_status(app: AppHandle) -> Result<String, String> {
    mobile::notification_status(&app)
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn request_notification_permission(app: AppHandle) -> Result<String, String> {
    mobile::request_notification_permission(&app)
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn open_notification_settings(app: AppHandle) -> Result<(), String> {
    mobile::open_settings(&app, platform::SettingsRequest::new(
        platform::SettingsTarget::AppNotifications, None
    )?)
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn test_notification(app: AppHandle) -> Result<(), String> {
    mobile::test_notification(&app, false)
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn test_water_sound(app: AppHandle) -> Result<(), String> {
    mobile::test_notification(&app, true)
}

#[tauri::command]
fn platform_info() -> platform::PlatformInfo { platform::info() }

#[tauri::command]
async fn system_status(app: AppHandle) -> Result<platform::SystemStatus, String> {
    #[cfg(target_os = "android")]
    { mobile::system_status(&app) }
    #[cfg(not(target_os = "android"))]
    { let _ = app; Err("系统状态查询仅支持 Android。".into()) }
}

#[tauri::command]
async fn open_system_settings(
    target: platform::SettingsTarget,
    channel_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let request = platform::SettingsRequest::new(target, channel_id)?;
    #[cfg(target_os = "android")]
    { mobile::open_settings(&app, request) }
    #[cfg(not(target_os = "android"))]
    { let _ = (app, request); Err("此系统设置入口仅支持 Android。".into()) }
}

#[tauri::command]
async fn move_task_to_back(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    { mobile::move_task_to_back(&app) }
    #[cfg(not(target_os = "android"))]
    { let _ = app; Err("退到后台仅支持 Android。".into()) }
}

#[cfg(mobile)]
#[tauri::command]
fn reveal_state_file(_shared: State<'_, Shared>) -> Result<(), String> {
    Err("移动端不支持在文件管理器中显示状态文件。".into())
}

#[cfg(mobile)]
#[tauri::command]
fn autostart_status(_app: AppHandle) -> bool { false }

#[cfg(mobile)]
#[tauri::command]
fn set_autostart(enabled: bool, _app: AppHandle) -> Result<bool, String> {
    let _ = enabled;
    Err("移动端不支持桌面开机自启。".into())
}

#[cfg(mobile)]
#[tauri::command]
async fn quit_after_save(_app: AppHandle) -> Result<(), String> {
    Err("移动端请使用系统返回键退到后台。".into())
}

#[cfg(mobile)]
#[tauri::command]
fn quit_leaving_blocking(_app: AppHandle) -> Result<(), String> {
    Err("移动端请使用系统返回键退到后台。".into())
}

#[cfg(mobile)]
#[tauri::command]
fn quit_without_saving(_app: AppHandle) -> Result<(), String> {
    Err("移动端请使用系统返回键退到后台。".into())
}

// ---------- 启动 ----------

/// 读档的三种结局。**只有「文件确实不存在」才算首次启动**——权限、I/O、卷没挂载
/// 这些错误如果也当成首启，接下来的 save() 就会把用户还在的存档覆盖掉。
enum InitialPlan {
    Fresh,
    Loaded(Box<core::State>),
    Protected(String),
}

fn io_reason(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::PermissionDenied => "没有读取权限",
        std::io::ErrorKind::NotFound => "文件不存在",
        _ => "系统读取失败",
    }
}

fn decide_initial(read: Result<String, std::io::ErrorKind>) -> InitialPlan {
    match read {
        Ok(raw) => match core::from_json(&raw) {
            Ok(state) => InitialPlan::Loaded(Box::new(state)),
            Err(reason) => InitialPlan::Protected(format!(
                "状态文件无法解析（{reason}），已进入保护模式：原文件保持原样，本次运行不会写盘。"
            )),
        },
        Err(std::io::ErrorKind::NotFound) => InitialPlan::Fresh,
        Err(kind) => InitialPlan::Protected(format!(
            "状态文件读不出来（{}），已进入保护模式：原文件保持原样，本次运行不会写盘。",
            io_reason(kind)
        )),
    }
}

/// 首启拿到的是内置默认计划；读得出来就接着用，读不出来进保护模式。
fn load_initial(path: &PathBuf) -> (core::State, Option<String>) {
    match decide_initial(fs::read_to_string(path).map_err(|e| e.kind())) {
        InitialPlan::Loaded(state) => {
            let mut state = *state;
            resume_time(&mut state, now_unix(), time_policy());
            (state, None)
        }
        InitialPlan::Protected(message) => (core::State::new(now_unix()), Some(message)),
        InitialPlan::Fresh => (core::State::new(now_unix()), None),
    }
}

/// `SITZFLEISCH_DATA_DIR` 的严格语义：没设置就用默认目录；设置了就必须是绝对路径，
/// 空、只有空白、相对路径、非 UTF-8 一律报错停止。**绝不退回真实目录**——
/// 隔离测试写进用户的真实存档比崩掉严重得多。
fn resolve_data_dir(
    env: Option<Result<String, std::env::VarError>>,
    default: PathBuf,
) -> Result<PathBuf, String> {
    match env {
        None => Ok(default),
        Some(Err(_)) => Err("不是有效的 UTF-8 路径".into()),
        Some(Ok(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("已设置但为空".into());
            }
            let path = PathBuf::from(trimmed);
            if !path.is_absolute() {
                return Err(format!("必须是绝对路径：{trimmed}"));
            }
            Ok(path)
        }
    }
}

#[cfg(target_os = "macos")]
fn install_chinese_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{PredefinedMenuItem, Submenu};
    let app_menu = Submenu::with_items(
        app,
        "坐功",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("关于 坐功"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("隐藏 坐功"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("全部显示"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "app-quit", "退出 坐功", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("拷贝"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
        ],
    )?;
    app.set_menu(Menu::with_items(app, &[&app_menu, &edit_menu, &window_menu])?)?;
    Ok(())
}

/// 窗口位置与尺寸要记住：这个 App 常年开在副屏上，每次启动都跳回屏幕中央很烦。
/// 只记 size 与 position——「关窗不退出」意味着窗口经常是隐藏的，记 visible 只会打架。
#[cfg(desktop)]
const WINDOW_STATE: StateFlags = StateFlags::SIZE.union(StateFlags::POSITION);

/// 正常退出先在后台清理，授权取消或写入失败时保留应用供用户重试。
#[cfg(desktop)]
fn begin_exit_cleanup(app: &AppHandle) {
    let shared = app.state::<Shared>();
    if shared.exiting.swap(true, Ordering::SeqCst) { return; }
    let app = app.clone();
    thread::spawn(move || { let _ = complete_exit_cleanup(&app); });
}

#[cfg(desktop)]
fn complete_exit_cleanup(app: &AppHandle) -> Result<(), String> {
    let shared = app.state::<Shared>();
    if let Err(reason) = release_system_blocking(&shared) {
        shared.exiting.store(false, Ordering::SeqCst);
        // 清理取消后进程继续运行，之前的「不保存退出」不能影响以后的退出。
        *shared.declined_final_save.lock().unwrap() = false;
        broadcast(app);
        show_main_window(app);
        let _ = app.emit("blocking://quit-blocked", &reason);
        return Err(reason);
    }
    shared.blocking_released.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

/// 退出前那次保存的结论。**纯函数**，好测；窗口与进程操作全留在调用方。
#[cfg_attr(mobile, allow(dead_code))]
enum QuitDecision {
    Exit,
    Stay(String),
}

#[cfg(desktop)]
fn decide_quit(save_error: Option<String>) -> QuitDecision {
    match save_error {
        None => QuitDecision::Exit,
        Some(reason) => QuitDecision::Stay(reason),
    }
}

/// 存不下来就不退：内存里的改动原样留着，把主窗口叫回来问用户。
#[cfg(desktop)]
fn quit_saving(app: &AppHandle) {
    let shared = app.state::<Shared>();
    shared.state.lock().unwrap().tick(now_unix());
    save(&shared);
    // 独立语句：guard 在这个分号处就还回去了，下面的窗口与退出操作不带任何锁。
    let error = shared.save_error.lock().unwrap().clone();
    match decide_quit(error) {
        QuitDecision::Exit => {
            // 关窗不退出，所以窗口多半没被销毁过，插件自己的时机等不到——退出前显式存一次。
            let _ = app.save_window_state(WINDOW_STATE);
            app.exit(0);
        }
        QuitDecision::Stay(reason) => {
            show_main_window(app);
            let _ = app.emit("save://quit-blocked", reason);
        }
    }
}

/// 退出前保存失败后的「重试并退出」。成功就没有下文了；失败把最新原因端回对话框。
#[cfg(desktop)]
#[tauri::command]
async fn quit_after_save(app: AppHandle) -> Result<(), String> {
    // 重试框必须等清理真正完成再恢复可点，避免授权仍在等待时出现“留在这里”按钮。
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        if shared.exiting.swap(true, Ordering::SeqCst) {
            return Err("正在退出坐功，请等待网站屏蔽解除。".into());
        }
        shared.state.lock().unwrap().tick(now_unix());
        save(&shared);
        if let Some(reason) = shared.save_error.lock().unwrap().clone() {
            shared.exiting.store(false, Ordering::SeqCst);
            return Err(reason);
        }
        let _ = app.save_window_state(WINDOW_STATE);
        complete_exit_cleanup(&app)
    }).await.map_err(|error| format!("退出处理失败：{error}"))?
}

/// 「仍然退出」：系统 hosts 一直解除不了（授权一再被拒、托管标记损坏）时，不能把人困在应用里。
/// 照常保存；系统 hosts 里坐功那一段先留着，下次打开坐功会核对并提示解除。
#[cfg(desktop)]
#[tauri::command]
fn quit_leaving_blocking(app: AppHandle) -> Result<(), String> {
    let shared = app.state::<Shared>();
    shared.state.lock().unwrap().tick(now_unix());
    save(&shared);
    if let Some(reason) = shared.save_error.lock().unwrap().clone() {
        return Err(reason);
    }
    // 标成已处理：退出事件里不再尝试清理，免得刚说完「仍然退出」又弹一次授权框。
    shared.blocking_released.store(true, Ordering::SeqCst);
    let _ = app.save_window_state(WINDOW_STATE);
    app.exit(0);
    Ok(())
}

/// 「不保存退出」：磁盘上保持上一次成功写入的完整文件。
#[cfg(desktop)]
#[tauri::command]
fn quit_without_saving(app: AppHandle) {
    // 先立标记再退出：`app.exit(0)` 之后还会走一次 `RunEvent::Exit`，
    // 那里的兜底保存必须让路，不然等于替用户把他刚拒绝的东西写了回去。
    *app.state::<Shared>().declined_final_save.lock().unwrap() = true;
    let _ = app.save_window_state(WINDOW_STATE);
    app.exit(0);
}

/// 进程真的要走了：还存不存这一次。**纯函数**，好测。
///
/// Dock 右键「退出」、AppleScript `quit`、注销都不经过 `quit_saving()`——
/// tao 没注册 `applicationShouldTerminate:`，Tauri 层拿不到可阻止的 `ExitRequested`，
/// 只剩 `applicationWillTerminate:` 转过来的 `RunEvent::Exit` 这一下。
/// 拦不住，但存得了：磁盘好使的时候，这一下把上次定期保存之后的计时补上。
fn should_save_on_exit(declined: bool) -> bool {
    !declined
}

/// `RunEvent::Exit` 的落点。保护模式与保存失败都由 `save()` 自己兜着，这里不额外开口子。
fn final_save_on_exit(shared: &Shared) {
    if !should_save_on_exit(*shared.declined_final_save.lock().unwrap()) {
        return;
    }
    advance_time(&mut shared.state.lock().unwrap(), now_unix(), time_policy());
    save(shared);
}

/// 环境变量那一路的严格判定 + 建目录。返回 `None` 表示压根没设置，走默认目录。
fn checked_data_dir_override() -> Result<Option<PathBuf>, String> {
    let env = match std::env::var("SITZFLEISCH_DATA_DIR") {
        Err(std::env::VarError::NotPresent) => return Ok(None),
        other => Some(other),
    };
    // 这一支一定带着环境变量，`resolve_data_dir` 的默认值分支走不到。
    let dir = resolve_data_dir(env, PathBuf::new())?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建不了这个目录：{e}"))?;
    Ok(Some(dir))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置了 SITZFLEISCH_DATA_DIR 就必须当场能用，否则报错停止，**绝不退回真实目录**。
    // 判定放在建窗口之前：Tauri 先按配置建窗口、再调 setup，判晚了错的路径会先闪一下窗口。
    let data_dir_override = match checked_data_dir_override() {
        Ok(dir) => dir,
        Err(why) => {
            eprintln!("sitzfleisch: SITZFLEISCH_DATA_DIR 无效：{why}");
            std::process::exit(1);
        }
    };
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例只负责把已有窗口叫出来。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(WINDOW_STATE)
                .build(),
        );
    let builder = builder.plugin(tauri_plugin_notification::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(tauri_plugin_sitzfleisch_android::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "app-quit" {
                quit_saving(app);
            }
        });
    let builder = builder.setup(move |app| {
            // 没被挪走就用系统的应用数据目录；挪走了的那一份在建窗口之前就判过了。
            let isolated = data_dir_override.is_some();
            let dir = match data_dir_override {
                Some(dir) => dir,
                None => {
                    let dir = app.path().app_data_dir()?;
                    fs::create_dir_all(&dir)?;
                    dir
                }
            };
            let path = dir.join("state.json");
            let (state, protection) = load_initial(&path);

            // 建窗前先只读体检；Ready 后按保留的学习日恢复规则或清理残留。
            #[cfg(desktop)]
            let mut blocking = BlockingStatus::default();
            #[cfg(mobile)]
            let blocking = BlockingStatus::default();
            #[cfg(desktop)]
            match read_system_hosts() {
                Ok(current) => refresh_hosts_status(&mut blocking, &current, &state.preferences.blocked_hosts, state.day.is_some()),
                Err(e) => blocking.error = Some(e),
            }

            let (qa_view, qa_scroll) = qa_launch_args();
            app.manage(Shared {
                state: Mutex::new(state),
                exiting: AtomicBool::new(false),
                blocking_released: AtomicBool::new(false),
                isolated,
                snapshot_revision: AtomicU64::new(0),
                save_lock: Mutex::new(()),
                blocking_lock: Mutex::new(()),
                qa_view,
                qa_scroll,
                write_protected: Mutex::new(protection),
                blocking: Mutex::new(blocking),
                browser_bridge: browser_blocking::new_bridge(),
                save_error: Mutex::new(None),
                declined_final_save: Mutex::new(false),
                path,
                data_dir: dir.clone(),
                tray_shape: Mutex::new(None),
                tray_display: Mutex::new(None),
            });
            #[cfg(target_os = "android")]
            app.manage(mobile::MobileShared::new(dir.join("alarms.json")));
            #[cfg(desktop)]
            browser_blocking::start(app.handle(), isolated);
            #[cfg(target_os = "macos")]
            install_chinese_menu(app)?;

            #[cfg(desktop)]
            {
                let menu = {
                    let shared = app.state::<Shared>();
                    let shape = tray_shape(&shared.state.lock().unwrap());
                    let menu = build_tray_menu(app.handle(), &shape)?;
                    *shared.tray_shape.lock().unwrap() = Some(shape);
                    menu
                };
                let initial_display = tray_display(&app.state::<Shared>().state.lock().unwrap());
                let tray_builder = TrayIconBuilder::with_id("main")
                    .icon(tray_icon(app.default_window_icon().unwrap(), initial_display.paused))
                    .tooltip(&initial_display.tooltip)
                    .menu(&menu)
                    .show_menu_on_left_click(true)
                    .on_menu_event(|app, event| handle_tray_menu(app, event.id.as_ref()));
                #[cfg(target_os = "macos")]
                let tray_builder = tray_builder.title(&initial_display.title);
                let _tray = tray_builder.build(app)?;
                #[cfg(target_os = "macos")]
                configure_tray_digits(&_tray)?;
                *app.state::<Shared>().tray_display.lock().unwrap() = Some(initial_display);
            }

            // 每秒走一格；只有「自然走完」的转变才配通知（用户手点的不用提醒自己）。
            let handle = app.handle().clone();
            thread::spawn(move || {
                #[cfg(target_os = "android")]
                {
                    mobile::set_heartbeat_thread(&handle);
                }
                #[cfg(target_os = "android")]
                let mut initial_sync = true;
                let mut ticks: u64 = 0;
                loop {
                    #[cfg(not(target_os = "android"))]
                    thread::sleep(Duration::from_secs(1));
                    #[cfg(target_os = "android")]
                    {
                        if initial_sync { initial_sync = false; }
                        else { thread::park_timeout(Duration::from_secs(1)); }
                    }
                    ticks += 1;
                    let step = {
                        let shared = handle.state::<Shared>();
                        let mut state = shared.state.lock().unwrap();
                        let now = now_unix();
                        let step = advance_heartbeat(&mut state, now);
                        drop(state);
                        if ticks.is_multiple_of(30) {
                            save(&shared);
                        }
                        step
                    };
                    if step.focus_done && step.show_banners {
                        notify_block_finished(&handle);
                    }
                    if step.break_done && step.show_banners {
                        let (title, body, _) = {
                            let shared = handle.state::<Shared>();
                            let state = shared.state.lock().unwrap();
                            let alert = core::PlannedAlert { kind: core::AlertKind::RestOver, at: state.last_tick, pause_started_at: None };
                            alerts::alert_copy(alert.kind, &state, &alert)
                        };
                        notify(&handle, title, &body, AlertSound::Standard);
                    }
                    if step.show_banners {
                        for (title, body, sound) in step.reminders {
                            notify(&handle, title, &body, sound);
                        }
                    }
                    #[cfg(target_os = "android")]
                    mobile::sync(&handle, step.desired, step.status, step.now);
                    broadcast(&handle);
                }
            });

            // 通知权限得主动要一次，否则 macOS 根本不会把这个 App 登记进通知中心，
            // 表现出来就是「设置里找不到它，也永远收不到提醒」。放后台线程，别挡住启动。
            #[cfg(desktop)]
            {
                use tauri_plugin_notification::PermissionState;
                let handle = app.handle().clone();
                let probe = std::env::args().any(|a| a == "--qa-notify");
                let report_path = dir.join("qa-notify.txt");
                thread::spawn(move || {
                    let before = handle.notification().permission_state();
                    let asked = matches!(before, Ok(PermissionState::Granted) | Ok(PermissionState::Denied));
                    let requested = if asked { None } else { Some(handle.notification().request_permission()) };
                    if probe {
                        // QA：把权限与投递结果写到文件，好在没有屏幕的情况下核对。
                        thread::sleep(Duration::from_secs(2));
                        let shown = handle
                            .notification()
                            .builder()
                            .title("坐功 · 通知自检")
                            .body("看到这条就说明系统通知能发出来。")
                            .show();
                        let report = format!(
                            "before={before:?}\nrequested={requested:?}\nafter={:?}\nshow={shown:?}\n",
                            handle.notification().permission_state()
                        );
                        let _ = fs::write(&report_path, report);
                    }
                });
            }

            broadcast(app.handle());
            Ok(())
        });
    #[cfg(desktop)]
    let builder = builder.on_window_event(|window, event| {
            // 关窗不退出：计时挂在托盘继续走，和 Mac 主程序一个规矩。
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        });
    #[cfg(target_os = "android")]
    let builder = builder.on_window_event(|window, event| {
        match event {
            WindowEvent::Suspended => mobile::on_lifecycle(window.app_handle(), true),
            WindowEvent::Resumed => mobile::on_lifecycle(window.app_handle(), false),
            _ => {},
        }
    });
    builder.invoke_handler(tauri::generate_handler![
            get_snapshot,
            browser_pairing_code,
            reset_browser_pairing,
            start_day,
            switch_profile,
            start_block,
            toggle_pause,
            extend_block,
            finish_block,
            set_completion_note,
            end_break,
            abandon_block,
            end_day,
            abandon_day,
            update_preferences,
            delete_history_day,
            normalize_host,
            normalize_url,
            reveal_browser_extension,
            retry_save,
            quit_after_save,
            quit_without_saving,
            quit_leaving_blocking,
            reapply_blocking,
            check_blocking,
            notification_status,
            request_notification_permission,
            test_notification,
            test_water_sound,
            open_notification_settings,
            reveal_state_file,
            autostart_status,
            set_autostart,
            app_version,
            platform_info,
            system_status,
            open_system_settings,
            move_task_to_back
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(&event, tauri::RunEvent::Ready) {
                let shared = app.state::<Shared>();
                // 再次打开未结束的学习日时恢复规则；上次异常退出的残留也按当前状态核对。
                if !shared.isolated && shared.write_protected.lock().unwrap().is_none()
                    && blocking_needs_sync(&shared) {
                    spawn_apply_blocking(app);
                }
            }
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if !app.state::<Shared>().blocking_released.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    begin_exit_cleanup(app);
                }
            }
            #[cfg(target_os = "macos")]
            match &event {
                tauri::RunEvent::Ready => space_preview::install(app),
                tauri::RunEvent::Exit => space_preview::uninstall(),
                _ => {}
            }
            // Dock 右键「退出」、AppleScript `quit`、注销都不走 `app-quit` 菜单项，
            // 于是绕过了 `quit_saving()`。这一层拦不住它们（tao 没注册
            // `applicationShouldTerminate:`），但 `applicationWillTerminate:` 会转成
            // `RunEvent::Exit`，还来得及把盘存了。⌘Q 那条路走到这里时已经存过一次，
            // 再存一次无害；用户点过「不保存退出」时则会被 `declined_final_save` 挡住。
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(shared) = app.try_state::<Shared>() {
                    final_save_on_exit(&shared);
                    // Dock 退出/注销可能绕过 ExitRequested；仍尝试清理自己的 hosts 段。
                    shared.exiting.store(true, Ordering::SeqCst);
                    if !shared.blocking_released.load(Ordering::SeqCst) {
                        if let Err(error) = release_system_blocking(&shared) {
                            eprintln!("sitzfleisch: 退出时未能解除系统屏蔽：{error}");
                        }
                    }
                }
            }
        });
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Barrier};

    pub(super) struct TestShared(pub(super) Shared);

    impl TestShared {
        pub(super) fn new(state: core::State) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "sitzfleisch-runtime-{}-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&dir).unwrap();
            Self(Shared {
                state: Mutex::new(state),
                exiting: AtomicBool::new(false),
                blocking_released: AtomicBool::new(false),
                isolated: true,
                snapshot_revision: AtomicU64::new(0),
                save_lock: Mutex::new(()),
                blocking_lock: Mutex::new(()),
                qa_view: None,
                qa_scroll: 0,
                write_protected: Mutex::new(None),
                blocking: Mutex::new(BlockingStatus::default()),
                browser_bridge: browser_blocking::new_bridge(),
                save_error: Mutex::new(None),
                declined_final_save: Mutex::new(false),
                path: dir.join("state.json"),
                data_dir: dir,
                tray_shape: Mutex::new(None),
                tray_display: Mutex::new(None),
            })
        }
    }

    impl Drop for TestShared {
        fn drop(&mut self) {
            fs::remove_dir_all(self.0.path.parent().unwrap()).unwrap();
        }
    }

    fn state_with_day() -> core::State {
        let mut state = core::State::new(1_000);
        state.start_day("standard", 1_000).unwrap();
        state
    }

    #[test]
    fn exit_cleanup_only_removes_owned_hosts_and_cleans_the_staged_file() {
        let fixture = TestShared::new(core::State::new(1_000));
        let dir = &fixture.0.data_dir;
        let path = dir.join("test.hosts");
        let original = "127.0.0.1 localhost\n::1 localhost\n192.0.2.10 custom.example # keep\n";
        let blocked = core::render_hosts(original, &["live.bilibili.com".into()], true);
        fs::write(&path, &blocked).unwrap();
        let result = sync_hosts_file(&path, &[], false, |content| {
            fs::write(&path, content).map_err(|e| e.to_string())
        });
        assert_eq!(result, Ok(false));
        assert_eq!(fs::read_to_string(&path).unwrap().trim(), original.trim());
        assert!(!dir.join(format!("hosts.staged.{}", std::process::id())).exists());
        assert_eq!(sync_hosts_file(&path, &[], false, |_| panic!("没有托管段时不能提权")), Ok(false));
    }

    #[test]
    fn exit_cleanup_reports_denied_or_unapplied_writes_without_losing_hosts() {
        let fixture = TestShared::new(core::State::new(1_000));
        let dir = &fixture.0.data_dir;
        let path = dir.join("test.hosts");
        let blocked = core::render_hosts("127.0.0.1 localhost\n", &["live.bilibili.com".into()], true);
        fs::write(&path, &blocked).unwrap();
        assert!(sync_hosts_file(&path, &[], false, |_| Err("授权被取消".into())).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), blocked);
        assert!(!dir.join(format!("hosts.staged.{}", std::process::id())).exists());
        assert!(sync_hosts_file(&path, &[], false, |_| Ok(())).is_err(), "必须核对真正解除，不能只信提权进程退出码");
    }

    #[test]
    fn malformed_managed_markers_never_remove_unowned_hosts_entries() {
        let fixture = TestShared::new(core::State::new(1_000));
        let dir = &fixture.0.data_dir;
        let path = dir.join("test.hosts");
        for markers in [core::HOSTS_BEGIN.to_string(), core::HOSTS_END.to_string(),
            format!("{}\n{}\n{}", core::HOSTS_BEGIN, core::HOSTS_BEGIN, core::HOSTS_END)] {
            let content = format!("127.0.0.1 localhost\n{markers}\n192.0.2.10 custom.example\n");
            fs::write(&path, &content).unwrap();
            assert!(sync_hosts_file(&path, &[], false, |_| panic!("标记异常时不能调用提权写入")).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn queued_enable_cannot_reinstate_hosts_after_exit_begins() {
        let fixture = std::sync::Arc::new(TestShared::new(state_with_day()));
        fixture.0.state.lock().unwrap().preferences.blocked_hosts = vec!["live.bilibili.com".into()];
        let before = serde_json::to_string(&*fixture.0.state.lock().unwrap()).unwrap();
        let held = fixture.0.blocking_lock.lock().unwrap();
        let worker = fixture.clone();
        let task = thread::spawn(move || {
            apply_latest_blocking(&worker.0, |hosts, enable| {
                assert_eq!(hosts, &["live.bilibili.com"]);
                assert!(!enable, "退出开始后，排队的启用请求只能清理规则");
            });
        });
        fixture.0.exiting.store(true, Ordering::SeqCst);
        drop(held);
        task.join().unwrap();
        assert!(!blocking_wanted(&fixture.0));
        assert_eq!(serde_json::to_string(&*fixture.0.state.lock().unwrap()).unwrap(), before);
        fixture.0.exiting.store(false, Ordering::SeqCst);
        assert!(blocking_wanted(&fixture.0), "取消退出或下次启动后，未结束的学习日仍可恢复屏蔽");
    }

    #[test]
    fn isolated_exit_never_touches_real_system_hosts() {
        let fixture = TestShared::new(state_with_day());
        assert!(fixture.0.isolated);
        assert!(release_system_blocking(&fixture.0).is_ok());
        assert!(fixture.0.state.lock().unwrap().day.is_some());
    }

    fn advance_awake_seconds(state: &mut core::State, seconds: i64) {
        let until = state.last_tick + seconds;
        // 用正常心跳推进，避免把测试里的大步进误判为休眠。
        while state.last_tick < until {
            state.tick((state.last_tick + 60).min(until));
        }
    }

    fn state_with_idle_reminder() -> core::State {
        let mut state = state_with_day();
        state.preferences.idle_reminder_enabled = true;
        state.preferences.idle_reminder_minutes = 10;
        state
    }

    #[test]
    fn idle_notifications_report_the_whole_pause_after_each_interval() {
        let mut state = state_with_idle_reminder();
        for expected_minutes in [10, 20, 30] {
            advance_awake_seconds(&mut state, 600);
            assert_eq!(
                take_due_reminder_notifications(&mut state),
                vec![("还没开格", format!("已经暂停 {expected_minutes} 分钟了。"), AlertSound::Standard)],
            );
            assert_eq!(state.day.as_ref().unwrap().paused_without_block, 0);
            assert!(take_due_reminder_notifications(&mut state).is_empty(), "同一次到期只消费一次");
        }
    }

    #[test]
    fn idle_notifications_reset_for_a_new_pause_and_include_its_break() {
        let mut state = state_with_idle_reminder();
        advance_awake_seconds(&mut state, 1_800);
        take_due_reminder_notifications(&mut state);
        state.start_block_with_break("main", 25, vec![], 5).unwrap();
        advance_awake_seconds(&mut state, 60);
        state.finish_block(state.last_tick).unwrap();

        advance_awake_seconds(&mut state, 300);
        assert!(take_due_reminder_notifications(&mut state).is_empty());
        assert!(state.take_due_break(state.last_tick));
        advance_awake_seconds(&mut state, 300);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "休息不算闲置：休息结束后才开始数");
        advance_awake_seconds(&mut state, 300);
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![("还没开格", "已经暂停 15 分钟了。".into(), AlertSound::Standard)],
            "休息后再停 10 分钟才催；文案里的暂停时长照实含休息，不累计上一段暂停",
        );
        assert_eq!(state.day.as_ref().unwrap().paused_seconds, 2_700);
    }

    #[test]
    fn idle_notifications_include_suspend_and_restart_without_advancing_the_reminder_counter() {
        let mut state = state_with_idle_reminder();
        advance_awake_seconds(&mut state, 300);
        state.tick(state.last_tick + 600);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "休眠不改变原有提醒节奏");
        advance_awake_seconds(&mut state, 300);
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![("还没开格", "已经暂停 20 分钟了。".into(), AlertSound::Standard)],
        );

        state = core::from_json(&core::to_json(&state)).unwrap();
        state.resume_after_restart(state.last_tick + 1_800);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "重启不额外触发提醒");
        advance_awake_seconds(&mut state, 600);
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![("还没开格", "已经暂停 60 分钟了。".into(), AlertSound::Standard)],
            "保存重启后仍从这一段暂停的原始起点累计",
        );
        assert_eq!(state.day.as_ref().unwrap().suspend_seconds, 2_400);
    }

    #[test]
    fn reminder_notifications_keep_existing_manual_pause_and_disabled_gates() {
        let mut state = state_with_idle_reminder();
        state.start_block("main", 25, vec![]).unwrap();
        state.toggle_pause(state.last_tick).unwrap();
        advance_awake_seconds(&mut state, 1_800);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "手动暂停已有格时不新增闲置提醒");

        state.abandon_block(state.last_tick).unwrap();
        state.preferences.idle_reminder_enabled = false;
        advance_awake_seconds(&mut state, 600);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "关闭提醒后仍不投递");
    }

    #[test]
    fn water_notifications_use_local_half_hours_and_their_own_sound() {
        let mut state = core::State::new(1_000);
        state.preferences.water_reminder_enabled = true;
        let now = Local::now();
        let boundary = now.with_minute(30).unwrap().with_second(0).unwrap().timestamp();
        state.last_tick = boundary - 1;
        state.water_clock_checked_at = Some(boundary - 1);
        assert!(take_due_reminder_notifications(&mut state).is_empty());
        state.tick(boundary);
        assert_eq!(take_due_reminder_notifications(&mut state), vec![
            ("喝点水吧", "忙了一阵，喝几口水再继续。".into(), AlertSound::Water),
        ]);
        assert!(take_due_reminder_notifications(&mut state).is_empty());
    }

    #[test]
    fn snapshots_order_commands_even_when_the_clock_does_not_advance() {
        let fixture = TestShared::new(core::State::new(1_000));
        let first = snapshot(&fixture.0, true);
        fixture.0.state.lock().unwrap().preferences.break_minutes = 15;
        let second = snapshot(&fixture.0, false);
        let third = snapshot(&fixture.0, true);
        assert_eq!(first.state.last_tick, second.state.last_tick);
        assert!(first.revision < second.revision && second.revision < third.revision);
        assert_eq!(first.state.preferences.break_minutes, 10);
        assert_eq!(second.state.preferences.break_minutes, 15);
        assert!(second.history.is_none());
        assert!(third.history.is_some());
    }

    #[test]
    fn queued_save_reads_state_after_acquiring_the_save_lock() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        let serial = shared.save_lock.lock().unwrap();
        thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let writer = scope.spawn(move || {
                started_tx.send(()).unwrap();
                save(shared);
                done_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            let saved_while_locked = done_rx.recv_timeout(Duration::from_millis(50)).is_ok();
            shared.state.lock().unwrap().last_tick = 2_000;
            drop(serial);
            writer.join().unwrap();
            assert!(!saved_while_locked, "另一个保存还没结束时不能取快照或写盘");
        });
        let saved = core::from_json(&fs::read_to_string(&shared.path).unwrap()).unwrap();
        assert_eq!(saved.last_tick, 2_000, "排队前的旧状态不能盖回最新存档");
        assert!(!shared.path.with_extension("json.tmp").exists());
    }

    #[test]
    fn concurrent_saves_leave_complete_monotonic_snapshots() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        {
            // 存档要够大，读者才有机会撞上写到一半的文件。计划字段有长度上限，用完成记录撑大。
            let mut state = shared.state.lock().unwrap();
            state.start_day("standard", 1_000).unwrap();
            let day = state.day.as_mut().unwrap();
            for _ in 0..12 {
                day.ledger.push(core::LedgerEntry {
                    category: "main".into(),
                    seconds: 60,
                    accepted: true,
                    tasks: vec![],
                    started_at: 1_000,
                    ended_at: 1_060,
                    completion_note: Some("研究".repeat(1_000)),
                });
            }
        }
        save(shared);
        let start = Barrier::new(5);
        let done = AtomicBool::new(false);
        thread::scope(|scope| {
            let reader = scope.spawn(|| {
                start.wait();
                let mut last_tick = 1_000;
                let mut reads = 0;
                loop {
                    let raw = fs::read_to_string(&shared.path).unwrap();
                    let state = core::from_json(&raw).expect("原子替换的读者只能看到完整存档");
                    assert!(state.last_tick >= last_tick, "旧快照不能覆盖新快照");
                    last_tick = state.last_tick;
                    reads += 1;
                    if done.load(Ordering::Acquire) { break }
                }
                reads
            });
            let writers: Vec<_> = (0..4).map(|_| scope.spawn(|| {
                start.wait();
                for _ in 0..32 {
                    shared.state.lock().unwrap().last_tick += 1;
                    save(shared);
                }
            })).collect();
            for writer in writers { writer.join().unwrap(); }
            done.store(true, Ordering::Release);
            assert!(reader.join().unwrap() > 0);
        });
        let saved = core::from_json(&fs::read_to_string(&shared.path).unwrap()).unwrap();
        assert_eq!(saved.last_tick, 1_128);
        assert!(!shared.path.with_extension("json.tmp").exists());
    }

    #[test]
    fn failed_or_protected_save_preserves_the_existing_file() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        save(shared);
        let before = fs::read(&shared.path).unwrap();
        shared.state.lock().unwrap().last_tick = 2_000;
        let tmp = shared.path.with_extension("json.tmp");
        fs::create_dir(&tmp).unwrap();
        save(shared);
        assert_eq!(fs::read(&shared.path).unwrap(), before);
        fs::remove_dir(&tmp).unwrap();
        *shared.write_protected.lock().unwrap() = Some("保护模式".into());
        save(shared);
        assert_eq!(fs::read(&shared.path).unwrap(), before);
        assert!(!tmp.exists());
    }

    /// 用一个同名目录占住暂存文件的位置，`fs::write` 就一定失败。
    fn block_the_temp_file(shared: &Shared) -> PathBuf {
        let tmp = shared.path.with_extension("json.tmp");
        fs::create_dir(&tmp).unwrap();
        tmp
    }

    #[test]
    fn save_error_is_set_on_failure_and_cleared_on_success() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        let tmp = block_the_temp_file(shared);
        save(shared);
        let failure = shared.save_error.lock().unwrap().clone().expect("写不进去必须留下原因");
        assert!(failure.contains("os error"), "原因得是系统给的那句话：{failure}");
        assert!(snapshot(shared, false).save_error.is_some(), "失败得随快照走到界面");
        fs::remove_dir(&tmp).unwrap();
        save(shared);
        assert!(shared.save_error.lock().unwrap().is_none(), "成功一次就清空");
        assert!(snapshot(shared, false).save_error.is_none());
    }

    #[test]
    fn save_error_appears_after_a_success() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        save(shared);
        assert!(shared.save_error.lock().unwrap().is_none());
        let tmp = block_the_temp_file(shared);
        shared.state.lock().unwrap().last_tick = 2_000;
        save(shared);
        assert!(shared.save_error.lock().unwrap().is_some(), "先成功后失败也要报出来");
        fs::remove_dir(&tmp).unwrap();
    }

    #[test]
    fn protected_mode_never_reports_save_error() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        *shared.write_protected.lock().unwrap() = Some("保护模式".into());
        let tmp = block_the_temp_file(shared);
        save(shared);
        assert!(shared.save_error.lock().unwrap().is_none(), "保护模式本来就不写盘，不算保存失败");
        assert!(snapshot(shared, false).save_error.is_none());
        fs::remove_dir(&tmp).unwrap();
    }

    fn queued_blocking_result(initial: core::State, change: impl FnOnce(&mut core::State)) -> String {
        let fixture = TestShared::new(initial);
        let shared = &fixture.0;
        let current = core::render_hosts("127.0.0.1 localhost\n", &["old.example".into()], true);
        let serial = shared.blocking_lock.lock().unwrap();
        thread::scope(|scope| {
            let (started_tx, started_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let worker = scope.spawn(move || {
                started_tx.send(()).unwrap();
                apply_latest_blocking(shared, |hosts, enable| {
                    assert!(shared.blocking_lock.try_lock().is_err(), "应用完成前必须保持串行");
                    assert!(shared.state.try_lock().is_ok(), "系统操作期间不能锁住应用状态");
                    result_tx.send(core::render_hosts(&current, hosts, enable)).unwrap();
                });
            });
            started_rx.recv().unwrap();
            let applied_while_locked = result_rx.recv_timeout(Duration::from_millis(50)).is_ok();
            change(&mut shared.state.lock().unwrap());
            drop(serial);
            worker.join().unwrap();
            assert!(!applied_while_locked, "排队时不能提前应用旧规则");
            result_rx.recv().unwrap()
        })
    }

    #[test]
    fn queued_enable_cannot_restore_blocking_after_end_day() {
        let mut state = state_with_day();
        state.preferences.blocked_hosts = vec!["old.example".into()];
        let result = queued_blocking_result(state, |state| {
            state.end_day(1_000).unwrap();
        });
        assert_eq!(result, "127.0.0.1 localhost\n");
    }

    #[test]
    fn queued_disable_uses_the_restarted_day_and_latest_hosts() {
        let mut state = core::State::new(1_000);
        state.preferences.blocked_hosts = vec!["old.example".into()];
        let result = queued_blocking_result(state, |state| {
            state.preferences.blocked_hosts = vec!["new.example".into()];
            state.start_day("standard", 1_000).unwrap();
        });
        assert!(core::hosts_section_present(&result));
        assert!(result.contains("127.0.0.1 new.example\n"));
        assert!(result.contains("127.0.0.1 www.new.example\n"));
        assert!(!result.contains("old.example"));
    }

    #[test]
    fn clearing_hosts_during_an_apply_still_queues_cleanup() {
        let fixture = TestShared::new(state_with_day());
        let shared = &fixture.0;
        shared.state.lock().unwrap().preferences.blocked_hosts.clear();
        assert!(!blocking_needs_sync(shared));
        shared.blocking.lock().unwrap().busy = true;
        assert!(blocking_needs_sync(shared), "正在写入的旧规则也需要排队清除");
        let current = core::render_hosts("127.0.0.1 localhost\n", &["old.example".into()], true);
        apply_latest_blocking(shared, |hosts, enable| {
            assert_eq!(core::render_hosts(&current, hosts, enable), "127.0.0.1 localhost\n");
        });
    }

    #[test]
    fn rechecking_cancelled_removal_keeps_residual_rules_visible() {
        let current = core::render_hosts("127.0.0.1 localhost\n", &["example.com".into()], true);
        let mut blocking = BlockingStatus {
            error: Some("授权被取消，屏蔽规则未写入。".into()),
            ..BlockingStatus::default()
        };
        refresh_hosts_status(&mut blocking, &current, &[], true);
        assert!(blocking.active, "列表已空也必须保留系统残留状态");
        assert!(blocking.error.as_deref().unwrap().contains("重新应用整站规则"));
        refresh_hosts_status(&mut blocking, "127.0.0.1 localhost\n", &[], true);
        assert!(!blocking.active);
        assert!(blocking.error.is_none(), "系统确认解除后才清空错误");
    }

    #[test]
    fn hosts_status_checks_missing_changed_and_finished_day_rules() {
        let hosts = vec!["example.com".into()];
        let current = core::render_hosts("", &hosts, true);
        let mut blocking = BlockingStatus::default();
        refresh_hosts_status(&mut blocking, "", &hosts, true);
        assert!(!blocking.active);
        assert!(blocking.error.is_some());
        refresh_hosts_status(&mut blocking, &current, &["new.example".into()], true);
        assert!(blocking.active);
        assert!(blocking.error.is_some());
        refresh_hosts_status(&mut blocking, &current, &hosts, false);
        assert!(blocking.error.is_some());
        refresh_hosts_status(&mut blocking, &current, &hosts, true);
        assert!(blocking.active);
        assert!(blocking.error.is_none());
    }

    #[test]
    fn clock_text_only_grows_an_hour_field_when_needed() {
        assert_eq!(clock_text(0), "00:00");
        assert_eq!(clock_text(59), "00:59");
        assert_eq!(clock_text(600), "10:00");
        assert_eq!(clock_text(3_600), "1:00:00");
        assert_eq!(clock_text(-5), "00:00", "负数一律当 0，标题里不能出现负号");
    }

    #[test]
    fn short_name_keeps_the_name_when_it_fits() {
        let state = state_with_day();
        // 内置默认计划四个名字都是 4 个汉字以内，全名直接放得下——
        // 旧规则会把「深度工作」截成「工作」，那是丢信息不是省地方。
        assert_eq!(short_name(&state, "main"), "深度工作");
        assert_eq!(short_name(&state, "reading"), "阅读");
        assert_eq!(short_name(&state, "nope"), "nope", "认不出来就把 id 原样端出去");
    }

    /// 回退规则本身：与 `src/format.ts::shortNameFrom` 必须给出同样的答案。
    #[test]
    fn short_name_fallback_prefers_the_whole_name_then_the_first_word() {
        assert_eq!(short_name_from("学js"), "学js", "宽度 4，放得下");
        assert_eq!(short_name_from("深度工作"), "深度工作", "4 个汉字正好是上限");
        assert_eq!(short_name_from("English Reading"), "English", "太长就取空格前的首个词");
        assert_eq!(short_name_from("深度工作计划"), "深度", "没空格又放不下，取前两个字");
        assert_eq!(short_name_from("Extraordinarily Long"), "Ex", "首个词也放不下，还是前两个字");
        assert_eq!(short_name_from("  写作  "), "写作", "首尾空白不算数");
        assert_eq!(display_width("学js"), 4);
        assert_eq!(display_width("深度工作"), 8);
    }

    /// 学习日进行中改名，托盘仍读当天冻结的名字；下一个学习日才换过来。
    #[test]
    fn short_name_reads_the_frozen_name_while_a_day_is_running() {
        let mut state = state_with_day();
        state.preferences.categories[0].name = "新名字".into();
        assert_eq!(short_name(&state, "main"), "深度工作", "进行中的学习日读冻结名");
        state.day = None;
        assert_eq!(short_name(&state, "main"), "新名字", "没有学习日就读偏好");
    }

    /// 非空短名是用户明确设置，立刻生效，不跟着学习日冻结。
    #[test]
    fn an_explicit_short_name_wins_immediately() {
        let mut state = state_with_day();
        state.preferences.categories[0].short_name = "  线  ".into();
        assert_eq!(short_name(&state, "main"), "线", "去掉首尾空白后直接用");
    }

    #[test]
    fn tray_text_walks_the_whole_day() {
        assert_eq!(tray_text(&core::State::new(1_000)), "坐功");

        let mut state = state_with_day();
        state.tick(1_060);
        assert!(tray_text(&state).starts_with("下一格 "), "还没开格时报该做什么：{}", tray_text(&state));

        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_120);
        assert_eq!(tray_text(&state), "深度工作 24:00", "放得下就用全名，不再截成「工作」");

        state.toggle_pause(1_120).unwrap();
        assert_eq!(tray_text(&state), "深度工作 暂停 00:00", "按停时名字要留着，光一个「暂停」看不出是哪一格");

        state.toggle_pause(1_140).unwrap();
        state.finish_block(1_200).unwrap();
        state.tick(1_260);
        assert_eq!(tray_text(&state), "休息 09:00");

        state.end_break().unwrap();
        assert!(tray_text(&state).starts_with("下一格 "), "休息结束回到该做什么");
    }

    #[test]
    fn tray_pause_preserves_the_project_and_clock_without_expanding_the_title() {
        let mut state = state_with_day();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_060);
        let running = tray_display(&state);
        assert_eq!(running.title, "深度工作 24:00");
        assert!(!running.paused);

        state.toggle_pause(1_060).unwrap();
        state.tick(1_120);
        let paused = tray_display(&state);
        assert_eq!(paused.title, "深度工作 01:00");
        assert_eq!(paused.title.chars().count(), running.title.chars().count());
        assert!(paused.paused);
        assert_eq!(paused.tooltip, "坐功 · 深度工作 暂停 01:00");
        assert!(tray_menu_line(&state, state.day.as_ref().unwrap()).contains("已暂停"));

        state.toggle_pause(1_120).unwrap();
        assert_eq!(tray_display(&state), running, "继续时恢复运行图标、剩余时间和提示");
    }

    #[test]
    fn tray_auto_pause_keeps_a_visible_title_and_a_pause_marker() {
        let mut state = state_with_day();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_000 + core::SUSPEND_GAP_SECONDS + 1);
        let paused = tray_display(&state);
        assert!(paused.paused);
        assert!(paused.title.starts_with("深度工作 "));
        assert!(paused.tooltip.contains("暂停"));
        assert!(!tray_display(&core::State::new(1_000)).paused);
    }

    #[test]
    fn tray_pause_changes_the_display_even_when_clock_digits_match() {
        let mut state = state_with_day();
        state.start_block("main", 5, vec![]).unwrap();
        let running = tray_display(&state);
        state.toggle_pause(1_000).unwrap();
        state.tick(1_300);
        let paused = tray_display(&state);
        assert_eq!(paused.title, running.title);
        assert_ne!(paused, running, "相同文字也要更新暂停角标和悬浮提示，不能被缓存跳过");
    }

    #[test]
    fn tray_clock_stays_compact_across_hours_and_keeps_full_time_in_tooltip() {
        assert_eq!(compact_tray_clock(-1), "00:00");
        assert_eq!(compact_tray_clock(3_599), "59:59");
        assert_eq!(compact_tray_clock(3_600), "1h00");
        assert_eq!(compact_tray_clock(36_000), "10h");
        assert_eq!(compact_tray_clock(i64::MAX), "99h+");
        for seconds in [0, 59, 3_599, 3_600, 35_999, 36_000, 359_999, 360_000, i64::MAX] {
            assert!(compact_tray_clock(seconds).chars().count() <= 5);
        }
        let mut state = state_with_day();
        state.start_block("main", 25, vec![]).unwrap();
        state.toggle_pause(1_000).unwrap();
        state.tick(4_965);
        let paused = tray_display(&state);
        assert_eq!(paused.title, "深度工作 1h06");
        assert_eq!(paused.tooltip, "坐功 · 深度工作 暂停 1:06:05");
    }

    #[test]
    fn tray_pause_badge_preserves_the_icon_canvas_and_original_pixels() {
        for size in [16, 32, 64] {
            let pixels = vec![100; size * size * 4];
            let icon = tauri::image::Image::new(&pixels, size as u32, size as u32);
            let paused = tray_icon(&icon, true);
            assert_eq!((paused.width(), paused.height()), (icon.width(), icon.height()));
            assert_ne!(paused.rgba(), icon.rgba());
            assert_eq!(&paused.rgba()[..size * (size / 2) * 4], &pixels[..size * (size / 2) * 4]);
            assert_eq!(tray_icon(&icon, false).rgba(), pixels, "继续后还原原图，不累积角标");
        }
    }

    #[test]
    fn tray_text_says_done_when_every_quota_is_full() {
        let mut state = state_with_day();
        for c in state.day.as_mut().unwrap().categories.iter_mut() {
            c.accepted_seconds = c.quota_minutes * 60;
        }
        assert_eq!(tray_text(&state), "今日达成");
    }

    #[test]
    fn tray_shape_ignores_the_ticking_numbers() {
        let mut state = state_with_day();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_060);
        let before = tray_shape(&state);
        state.tick(1_120);
        assert_eq!(tray_shape(&state), before, "秒数不能进 shape，否则菜单每秒重建，点开就缩");

        state.toggle_pause(1_120).unwrap();
        assert_ne!(tray_shape(&state), before, "暂停这种真的改了菜单的事必须进 shape");

        state.toggle_pause(1_140).unwrap();
        state.day.as_mut().unwrap().cups = 5;
        assert_eq!(tray_shape(&state), before, "旧杯数不再参与菜单重建");

        assert!(matches!(tray_shape(&core::State::new(1_000)), TrayMenuState::Idle { .. }));
    }

    #[test]
    fn tray_shape_owns_displayed_fields_and_tracks_profile_renames() {
        let mut state = core::State::new(1_000);
        let before = tray_shape(&state);
        let id = default_profile(&state).unwrap().id.clone();
        state.preferences.profiles.iter_mut().find(|p| p.id == id).unwrap().name = "新名字".into();
        let after = tray_shape(&state);
        assert_ne!(before, after, "计划名称变了也必须更新菜单");
        drop(state);
        assert!(matches!(after, TrayMenuState::Idle { profile: Some((_, name)) } if name == "新名字"));
    }

    #[test]
    fn only_a_missing_file_counts_as_a_first_run() {
        assert!(matches!(decide_initial(Err(std::io::ErrorKind::NotFound)), InitialPlan::Fresh));
        assert!(
            matches!(decide_initial(Err(std::io::ErrorKind::PermissionDenied)), InitialPlan::Protected(_)),
            "读不动不等于没有——当成首启会覆盖用户的存档"
        );
        assert!(matches!(decide_initial(Err(std::io::ErrorKind::Other)), InitialPlan::Protected(_)));
        assert!(matches!(decide_initial(Ok("{ 不是 json".into())), InitialPlan::Protected(_)));
        let raw = core::to_json(&state_with_day());
        assert!(matches!(decide_initial(Ok(raw)), InitialPlan::Loaded(_)));
    }

    #[test]
    fn decide_quit_stays_only_when_save_failed() {
        assert!(matches!(decide_quit(None), QuitDecision::Exit), "存下来了就正常退出");
        assert!(
            matches!(decide_quit(Some("写不进去".into())), QuitDecision::Stay(reason) if reason == "写不进去"),
            "存不下来不能一走了之，得把原因端给用户"
        );
    }

    /// Dock 右键「退出」这类路径不经过 `quit_saving()`，全靠 `RunEvent::Exit` 这一下兜底。
    #[test]
    fn exit_hook_saves_what_the_heartbeat_had_not_written_yet() {
        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        save(shared);
        let on_disk = |shared: &Shared| -> i64 {
            core::from_json(&fs::read_to_string(&shared.path).unwrap()).unwrap().last_tick
        };
        let before = on_disk(shared);
        // 内存往前走了，但还没轮到定期保存——这正是从 Dock 退出会丢掉的那一段。
        shared.state.lock().unwrap().last_tick = before + 17;
        assert_eq!(on_disk(shared), before, "定期保存还没跑，磁盘理应还是旧的");
        final_save_on_exit(shared);
        assert!(on_disk(shared) >= before + 17, "退出钩子必须把这一段补上");
        assert!(shared.save_error.lock().unwrap().is_none(), "补存成功不该留下失败原因");
    }

    /// 用户明确点了「不保存退出」，兜底保存不能替他把东西写回去。
    #[test]
    fn exit_hook_keeps_its_hands_off_after_quit_without_saving() {
        assert!(should_save_on_exit(false), "没拒绝过就该补存");
        assert!(!should_save_on_exit(true), "拒绝过就一个字节都不写");

        let fixture = TestShared::new(core::State::new(1_000));
        let shared = &fixture.0;
        save(shared);
        let untouched = fs::read_to_string(&shared.path).unwrap();
        *shared.declined_final_save.lock().unwrap() = true;
        shared.state.lock().unwrap().last_tick = 9_999;
        final_save_on_exit(shared);
        assert_eq!(
            fs::read_to_string(&shared.path).unwrap(),
            untouched,
            "「不保存退出」之后磁盘必须逐字节保持原样"
        );
    }

    #[test]
    fn resolve_data_dir_is_strict() {
        let default = std::env::temp_dir().join("sitzfleisch-default");
        let custom = std::env::temp_dir().join("sitzfleisch-custom");
        assert_eq!(resolve_data_dir(None, default.clone()), Ok(default.clone()), "没设置就用默认目录");
        assert_eq!(
            resolve_data_dir(Some(Ok(custom.display().to_string())), default.clone()),
            Ok(custom),
            "绝对路径原样采用"
        );
        for bad in ["", "   ", "relative/x"] {
            assert!(
                resolve_data_dir(Some(Ok(bad.into())), default.clone()).is_err(),
                "「{bad}」必须报错停止，绝不退回真实目录"
            );
        }
        assert!(resolve_data_dir(Some(Err(std::env::VarError::NotUnicode("x".into()))), default).is_err());
    }

    #[test]
    fn a_fresh_start_gets_the_builtin_plan() {
        let fixture = TestShared::new(core::State::new(1_000));
        let missing = fixture.0.path.parent().unwrap().join("nothing-here.json");
        let (state, protection) = load_initial(&missing);
        assert!(protection.is_none());
        let ids: Vec<&str> = state.preferences.categories.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["main", "reading", "writing", "browse"], "拿到的必须是内置默认计划");
    }

    /// 会把路径塞进命令行的那些恶心字符。`SITZFLEISCH_DATA_DIR` 是用户给的，
    /// 看起来像命令的 hosts 内容：嵌进脚本以后，一个字都不能被 shell 或 AppleScript 当真。
    #[cfg(target_os = "macos")]
    const NASTY_CONTENTS: [&str; 7] = [
        "127.0.0.1 localhost\n",
        "# it's here\n127.0.0.1 a.example\n",
        "# quote\"inside\\ and back\\slash\n",
        "# $(id) `id` ; echo nope && rm -rf /\n",
        "# 新建 文件夹\r\n127.0.0.1 b.example\r\n",
        "",
        "no trailing newline",
    ];

    #[cfg(target_os = "macos")]
    #[test]
    fn base64_matches_the_system_decoder() {
        for (raw, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64_encode(raw.as_bytes()), expected);
        }
        for raw in NASTY_CONTENTS {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("/usr/bin/printf '%s' '{}' | /usr/bin/base64 -D", base64_encode(raw.as_bytes())))
                .output()
                .expect("sh 应该跑得起来");
            assert_eq!(String::from_utf8_lossy(&output.stdout), raw, "系统解码后必须原样还原");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn install_script_embeds_the_content_instead_of_reading_a_user_writable_file() {
        for raw in NASTY_CONTENTS {
            let script = install_script(raw);
            assert!(script.starts_with("do shell script \""));
            assert!(script.contains("with administrator privileges"));
            // 解除屏蔽之后要能立刻恢复解析：SIGHUP 丢不掉已缓存的 hosts 派生记录，
            // 必须把 mDNSResponder 整个收掉让 launchd 重新拉起。别退回 -HUP。
            assert!(script.contains("killall mDNSResponder"), "刷新必须是整个重启");
            assert!(!script.contains("killall -HUP"), "-HUP 不足以丢掉 hosts 派生的缓存");
            assert!(script.contains("|| exit 1"), "写入失败必须让整条脚本失败");
            assert!(
                script.contains("; exit 0\" with administrator privileges"),
                "刷新失败不能影响脚本的退出码：shell 部分必须以 exit 0 收尾"
            );
            assert!(script.contains(&base64_encode(raw.as_bytes())), "内容以 Base64 嵌入");
            if raw.contains(['$', '`', '\'', '"']) {
                assert!(!script.contains(raw.trim_end()), "原文不能直接出现在脚本里");
            }
            // 写入目标固定是 /etc/hosts，脚本里不再出现任何暂存文件路径。
            assert_eq!(script.matches("/etc/hosts").count(), 1);
            assert!(!script.contains("hosts.staged"));
        }
    }

    /// AppleScript 自己解析一遍：`return` 一个字符串字面量，取回来必须与原文一致。
    /// 只求值字符串，**没有 do shell script、不提权**。
    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_escaping_round_trips_through_osascript() {
        for raw in ["plain", r#"has "quotes""#, r"has\backslash", r#"both\"mixed"#] {
            let output = Command::new("/usr/bin/osascript")
                .arg("-e")
                .arg(format!(r#"return "{}""#, applescript_string(raw)))
                .output()
                .expect("osascript 应该跑得起来");
            assert!(output.status.success(), "「{raw}」转义后 AppleScript 解析失败");
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), raw, "「{raw}」转义后必须原样还原");
        }
    }

    /// 免密助手的内容校验是这套方案唯一的防线：放行之后，本机上以该用户身份运行的
    /// 任何程序都能免密调用它。所以「只接受 hosts 记录」必须在 CI 里真的跑一遍。
    #[cfg(target_os = "macos")]
    #[test]
    fn hosts_helper_accepts_real_hosts_files_and_rejects_anything_else() {
        use std::io::Write;
        use std::process::Stdio;

        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/hosts-helper.sh");
        let check = |content: &str| {
            let mut child = Command::new("/bin/sh")
                .arg(&script)
                .arg("--check")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("助手脚本应该跑得起来");
            // 超限时助手会提前停止读取，写端收到 BrokenPipe 是正常的拒绝路径。
            if let Err(error) = child.stdin.take().unwrap().write_all(content.as_bytes()) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
            child.wait().unwrap().success()
        };

        // 真实的 /etc/hosts 长这样：制表符、多空格、IPv6、注释、以及托管段。
        let real = "##\n# Host Database\n##\n127.0.0.1\tlocalhost\n255.255.255.255\tbroadcasthost\n::1             localhost\n\n";
        let managed = core::render_hosts(real, &["live.bilibili.com".into()], true);
        assert!(check(real), "系统原样的 hosts 必须放行");
        assert!(check(&managed), "应用自己渲染出来的 hosts 必须放行");
        assert!(check(&core::render_hosts(&managed, &[], false)), "解除屏蔽后的内容必须放行");

        assert!(!check(""), "空内容要拒绝");
        assert!(!check("127.0.0.1 localhost\nrm -rf /\n"), "夹带的命令行要拒绝");
        assert!(!check("127.0.0.1\n"), "只有地址没有主机名要拒绝");
        assert!(check("127.0.0.1 a"), "末尾没有换行的合法记录要放行");
        assert!(check("127.0.0.1 localhost # 行尾注释\n::ffff:127.0.0.1 local\nfe80::1%lo0 local\n"));
        assert!(!check("999.0.0.1 local\n"), "不能只凭地址字符形状放行");
        assert!(!check("1:2 local\n"));
        assert!(!check("127.0.0.1 local;command\n"));
        assert!(!check("127.0.0.1 a\nrm -rf /"), "末尾没有换行也要逐行校验");
        assert!(!check(&format!("127.0.0.1 a\n{}", "# x\n".repeat(30_000))), "超过大小上限要拒绝");
    }

    #[test]
    fn qa_args_are_parsed_and_bad_values_ignored() {
        let parse = |v: &[&str]| parse_qa_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(parse(&["sitzfleisch"]), (None, 0));
        assert_eq!(
            parse(&["sitzfleisch", "--qa-view", "history", "--qa-scroll", "240"]),
            (Some("history".into()), 240)
        );
        assert_eq!(parse(&["sitzfleisch", "--qa-view", "nope"]), (None, 0), "只认三个页面名");
        assert_eq!(parse(&["sitzfleisch", "--qa-scroll", "abc"]), (None, 0));
        assert_eq!(parse(&["sitzfleisch", "--qa-view"]), (None, 0), "缺参数不能 panic");
    }

}
