use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sitzfleisch_core as core;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

mod browser_blocking;

#[derive(Clone, Default, Serialize)]
struct BlockingStatus {
    active: bool,
    busy: bool,
    error: Option<String>,
    browser: browser_blocking::Status,
}

struct Shared {
    state: Mutex<core::State>,
    snapshot_revision: AtomicU64,
    /// 从取快照到原子替换必须串行，避免旧存档后写或共用暂存文件。
    save_lock: Mutex<()>,
    /// 屏蔽只能一次做一件：连着改列表会起好几个线程，各自读 hosts、各自提权，
    /// 后写的会盖掉先写的，还共用同一个暂存文件。
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
}

#[derive(Debug, PartialEq, Eq)]
enum TrayMenuState {
    Day { line: String, paused: Option<bool>, totals: String, cups: i64 },
    Idle { profile: Option<(String, String)> },
}

/// 托盘菜单主行的文字。菜单和 shape 都从这里取，两边不会分叉。
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

fn default_profile(state: &core::State) -> Option<&core::ProfileDef> {
    state
        .preferences
        .profiles
        .iter()
        .find(|p| p.id == state.preferences.default_profile_id)
        .or_else(|| state.preferences.profiles.first())
}

/// 菜单的「形状」。**里面绝不能放每秒都变的数字**——菜单一旦被重建，正打开的那份就会被
/// 系统收走，表现出来就是「点开秒缩」。倒计时只放在菜单栏标题上，那里可以随便刷。
/// 它必须便宜：每秒都要算一次，用来决定要不要真的去造那一整套原生菜单项。
fn tray_shape(state: &core::State) -> TrayMenuState {
    match &state.day {
        Some(day) => TrayMenuState::Day {
            line: tray_menu_line(state, day),
            paused: day.timer.as_ref().map(|_| day.is_paused()),
            totals: format!("已学 {}", core::duration_text(day.net_seconds())),
            cups: day.cups,
        },
        None => TrayMenuState::Idle {
            profile: default_profile(state).map(|p| (p.id.clone(), p.name.clone())),
        },
    }
}

/// 菜单栏菜单：跟着状态走。只在 `tray_shape` 变化时才需要重建。
fn build_tray_menu(app: &AppHandle, state: &TrayMenuState) -> tauri::Result<Menu<Wry>> {
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<Wry>>> = Vec::new();
    let text = |app: &AppHandle, id: &str, text: &str| MenuItem::with_id(app, id, text, false, None::<&str>);
    let action = |app: &AppHandle, id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);

    items.push(Box::new(MenuItem::with_id(app, "show", "打开坐功", true, Some("CmdOrCtrl+O"))?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));

    match state {
        TrayMenuState::Day { line, paused, totals, cups } => {
            items.push(Box::new(text(app, "info", line)?));
            if let Some(paused) = paused {
                items.push(Box::new(action(app, "toggle", if *paused { "继续" } else { "暂停" })?));
                items.push(Box::new(action(app, "extend", "+10 分钟")?));
                items.push(Box::new(action(app, "finish", "结束这一格")?));
            } else {
                items.push(Box::new(action(app, "show", "去开一格")?));
            }
            items.push(Box::new(PredefinedMenuItem::separator(app)?));
            // 已学时间只到分钟，一天里最多变几百次，不会打断菜单操作。
            items.push(Box::new(text(app, "totals", totals)?));
            items.push(Box::new(action(app, "water", &format!("记一杯水（{cups}）"))?));
        }
        TrayMenuState::Idle { profile } => {
            items.push(Box::new(text(app, "info", "今天还没开始")?));
            if let Some((id, name)) = profile {
                items.push(Box::new(action(app, &format!("start:{id}"), &format!("开始{name}日"))?));
            }
        }
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?));
    let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i.as_ref()).collect();
    Menu::with_items(app, &refs)
}

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

fn handle_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main_window(app),
        "toggle" => {
            let _ = mutate(app, |s| s.toggle_pause(now_unix()));
        }
        "extend" => {
            let _ = mutate(app, |s| s.extend_block(10));
        }
        "finish" => {
            let _ = mutate(app, |s| s.finish_block(now_unix()));
        }
        "water" => {
            let _ = mutate(app, |s| s.drink_water());
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

fn snapshot(shared: &Shared, with_history: bool) -> Snapshot {
    let state = shared.state.lock().unwrap();
    let mut blocking = shared.blocking.lock().unwrap().clone();
    blocking.browser = shared.browser_bridge.lock().unwrap()
        .status(&browser_blocking::Rules::from_state(&state));
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
    let result = fs::write(&tmp, json).and_then(|_| fs::rename(&tmp, &shared.path));
    // 结果发布在 save_lock 之内：两笔排队的保存，后写的那份结果才是最终结论。
    *shared.save_error.lock().unwrap() = result.err().map(|e| e.to_string());
}

/// 短名放得下多宽：4 个汉字，或 8 个拉丁字符。
const SHORT_NAME_WIDTH: usize = 8;

/// 一个字顶两个拉丁字符宽的那类字：汉字、假名、谚文、全角标点。
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

fn display_width(text: &str) -> usize {
    text.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum()
}

/// `short_name` 为空时的自动回退。**与 `src/format.ts::shortNameFrom` 必须一模一样**。
/// 取的是**前**两个字不是后两个：「深度工作」截成「工作」会丢掉是哪一种。
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

fn short_name(state: &core::State, id: &str) -> String {
    // 短名是显示偏好，改完立刻生效，不跟着学习日冻结——和图标一个待遇。
    if let Some(def) = state.preferences.categories.iter().find(|c| c.id == id) {
        if !def.short_name.trim().is_empty() {
            return def.short_name.trim().to_string();
        }
    }
    short_name_from(&display_name(state, id))
}

fn clock_text(seconds: i64) -> String {
    let safe = seconds.max(0);
    if safe >= 3600 {
        format!("{}:{:02}:{:02}", safe / 3600, (safe % 3600) / 60, safe % 60)
    } else {
        format!("{:02}:{:02}", safe / 60, safe % 60)
    }
}

/// 菜单栏标题：深度工作 42:11 / 休息 07:20 / 暂停 12:03 / 下一格 阅读 / 今日达成。
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

fn broadcast(app: &AppHandle) {
    let shared = app.state::<Shared>();
    // 心跳推送不带历史：历史只在命令返回和首次拉取时随快照走一遍。
    let snap = snapshot(&shared, false);
    let _ = app.emit("state://update", &snap);
    // 原生菜单会同步等待主线程；带着应用锁调用会与主线程的命令互相等。
    // 到主线程后再取当前形状，排队的更新也不会把旧菜单装回去。
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let shared = handle.state::<Shared>();
        let (text, shape) = {
            let state = shared.state.lock().unwrap();
            (tray_text(&state), tray_shape(&state))
        };
        let Some(tray) = handle.tray_by_id("main") else { return };
        // macOS 菜单栏直接显示倒计时文字；Windows 托盘没有标题位，挂在悬停提示上。
        #[cfg(target_os = "macos")]
        let _ = tray.set_title(Some(text.clone()));
        let _ = tray.set_tooltip(Some(format!("坐功 · {text}")));
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

fn take_due_reminder_notifications(state: &mut core::State) -> Vec<(&'static str, String)> {
    let (water_due, stretch_due, idle_due) = state.take_due_reminders();
    let mut notifications = Vec::new();
    if water_due {
        notifications.push(("喝点水吧", "忙了一阵，喝几口水再继续。".into()));
    }
    if stretch_due {
        notifications.push((
            "起来活动一下",
            format!("已经连续在座 {} 分钟。", state.preferences.stretch_reminder_minutes),
        ));
    }
    if idle_due {
        // 提醒计数器每次投递后归零，文案仍要从本次暂停的起点累计，包含休息和休眠。
        let paused_minutes = state.day.as_ref()
            .map(|day| day.current_pause_seconds(state.last_tick) / 60)
            .unwrap_or(0);
        notifications.push(("还没开格", format!("已经暂停 {paused_minutes} 分钟了。")));
    }
    notifications
}

/// 系统横幅显不显示由每个 App 的通知设置说了算，App 自己既读不到也改不了。
/// 所以提醒一次走四条路，任何一条都能让人察觉：
/// 系统通知 · 界面提示条 · 一声响 · Dock 图标跳一下。
fn notify(app: &AppHandle, title: &str, body: &str) {
    let delivered = app.notification().builder().title(title).body(body).show().is_ok();
    let _ = app.emit("reminder://show", serde_json::json!({
        "title": title,
        "body": body,
        "system": delivered,
    }));
    let sound_on = app.state::<Shared>().state.lock().unwrap().preferences.sound_enabled;
    if sound_on {
        play_alert_sound();
    }
    // 人多半在别的 App 里，Dock 上跳一下比什么都直接。
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// 响一声。走系统自带的声音，不需要任何权限，窗口关着也听得见。
fn play_alert_sound() {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/afplay")
        .arg("/System/Library/Sounds/Glass.aiff")
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", "[console]::beep(880,180); [console]::beep(1320,180)"])
        .spawn();
}

fn mutate(
    app: &AppHandle,
    op: impl FnOnce(&mut core::State) -> core::RuleResult,
) -> Result<Snapshot, String> {
    let shared = app.state::<Shared>();
    let outcome = {
        let mut state = shared.state.lock().unwrap();
        state.tick(now_unix());
        op(&mut state)
    };
    outcome.map_err(|e| e.to_string())?;
    save(&shared);
    broadcast(app);
    Ok(snapshot(&shared, true))
}

// ---------- 网站屏蔽（hosts；变换在 core，落盘与提权在这里） ----------

fn hosts_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        return PathBuf::from(root).join("System32").join("drivers").join("etc").join("hosts");
    }
    #[cfg(not(target_os = "windows"))]
    PathBuf::from("/etc/hosts")
}

/// 路径要先过这里再进命令行。`SITZFLEISCH_DATA_DIR` 是用户给的，暂存文件的路径里
/// 可以有单引号、`$`、反引号、分号——直接拼进命令就是一条以**管理员权限**执行的注入。
/// 单引号里 shell 不做任何解释，唯一要处理的是单引号自己：闭合、转义一个、再开。
#[cfg(target_os = "macos")]
fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', r"'\''"))
}

/// AppleScript 字符串字面量的转义。**反斜杠必须先换**：反过来的话 `"` → `\"` 里
/// 新加的那个反斜杠会被第二遍再转一次，变成 `\\"`，字符串当场断掉。
#[cfg(target_os = "macos")]
fn applescript_string(raw: &str) -> String {
    raw.replace('\\', r"\\").replace('"', "\\\"")
}

/// 路径 → 完整的 AppleScript。抽出来是为了能测引用与转义而不真的去提权。
#[cfg(target_os = "macos")]
fn install_script(staged: &Path) -> Result<String, String> {
    let path = staged.to_str().ok_or("暂存文件路径不是有效的 UTF-8，无法安全地交给系统授权。")?;
    // `killall mDNSResponder` 而不是 `-HUP`：SIGHUP 是苹果文档里的路子，但它**不保证**丢掉
    // 已经缓存下来的 hosts 派生记录。实测（macOS 26.6）解除屏蔽后 hosts 明明清干净了，
    // 被解除的域名仍然解析到 127.0.0.1 至少两分钟，手动再跑一次 flushcache 也没用，
    // 约 25 分钟后才自己恢复——用户看到的就是「收工了但站还打不开」。
    // 整个进程收掉、由 launchd 重新拉起，新实例带着空缓存重读 hosts，这一下是确定的。
    // 不用 `launchctl kickstart`：它要 launchd 的标签，而那个标签跟版本走
    // （macOS 26 上是 com.apple.mDNSResponder.reloaded，不是通用的 com.apple.mDNSResponder），
    // 按**进程名**杀更耐得住系统升级。
    // 刷新是尽力而为，不能让它把「写成功了」报成失败：`do shell script` 拿最后一条命令的
    // 退出码当整条脚本的结果，而 `killall` 在进程名对不上时返回非零。所以 `cp` 失败才 exit 1，
    // 后面两条各自吞掉错误，最后显式 exit 0。
    let shell = [
        format!("/bin/cp -f {} /etc/hosts || exit 1", shell_single_quote(path)),
        "/usr/bin/dscacheutil -flushcache >/dev/null 2>&1".into(),
        "/usr/bin/killall mDNSResponder >/dev/null 2>&1".into(),
        "exit 0".into(),
    ]
    .join("; ");
    Ok(format!(
        r#"do shell script "{}" with administrator privileges with prompt "坐功需要管理员权限来更新学习日的网站屏蔽规则。""#,
        applescript_string(&shell)
    ))
}

#[cfg(target_os = "macos")]
fn privileged_install(staged: &Path) -> Result<(), String> {
    let script = install_script(staged)?;
    let output = Command::new("osascript")
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
        Err(format!("写入失败：{}", err.trim()))
    }
}

/// PowerShell 单引号字符串里，单引号自己写两遍。外层这一层用它，
/// 里层 cmd 的双引号引用要求路径本身不含 `"`——Windows 文件名本来就不允许，
/// 但那是用户给的 `SITZFLEISCH_DATA_DIR`，还是当场挡住，别指望约定。
#[cfg(target_os = "windows")]
fn powershell_single_quote(raw: &str) -> String {
    raw.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn install_command(staged: &Path) -> Result<String, String> {
    let path = staged.to_str().ok_or("暂存文件路径不是有效的 UTF-8，无法安全地交给 UAC。")?;
    let target = hosts_path();
    let target = target.to_str().ok_or("系统 hosts 路径不是有效的 UTF-8。")?;
    if path.contains('"') || target.contains('"') {
        return Err("路径里不能有双引号，无法安全地交给 UAC。".into());
    }
    // 只有 -ArgumentList 后面那一对单引号里的内容需要转义，外面是 PowerShell 语法本身。
    let arguments = format!("/c copy /y \"{path}\" \"{target}\" & ipconfig /flushdns");
    Ok(format!(
        "Start-Process -FilePath cmd.exe -ArgumentList '{}' -Verb RunAs -Wait",
        powershell_single_quote(&arguments)
    ))
}

#[cfg(target_os = "windows")]
fn privileged_install(staged: &Path) -> Result<(), String> {
    // 未在真机验收：通过 UAC 提权把暂存文件拷成系统 hosts 并刷新 DNS。
    let script = install_command(staged)?;
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| format!("无法调用 UAC 提权：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("提权被取消或失败，屏蔽规则未写入。".into())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn privileged_install(_staged: &Path) -> Result<(), String> {
    Err("此平台暂不支持网站屏蔽。".into())
}

/// 在独立线程里应用/解除屏蔽；结果通过 blocking 状态广播回界面。
fn spawn_apply_blocking(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        let shared = app.state::<Shared>();
        // 暂存文件跟着状态文件走同一个目录：隔离测试时不能落回真实目录。
        let data_dir = shared.data_dir.clone();
        apply_latest_blocking(&shared, |hosts, enable| {
            broadcast(&app);

            let result = (|| -> Result<bool, String> {
                let path = hosts_path();
                let current =
                    fs::read_to_string(&path).map_err(|e| format!("读不到系统 hosts：{e}"))?;
                let desired = core::render_hosts(&current, hosts, enable);
                if desired == current {
                    return Ok(core::hosts_section_present(&current));
                }
                let staged = data_dir.join(format!("hosts.staged.{}", std::process::id()));
                fs::write(&staged, &desired).map_err(|e| format!("暂存文件写入失败：{e}"))?;
                privileged_install(&staged)?;
                let _ = fs::remove_file(&staged);
                let after =
                    fs::read_to_string(&path).map_err(|e| format!("回读系统 hosts 失败：{e}"))?;
                if after != desired {
                    return Err("回读核验不一致，规则可能没有生效。".into());
                }
                Ok(core::hosts_section_present(&after))
            })();

            {
                let mut blocking = shared.blocking.lock().unwrap();
                blocking.busy = false;
                match result {
                    Ok(present) => {
                        blocking.active = present;
                        blocking.error = None;
                    }
                    Err(message) => {
                        blocking.error = Some(message);
                        blocking.active = fs::read_to_string(hosts_path())
                            .map(|c| core::hosts_section_present(&c))
                            .unwrap_or(blocking.active);
                    }
                }
            }
            broadcast(&app);
        });
    });
}

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
        (state.preferences.blocked_hosts.clone(), state.day.is_some())
    };
    apply(&hosts, enable);
}

fn blocking_wanted(shared: &Shared) -> bool {
    let state = shared.state.lock().unwrap();
    state.day.is_some() && !state.preferences.blocked_hosts.is_empty()
}

fn blocking_needs_sync(shared: &Shared) -> bool {
    let active_or_busy = {
        let blocking = shared.blocking.lock().unwrap();
        blocking.active || blocking.busy
    };
    active_or_busy || blocking_wanted(shared)
}

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
    mutate(&app, |s| {
        s.start_block_with_break(&category_id, minutes, tasks.clone(), break_minutes.unwrap_or(0))
    })
}

#[tauri::command]
fn toggle_task(index: usize, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.toggle_task(index))
}

#[tauri::command]
fn add_task(text: String, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.add_task(&text))
}

/// 进行中改档：进度与台账保留，只按新档位重铺配额。
#[tauri::command]
fn switch_profile(profile_id: String, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.switch_profile(&profile_id))
}

#[tauri::command]
fn toggle_pause(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.toggle_pause(now_unix()))
}

#[tauri::command]
fn extend_block(minutes: i64, app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.extend_block(minutes))
}

#[tauri::command]
fn finish_block(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.finish_block(now_unix()))
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
fn drink_water(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.drink_water())
}

#[tauri::command]
fn undo_water(app: AppHandle) -> Result<Snapshot, String> {
    mutate(&app, |s| s.undo_water())
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
    let snap = mutate(&app, |s| s.update_preferences(prefs))?;
    let shared = app.state::<Shared>();
    let day_active = shared.state.lock().unwrap().day.is_some();
    if day_active && blocking_needs_sync(&shared) {
        // 学习日进行中改了屏蔽列表：立即同步系统规则。
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
#[tauri::command]
fn check_blocking(app: AppHandle) -> Snapshot {
    {
        let shared = app.state::<Shared>();
        let (hosts, enable) = {
            let state = shared.state.lock().unwrap();
            (state.preferences.blocked_hosts.clone(), state.day.is_some())
        };
        let mut blocking = shared.blocking.lock().unwrap();
        match fs::read_to_string(hosts_path()) {
            Ok(current) => refresh_hosts_status(&mut blocking, &current, &hosts, enable),
            Err(e) => blocking.error = Some(format!("读不到系统 hosts：{e}")),
        }
    }
    broadcast(&app);
    snapshot(&app.state::<Shared>(), true)
}

/// 通知权限：granted / denied / unknown。
#[tauri::command]
fn notification_status(app: AppHandle) -> String {
    use tauri_plugin_notification::PermissionState;
    match app.notification().permission_state() {
        Ok(PermissionState::Granted) => "granted".into(),
        Ok(PermissionState::Denied) => "denied".into(),
        Ok(_) => "unknown".into(),
        Err(_) => "unknown".into(),
    }
}

#[tauri::command]
fn request_notification_permission(app: AppHandle) -> String {
    use tauri_plugin_notification::PermissionState;
    match app.notification().request_permission() {
        Ok(PermissionState::Granted) => "granted".into(),
        Ok(PermissionState::Denied) => "denied".into(),
        _ => "unknown".into(),
    }
}

/// 打开系统的通知设置页。
/// 设置页的「试一条」：立刻按完整链路发一条，看得见就说明这条路通。
#[tauri::command]
fn test_notification(app: AppHandle) {
    notify(&app, "坐功 · 试一条", "看到这条横幅，说明系统通知这条路是通的。");
}

#[tauri::command]
fn open_notification_settings() -> Result<(), String> {
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
#[tauri::command]
fn reapply_blocking(app: AppHandle) -> Snapshot {
    spawn_apply_blocking(&app);
    snapshot(&app.state::<Shared>(), true)
}

/// 在 Finder / 资源管理器中显示状态文件。
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

#[tauri::command]
fn autostart_status(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(enabled: bool, app: AppHandle) -> Result<bool, String> {
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }
        .map_err(|e| e.to_string())?;
    Ok(launcher.is_enabled().unwrap_or(false))
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
            state.resume_after_restart(now_unix());
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
const WINDOW_STATE: StateFlags = StateFlags::SIZE.union(StateFlags::POSITION);

/// 退出前那次保存的结论。**纯函数**，好测；窗口与进程操作全留在调用方。
enum QuitDecision {
    Exit,
    Stay(String),
}

fn decide_quit(save_error: Option<String>) -> QuitDecision {
    match save_error {
        None => QuitDecision::Exit,
        Some(reason) => QuitDecision::Stay(reason),
    }
}

/// 存不下来就不退：内存里的改动原样留着，把主窗口叫回来问用户。
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
#[tauri::command]
fn quit_after_save(app: AppHandle) -> Result<(), String> {
    let shared = app.state::<Shared>();
    shared.state.lock().unwrap().tick(now_unix());
    save(&shared);
    let error = shared.save_error.lock().unwrap().clone();
    match decide_quit(error) {
        QuitDecision::Exit => {
            let _ = app.save_window_state(WINDOW_STATE);
            app.exit(0);
            Ok(())
        }
        QuitDecision::Stay(reason) => Err(reason),
    }
}

/// 「不保存退出」：磁盘上保持上一次成功写入的完整文件。
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
    shared.state.lock().unwrap().tick(now_unix());
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
    tauri::Builder::default()
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
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .on_menu_event(|app, event| {
            if event.id.as_ref() == "app-quit" {
                quit_saving(app);
            }
        })
        .setup(move |app| {
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

            // 启动只做只读体检：残留的屏蔽段提示到设置页，绝不主动弹授权框。
            let mut blocking = BlockingStatus::default();
            match fs::read_to_string(hosts_path()) {
                Ok(current) => refresh_hosts_status(&mut blocking, &current, &state.preferences.blocked_hosts, state.day.is_some()),
                Err(e) => blocking.error = Some(format!("读不到系统 hosts：{e}")),
            }

            let (qa_view, qa_scroll) = qa_launch_args();
            app.manage(Shared {
                state: Mutex::new(state),
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
            });
            browser_blocking::start(app.handle(), isolated);
            #[cfg(target_os = "macos")]
            install_chinese_menu(app)?;

            let menu = {
                let shared = app.state::<Shared>();
                let shape = tray_shape(&shared.state.lock().unwrap());
                let menu = build_tray_menu(app.handle(), &shape)?;
                *shared.tray_shape.lock().unwrap() = Some(shape);
                menu
            };
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| handle_tray_menu(app, event.id.as_ref()))
                .build(app)?;

            // 每秒走一格；只有「自然走完」的转变才配通知（用户手点的不用提醒自己）。
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut ticks: u64 = 0;
                loop {
                    thread::sleep(Duration::from_secs(1));
                    ticks += 1;
                    let (focus_done, break_done, reminders) = {
                        let shared = handle.state::<Shared>();
                        let mut state = shared.state.lock().unwrap();
                        let had_timer = state.day.as_ref().map(|d| d.timer.is_some()).unwrap_or(false);
                        let ledger_before = state.day.as_ref().map(|d| d.ledger.len()).unwrap_or(0);
                        let now = now_unix();
                        state.tick(now);
                        let reminders = take_due_reminder_notifications(&mut state);
                        let break_done = state.take_due_break(now);
                        let day = state.day.as_ref();
                        // 一格自然走完：台账多了一条，格也不在了。
                        let focus_done = had_timer
                            && day.map(|d| d.ledger.len() > ledger_before && d.timer.is_none()).unwrap_or(false);
                        drop(state);
                        if ticks.is_multiple_of(30) {
                            save(&shared);
                        }
                        (focus_done, break_done, reminders)
                    };
                    if focus_done {
                        notify(&handle, "这一格走完了", "已经记进今天的进度。");
                    }
                    if break_done {
                        notify(&handle, "休息结束", "开下一格吧。");
                    }
                    for (title, body) in reminders {
                        notify(&handle, title, &body);
                    }
                    broadcast(&handle);
                }
            });

            // 通知权限得主动要一次，否则 macOS 根本不会把这个 App 登记进通知中心，
            // 表现出来就是「设置里找不到它，也永远收不到提醒」。放后台线程，别挡住启动。
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
        })
        .on_window_event(|window, event| {
            // 关窗不退出：计时挂在托盘继续走，和 Mac 主程序一个规矩。
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            start_day,
            switch_profile,
            start_block,
            toggle_task,
            add_task,
            toggle_pause,
            extend_block,
            finish_block,
            end_break,
            abandon_block,
            drink_water,
            undo_water,
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
            reapply_blocking,
            check_blocking,
            notification_status,
            request_notification_permission,
            test_notification,
            open_notification_settings,
            reveal_state_file,
            autostart_status,
            set_autostart,
            app_version
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Dock 右键「退出」、AppleScript `quit`、注销都不走 `app-quit` 菜单项，
            // 于是绕过了 `quit_saving()`。这一层拦不住它们（tao 没注册
            // `applicationShouldTerminate:`），但 `applicationWillTerminate:` 会转成
            // `RunEvent::Exit`，还来得及把盘存了。⌘Q 那条路走到这里时已经存过一次，
            // 再存一次无害；用户点过「不保存退出」时则会被 `declined_final_save` 挡住。
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(shared) = app.try_state::<Shared>() {
                    final_save_on_exit(&shared);
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

    struct TestShared(Shared);

    impl TestShared {
        fn new(state: core::State) -> Self {
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
                vec![("还没开格", format!("已经暂停 {expected_minutes} 分钟了。"))],
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
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![("还没开格", "已经暂停 10 分钟了。".into())],
            "新暂停包含刚结束的 5 分钟休息，不累计上一段暂停",
        );
        assert_eq!(state.day.as_ref().unwrap().paused_seconds, 2_400);
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
            vec![("还没开格", "已经暂停 20 分钟了。".into())],
        );

        state = core::from_json(&core::to_json(&state)).unwrap();
        state.resume_after_restart(state.last_tick + 1_800);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "重启不额外触发提醒");
        advance_awake_seconds(&mut state, 600);
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![("还没开格", "已经暂停 60 分钟了。".into())],
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
    fn water_notifications_use_the_new_copy_when_the_existing_timer_is_due() {
        let mut state = state_with_day();
        state.preferences.water_reminder_enabled = true;
        state.preferences.water_reminder_minutes = 1;
        state.preferences.stretch_reminder_enabled = true;
        state.preferences.stretch_reminder_minutes = 1;
        advance_awake_seconds(&mut state, 60);
        assert!(take_due_reminder_notifications(&mut state).is_empty(), "没有格时不催喝水或活动");

        state.start_block("main", 25, vec![]).unwrap();
        advance_awake_seconds(&mut state, 59);
        assert!(take_due_reminder_notifications(&mut state).is_empty());
        advance_awake_seconds(&mut state, 1);
        assert_eq!(
            take_due_reminder_notifications(&mut state),
            vec![
                ("喝点水吧", "忙了一阵，喝几口水再继续。".into()),
                ("起来活动一下", "已经连续在座 1 分钟。".into()),
            ],
        );
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
        shared.state.lock().unwrap().preferences.categories[0].name = "研究".repeat(4_096);
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
        state.drink_water().unwrap();
        assert_ne!(tray_shape(&state), before, "记一杯水会改菜单里的数字");

        assert!(matches!(tray_shape(&core::State::new(1_000)), TrayMenuState::Idle { .. }));
    }

    #[test]
    fn tray_shape_owns_displayed_fields_and_tracks_profile_renames() {
        let mut state = core::State::new(1_000);
        let before = tray_shape(&state);
        let id = default_profile(&state).unwrap().id.clone();
        state.preferences.profiles.iter_mut().find(|p| p.id == id).unwrap().name = "新档位".into();
        let after = tray_shape(&state);
        assert_ne!(before, after, "档位名称变了也必须更新菜单");
        drop(state);
        assert!(matches!(after, TrayMenuState::Idle { profile: Some((_, name)) } if name == "新档位"));
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
    /// 这些全都是合法的 macOS 目录名。
    #[cfg(target_os = "macos")]
    const NASTY_PATHS: [&str; 9] = [
        "/tmp/plain/hosts.staged.1",
        "/tmp/with space/hosts.staged.1",
        "/tmp/it's here/hosts.staged.1",
        r#"/tmp/quote"inside/hosts.staged.1"#,
        r"/tmp/back\slash/hosts.staged.1",
        "/tmp/$(id)/hosts.staged.1",
        "/tmp/`id`/hosts.staged.1",
        "/tmp/semi;colon && echo nope/hosts.staged.1",
        "/tmp/新建 文件夹/hosts.staged.1",
    ];

    /// 真的交给 `/bin/sh` 解析一遍：引用对了，`printf %s` 就该原样吐回来。
    /// 用的是 printf 不是 cp，**不提权、不碰 /etc/hosts**；`$(...)`、反引号在单引号里是字面量，
    /// 万一引用漏了，下面的相等断言会先失败。
    #[cfg(target_os = "macos")]
    #[test]
    fn shell_quoting_survives_paths_that_look_like_commands() {
        // 绊线走自己的唯一路径：写死一个 /tmp/xxx 的话，别的进程碰巧建了它，
        // 这条测试就永远是红的，而且它自己还不清理。
        let tripwire = std::env::temp_dir().join(format!(
            "sitzfleisch-quote-tripwire-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let _ = fs::remove_file(&tripwire);
        let armed = format!("/tmp/$(touch {})/hosts.staged.1", tripwire.display());
        for raw in NASTY_PATHS.iter().copied().chain(std::iter::once(armed.as_str())) {
            let quoted = shell_single_quote(raw);
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("printf %s {quoted}"))
                .output()
                .expect("sh 应该跑得起来");
            assert!(output.status.success(), "「{raw}」引用后 sh 解析失败");
            assert_eq!(String::from_utf8_lossy(&output.stdout), raw, "「{raw}」引用后必须原样还原");
        }
        assert!(!tripwire.exists(), "注入片段绝不能真的被执行");
        let _ = fs::remove_file(&tripwire);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_escaping_does_backslash_before_quote() {
        assert_eq!(applescript_string(r#"a\b"c"#), r#"a\\b\"c"#);
        // 顺序反过来的话，`"` → `\"` 里新加的反斜杠会被第二遍再转一次，字符串当场断掉。
        let wrong = r#"a\b"c"#.replace('"', "\\\"").replace('\\', r"\\");
        assert_ne!(wrong, applescript_string(r#"a\b"c"#), "先转引号再转反斜杠是错的");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn install_script_quotes_the_staged_path() {
        for raw in NASTY_PATHS {
            let script = install_script(Path::new(raw)).expect("UTF-8 路径应该能生成脚本");
            assert!(script.starts_with("do shell script \""));
            assert!(script.contains("with administrator privileges"));
            // 解除屏蔽之后要能立刻恢复解析：SIGHUP 丢不掉已缓存的 hosts 派生记录，
            // 必须把 mDNSResponder 整个收掉让 launchd 重新拉起。别退回 -HUP。
            assert!(script.contains("killall mDNSResponder"), "刷新必须是整个重启");
            assert!(!script.contains("killall -HUP"), "-HUP 不足以丢掉 hosts 派生的缓存");
            // 刷新失败不能把「写成功了」报成失败：`do shell script` 拿最后一条命令的退出码
            // 当结果，而 killall 在进程名对不上时返回非零。cp 失败才 exit 1，末尾显式 exit 0。
            assert!(script.contains("|| exit 1"), "cp 失败必须让整条脚本失败");
            assert!(
                script.contains("; exit 0\" with administrator privileges"),
                "刷新失败不能影响脚本的退出码：shell 部分必须以 exit 0 收尾"
            );
            // 脚本里绝不能出现「裸着的」路径：它必须以引用后的形态出现。
            let expected = applescript_string(&shell_single_quote(raw));
            assert!(script.contains(&expected), "「{raw}」没有按引用后的形态进脚本");
            // 复制目标固定是 /etc/hosts，路径不该有本事把它换成别的。
            assert_eq!(script.matches("/etc/hosts").count(), 1, "「{raw}」改变了复制目标的数量");
        }
    }

    /// AppleScript 自己解析一遍：`return` 一个字符串字面量，取回来必须与原文一致。
    /// 只求值字符串，**没有 do shell script、不提权**。
    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_escaping_round_trips_through_osascript() {
        for raw in ["plain", r#"has "quotes""#, r"has\backslash", r#"both\"mixed"#] {
            let output = Command::new("osascript")
                .arg("-e")
                .arg(format!(r#"return "{}""#, applescript_string(raw)))
                .output()
                .expect("osascript 应该跑得起来");
            assert!(output.status.success(), "「{raw}」转义后 AppleScript 解析失败");
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), raw, "「{raw}」转义后必须原样还原");
        }
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
