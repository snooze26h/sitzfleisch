//! 坐功的规则核心，纯逻辑、无平台依赖：
//! - 一天从「开始今天」那一刻起算，永不跨午夜重置；
//! - 心跳间隔超过 120 秒判为挂起，那段空档一秒都不记；进程不在的空档不吃宽限；
//! - 每个专注格结束必须验收：采纳才计入配额进度，拒收只留在台账上；
//! - 项目与档位是用户数据（preferences），学习日开始时把配额拷走冻结。

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 3;
pub const SUSPEND_GAP_SECONDS: i64 = 120;
pub const MIN_BLOCK_MINUTES: i64 = 5;
pub const MAX_BLOCK_MINUTES: i64 = 180;
pub const HISTORY_LIMIT: usize = 60;

// ---------- 计划（用户可编辑） ----------

fn default_icon() -> String {
    "book-open".into()
}

fn default_hydration_goal() -> i64 {
    8
}

fn default_role() -> String {
    "general".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryDef {
    pub id: String,
    pub name: String,
    /// 两个字的短名（侧栏、运行图、菜单栏用）；为空时界面按全名推导。
    #[serde(default)]
    pub short_name: String,
    /// 选中这个项目时预填的块时长。
    pub default_block_minutes: i64,
    /// Lucide 图标名。
    #[serde(default = "default_icon")]
    pub icon: String,
    /// 调度性质：deepWork / exploration / dailyFloor / general / movement。
    #[serde(default = "default_role")]
    pub role: String,
    /// 这个块时长的理由，一句话。
    #[serde(default)]
    pub block_rationale: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileQuota {
    pub category: String,
    pub minutes: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileDef {
    pub id: String,
    pub name: String,
    /// 一句话说明这一档是给什么样的日子用的。
    #[serde(default)]
    pub subtitle: String,
    pub quotas: Vec<ProfileQuota>,
}

fn default_water_interval() -> i64 {
    45
}

fn default_stretch_interval() -> i64 {
    50
}

fn default_idle_interval() -> i64 {
    15
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub categories: Vec<CategoryDef>,
    pub profiles: Vec<ProfileDef>,
    pub break_minutes: i64,
    /// 一天的喝水目标（杯）。
    #[serde(default = "default_hydration_goal")]
    pub hydration_goal_cups: i64,
    /// 在座满 N 分钟没喝水就提醒（学习日内）。
    #[serde(default)]
    pub water_reminder_enabled: bool,
    #[serde(default = "default_water_interval")]
    pub water_reminder_minutes: i64,
    /// 连续在座满 N 分钟没起身就提醒。
    #[serde(default)]
    pub stretch_reminder_enabled: bool,
    #[serde(default = "default_stretch_interval")]
    pub stretch_reminder_minutes: i64,
    /// 在座满 N 分钟却没有在走的计时格就提醒。
    #[serde(default)]
    pub idle_reminder_enabled: bool,
    #[serde(default = "default_idle_interval")]
    pub idle_reminder_minutes: i64,
    /// 学习日期间按精确域名屏蔽的网站。
    #[serde(default)]
    pub blocked_hosts: Vec<String>,
    /// 统一块长（分钟）；0 表示按项目各自的默认块长。
    #[serde(default)]
    pub uniform_block_minutes: i64,
    /// 开始页与菜单栏默认选中的档位；空串表示第一个档位。
    #[serde(default)]
    pub default_profile_id: String,
    /// 计时结束时响一声；生活提醒始终静音。
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        builtin_preferences()
    }
}

fn def(id: &str, name: &str, block: i64, icon: &str, role: &str) -> CategoryDef {
    CategoryDef {
        id: id.into(),
        name: name.into(),
        short_name: String::new(),
        default_block_minutes: block,
        icon: icon.into(),
        role: role.into(),
        block_rationale: String::new(),
    }
}

fn quota(category: &str, minutes: i64) -> ProfileQuota {
    ProfileQuota { category: category.into(), minutes }
}

/// 全新用户的出厂计划；一经编辑就以用户自己的设置为准。
pub fn builtin_preferences() -> Preferences {
    Preferences {
        categories: vec![
            def("main", "深度工作", 50, "brain", "deepWork"),
            def("reading", "阅读", 45, "book-open", "dailyFloor"),
            def("writing", "写作", 50, "pen-line", "general"),
            def("browse", "浏览", 30, "newspaper", "general"),
        ],
        profiles: vec![
            ProfileDef {
                id: "light".into(),
                name: "轻量".into(),
                subtitle: "留给状态不好的日子".into(),
                quotas: vec![quota("main", 120), quota("reading", 60), quota("writing", 60), quota("browse", 0)],
            },
            ProfileDef {
                id: "standard".into(),
                name: "标准".into(),
                subtitle: "大部分日子的默认目标".into(),
                quotas: vec![quota("main", 240), quota("reading", 120), quota("writing", 120), quota("browse", 30)],
            },
            ProfileDef {
                id: "sprint".into(),
                name: "冲刺".into(),
                subtitle: "状态好时的满配".into(),
                quotas: vec![quota("main", 360), quota("reading", 240), quota("writing", 240), quota("browse", 30)],
            },
        ],
        break_minutes: 10,
        hydration_goal_cups: 8,
        water_reminder_enabled: false,
        water_reminder_minutes: 45,
        stretch_reminder_enabled: false,
        stretch_reminder_minutes: 50,
        idle_reminder_enabled: false,
        idle_reminder_minutes: 15,
        blocked_hosts: Vec::new(),
        uniform_block_minutes: 0,
        default_profile_id: String::new(),
        sound_enabled: true,
    }
}

/// 精确域名的唯一权威归一：小写、剥协议 / 路径 / 查询串 / 端口 / `www.`，至少一个点，不能是 IP。
/// 界面里的 `normalizeHost` 只是同一套规则的即时预览，任何东西入库前都要过这里。
pub fn validate_host(raw: &str) -> Result<String, &'static str> {
    let mut host = raw.trim().to_lowercase();
    if host.contains('@') {
        return Err("网址不能包含用户名或密码");
    }
    if let Some(rest) = host.strip_prefix("https://") {
        host = rest.to_string();
    } else if let Some(rest) = host.strip_prefix("http://") {
        host = rest.to_string();
    } else if host.contains("://") {
        return Err("只支持 http 或 https 网址");
    }
    // 只留主机名。
    host = host.split(['/', '?', '#']).next().unwrap_or("").to_string();
    if let Some((name, port)) = host.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            host = name.to_string();
        }
    }
    // 存储形态一律不带 www：写 hosts 时会自己补上 www 那一份。
    if let Some(rest) = host.strip_prefix("www.") {
        host = rest.to_string();
    }
    if host.is_empty() {
        return Err("域名不能为空");
    }
    if host.contains(' ') || host.contains(':') {
        return Err("只要域名本身，不要带路径或端口");
    }
    if !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return Err("这不像一个域名");
    }
    if !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("域名只能包含字母、数字、点和连字符");
    }
    if host.split('.').all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_digit())) {
        return Err("不能添加 IP 地址");
    }
    Ok(host)
}

/// 计划编辑的准入检查：空名、空档位、幽灵引用、离谱数值一律拒绝。
pub fn validate_preferences(prefs: &Preferences) -> RuleResult {
    if prefs.categories.is_empty() {
        return Err("至少要有一个项目");
    }
    if prefs.profiles.is_empty() {
        return Err("至少要有一个档位");
    }
    if !(0..=120).contains(&prefs.break_minutes) {
        return Err("休息时长要在 0–120 分钟之间");
    }
    if prefs.uniform_block_minutes != 0
        && !(MIN_BLOCK_MINUTES..=MAX_BLOCK_MINUTES).contains(&prefs.uniform_block_minutes)
    {
        return Err("统一块长要在 5–180 分钟之间");
    }
    if !prefs.default_profile_id.is_empty()
        && !prefs.profiles.iter().any(|p| p.id == prefs.default_profile_id)
    {
        return Err("默认档位不存在");
    }
    if !(1..=24).contains(&prefs.hydration_goal_cups) {
        return Err("喝水目标要在 1–24 杯之间");
    }
    if !(5..=240).contains(&prefs.water_reminder_minutes)
        || !(5..=240).contains(&prefs.stretch_reminder_minutes)
        || !(5..=240).contains(&prefs.idle_reminder_minutes)
    {
        return Err("提醒间隔要在 5–240 分钟之间");
    }
    for host in &prefs.blocked_hosts {
        if validate_host(host).as_deref() != Ok(host.as_str()) {
            return Err("屏蔽列表里有不合法的域名");
        }
    }
    let mut ids = Vec::new();
    for category in &prefs.categories {
        if category.id.trim().is_empty() || category.name.trim().is_empty() {
            return Err("项目名不能为空");
        }
        if !(MIN_BLOCK_MINUTES..=MAX_BLOCK_MINUTES).contains(&category.default_block_minutes) {
            return Err("默认块时长要在 5–180 分钟之间");
        }
        if ids.contains(&category.id) {
            return Err("项目 id 重复");
        }
        ids.push(category.id.clone());
    }
    for profile in &prefs.profiles {
        if profile.name.trim().is_empty() {
            return Err("档位名不能为空");
        }
        let mut active = 0;
        for q in &profile.quotas {
            if !ids.contains(&q.category) {
                return Err("档位引用了不存在的项目");
            }
            if !(0..=24 * 60).contains(&q.minutes) {
                return Err("配额要在 0–24 小时之间");
            }
            if q.minutes > 0 {
                active += 1;
            }
        }
        if active == 0 {
            return Err("每个档位至少要给一个项目配时");
        }
    }
    Ok(())
}

// ---------- 学习日状态 ----------

/// 一格里可选的小任务，勾掉表示做完了。留空就是这一格不列任务。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockTimer {
    pub category: String,
    pub total_seconds: i64,
    pub elapsed_seconds: i64,
    /// 这一格打算做的事，可以为空。
    #[serde(default)]
    pub tasks: Vec<TaskItem>,
    /// 开格的墙钟时刻（运行图与走时条用）。
    #[serde(default)]
    pub started_at: i64,
    /// 这一格之后的休息分钟数；0 表示沿用偏好里的休息时长。
    #[serde(default)]
    pub break_minutes: i64,
}

impl BlockTimer {
    pub fn remaining_seconds(&self) -> i64 {
        (self.total_seconds - self.elapsed_seconds).max(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub category: String,
    pub seconds: i64,
    /// 计入今天的进度；主动放弃的格为 false，时间只留在台账上。
    pub accepted: bool,
    #[serde(default)]
    pub tasks: Vec<TaskItem>,
    /// 这一格开始的墙钟时刻。
    #[serde(default)]
    pub started_at: i64,
    pub ended_at: i64,
}

impl LedgerEntry {
    pub fn done_tasks(&self) -> usize {
        self.tasks.iter().filter(|t| t.done).count()
    }
}

/// 一段暂停：手动按的，或休眠、锁屏自动判定的。运行图靠它把时间线连起来。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PauseSpan {
    pub started_at: i64,
    /// 还没继续时为 None。
    #[serde(default)]
    pub ended_at: Option<i64>,
    /// true 表示是休眠/锁屏自动按停的，不是人点的。
    #[serde(default)]
    pub auto: bool,
}

impl PauseSpan {
    pub fn seconds(&self, now: i64) -> i64 {
        (self.ended_at.unwrap_or(now) - self.started_at).max(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryState {
    pub id: String,
    pub name: String,
    pub quota_minutes: i64,
    pub accepted_seconds: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Day {
    pub profile_name: String,
    /// 开始今天时选的档位 id；进行中改档会更新。
    #[serde(default)]
    pub profile_id: String,
    pub started_at: i64,
    pub categories: Vec<CategoryState>,
    pub timer: Option<BlockTimer>,
    pub ledger: Vec<LedgerEntry>,
    /// 在座：从坐下到现在，减去所有暂停。
    pub seated_seconds: i64,
    #[serde(default)]
    pub paused_seconds: i64,
    pub suspend_seconds: i64,
    pub cups: i64,
    /// 提醒计数器：在座且没喝水/没起身的累计秒数。
    #[serde(default)]
    pub seated_since_water: i64,
    #[serde(default)]
    pub seated_since_relief: i64,
    /// 没有格在走时，已经暂停了多久（闲置提醒看它）。
    #[serde(default)]
    pub paused_without_block: i64,
    /// 休息到几点。它只是暂停上的一个标签：休息期间一样什么都不走。
    #[serde(default)]
    pub break_until: Option<i64>,
    /// 今天每一段暂停的起止；最后一段 ended_at 为 None 表示现在正暂停着。
    #[serde(default)]
    pub pauses: Vec<PauseSpan>,
}

impl Day {
    /// 学习日开始时把档位配额拷走冻结；配 0 分钟的项目不进今天。
    fn from_profile(prefs: &Preferences, profile: &ProfileDef, now: i64) -> Self {
        let categories = profile
            .quotas
            .iter()
            .filter(|q| q.minutes > 0)
            .filter_map(|q| {
                prefs.categories.iter().find(|c| c.id == q.category).map(|c| CategoryState {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    quota_minutes: q.minutes,
                    accepted_seconds: 0,
                })
            })
            .collect();
        Day {
            profile_name: profile.name.clone(),
            profile_id: profile.id.clone(),
            started_at: now,
            categories,
            timer: None,
            ledger: Vec::new(),
            seated_seconds: 0,
            paused_seconds: 0,
            suspend_seconds: 0,
            cups: 0,
            seated_since_water: 0,
            seated_since_relief: 0,
            paused_without_block: 0,
            break_until: None,
            pauses: Vec::new(),
        }
    }

    pub fn net_seconds(&self) -> i64 {
        self.categories.iter().map(|c| c.accepted_seconds).sum()
    }

    pub fn quota_seconds(&self) -> i64 {
        self.categories.iter().map(|c| c.quota_minutes * 60).sum()
    }

    /// 一天的每一秒不是在某个格里，就是在暂停里。没有格在走时一定是暂停。
    pub fn is_paused(&self) -> bool {
        self.pauses.last().is_some_and(|p| p.ended_at.is_none())
    }

    /// 正在休息（休息只是一段带截止时刻的暂停）。
    pub fn resting(&self, now: i64) -> bool {
        self.break_until.is_some_and(|until| now < until)
    }

    pub fn break_remaining(&self, now: i64) -> i64 {
        self.break_until.map(|until| (until - now).max(0)).unwrap_or(0)
    }

    /// 正在暂停的话，这一段已经暂停了多久。
    pub fn current_pause_seconds(&self, now: i64) -> i64 {
        match self.pauses.last() {
            Some(p) if p.ended_at.is_none() => p.seconds(now),
            _ => 0,
        }
    }

    fn begin_pause(&mut self, now: i64, auto: bool) {
        if !self.is_paused() {
            self.pauses.push(PauseSpan { started_at: now, ended_at: None, auto });
        }
    }

    fn close_pause(&mut self, now: i64) {
        if let Some(open) = self.pauses.last_mut() {
            if open.ended_at.is_none() {
                open.ended_at = Some(now.max(open.started_at));
            }
        }
    }

    /// 把一格收进台账。accepted=false 表示主动放弃，时间不计进度。
    fn record(&mut self, timer: BlockTimer, accepted: bool, now: i64) {
        let seconds = timer.elapsed_seconds.max(0);
        if seconds == 0 {
            return;
        }
        let started_at = if timer.started_at > 0 { timer.started_at } else { now - seconds };
        if accepted {
            if let Some(category) = self.categories.iter_mut().find(|c| c.id == timer.category) {
                category.accepted_seconds += seconds;
            }
        }
        self.ledger.push(LedgerEntry {
            category: timer.category,
            seconds,
            accepted,
            tasks: timer.tasks,
            started_at,
            ended_at: now,
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivedDay {
    pub day: Day,
    pub ended_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    pub last_tick: i64,
    #[serde(default)]
    pub preferences: Preferences,
    pub day: Option<Day>,
    pub history: Vec<ArchivedDay>,
}

pub type RuleResult = Result<(), &'static str>;

impl State {
    pub fn new(now: i64) -> Self {
        State {
            schema: SCHEMA_VERSION,
            last_tick: now,
            preferences: builtin_preferences(),
            day: None,
            history: Vec::new(),
        }
    }

    // ---------- 心跳 ----------

    /// 重启后的第一件事：进程不在的空档不论长短，一律按挂起结算，一秒不补，并自动暂停。
    /// 暂停段必须从**空档开始的那一刻**起算，否则这段时间不属于任何暂停，
    /// 运行图会把它当成「一直在做事」画成实心条。
    pub fn resume_after_restart(&mut self, now: i64) {
        let gap = now - self.last_tick;
        let gap_started_at = self.last_tick;
        self.last_tick = now;
        let Some(day) = &mut self.day else { return };
        if gap <= 0 {
            return;
        }
        day.suspend_seconds += gap;
        day.paused_seconds += gap;
        day.begin_pause(gap_started_at, true);
    }

    /// 心跳后由外壳调用：哪个提醒到点了就返回 true 并把计数器归零。
    /// 只在学习日内、没暂停时生效。
    pub fn take_due_reminders(&mut self) -> (bool, bool, bool) {
        let water_interval = self.preferences.water_reminder_minutes * 60;
        let stretch_interval = self.preferences.stretch_reminder_minutes * 60;
        let idle_interval = self.preferences.idle_reminder_minutes * 60;
        let water_on = self.preferences.water_reminder_enabled;
        let stretch_on = self.preferences.stretch_reminder_enabled;
        let idle_on = self.preferences.idle_reminder_enabled;
        let Some(day) = &mut self.day else { return (false, false, false) };
        if day.is_paused() {
            // 暂停时只剩闲置提醒：暂停这么久还没开下一格，该催一句了。
            let idle_due = idle_on && day.timer.is_none() && day.paused_without_block >= idle_interval;
            if idle_due {
                day.paused_without_block = 0;
            }
            return (false, false, idle_due);
        }
        let mut water_due = false;
        let mut stretch_due = false;
        if water_on && day.seated_since_water >= water_interval {
            day.seated_since_water = 0;
            water_due = true;
        }
        if stretch_on && day.seated_since_relief >= stretch_interval {
            day.seated_since_relief = 0;
            stretch_due = true;
        }
        (water_due, stretch_due, false)
    }

    /// 休息到点了没？到了就返回一次 true 并把标签摘掉。
    pub fn take_due_break(&mut self, now: i64) -> bool {
        let Some(day) = &mut self.day else { return false };
        match day.break_until {
            Some(until) if now >= until => {
                day.break_until = None;
                true
            }
            _ => false,
        }
    }

    /// 一切时间只经由心跳计入。暂停时什么都不走；
    /// 间隔超过 120 秒（休眠、锁屏）判为挂起：只记挂起账，并自动暂停。
    pub fn tick(&mut self, now: i64) {
        let delta = now - self.last_tick;
        self.last_tick = now;
        if delta <= 0 {
            return;
        }
        let break_seconds = self.preferences.break_minutes * 60;
        let Some(day) = &mut self.day else { return };

        if delta > SUSPEND_GAP_SECONDS {
            // 休眠 / 锁屏：一秒都不补，但这段空档要落在暂停段里，从空档开始那一刻算起。
            day.suspend_seconds += delta;
            day.paused_seconds += delta;
            day.begin_pause(now - delta, true);
            return;
        }

        if day.is_paused() {
            day.paused_seconds += delta;
            if day.timer.is_none() {
                day.paused_without_block += delta;
            }
            return;
        }

        if day.timer.is_none() {
            // 没有格却没在暂停：把不变量补回来，这一段时间算暂停。
            day.begin_pause(now - delta, false);
            day.paused_seconds += delta;
            day.paused_without_block += delta;
            return;
        }

        // 一次心跳可能跨过格的终点（系统卡顿时 delta 会大于 1 秒）：
        // 只有落在格里的那部分算在座，越过终点的余量算暂停。
        let (worked, overflow, done) = {
            let timer = day.timer.as_mut().expect("guarded above");
            let room = (timer.total_seconds - timer.elapsed_seconds).max(0);
            let worked = delta.min(room);
            timer.elapsed_seconds += worked;
            (worked, delta - worked, timer.elapsed_seconds >= timer.total_seconds)
        };
        day.seated_seconds += worked;
        day.seated_since_water += worked;
        day.seated_since_relief += worked;
        if !done {
            return;
        }

        let ended_at = now - overflow;
        let finished = day.timer.take().expect("guarded above");
        let rest = if finished.break_minutes > 0 { finished.break_minutes * 60 } else { break_seconds };
        // 走完直接计入，然后立刻进入暂停：不做任务的时间也要有个去处。
        day.record(finished, true, ended_at);
        day.begin_pause(ended_at, false);
        day.paused_seconds += overflow;
        day.paused_without_block = overflow;
        day.break_until = if rest > 0 { Some(ended_at + rest) } else { None };
    }

    // ---------- 计划 ----------

    pub fn update_preferences(&mut self, prefs: Preferences) -> RuleResult {
        validate_preferences(&prefs)?;
        self.preferences = prefs;
        Ok(())
    }

    // ---------- 学习日 ----------

    pub fn start_day(&mut self, profile_id: &str, now: i64) -> RuleResult {
        if self.day.is_some() {
            return Err("今天已经开始了");
        }
        let Some(profile) = self.preferences.profiles.iter().find(|p| p.id == profile_id) else {
            return Err("没有这个档位");
        };
        let mut day = Day::from_profile(&self.preferences, profile, now);
        if day.categories.is_empty() {
            return Err("这个档位没有任何配了时的项目");
        }
        // 还没开格，所以从暂停起步：这段挑项目的时间也要有个去处。
        day.begin_pause(now, false);
        self.day = Some(day);
        self.last_tick = now;
        Ok(())
    }

    /// 进行中改档：按新档位的目标重铺今天的配额。已记录的进度、台账与项目元数据
    /// 原样保留；新档位没配时但今天已经记了账的项目留在册上，目标归零。
    pub fn switch_profile(&mut self, profile_id: &str) -> RuleResult {
        let Some(profile) = self.preferences.profiles.iter().find(|p| p.id == profile_id) else {
            return Err("没有这个档位");
        };
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        let mut categories: Vec<CategoryState> = Vec::new();
        for q in profile.quotas.iter().filter(|q| q.minutes > 0) {
            if let Some(existing) = day.categories.iter().find(|c| c.id == q.category) {
                categories.push(CategoryState { quota_minutes: q.minutes, ..existing.clone() });
            } else if let Some(def) = self.preferences.categories.iter().find(|c| c.id == q.category) {
                categories.push(CategoryState {
                    id: def.id.clone(),
                    name: def.name.clone(),
                    quota_minutes: q.minutes,
                    accepted_seconds: 0,
                });
            }
        }
        for existing in &day.categories {
            let already = categories.iter().any(|c| c.id == existing.id);
            let touched = existing.accepted_seconds > 0
                || day.ledger.iter().any(|l| l.category == existing.id)
                || day.timer.as_ref().is_some_and(|t| t.category == existing.id);
            if !already && touched {
                categories.push(CategoryState { quota_minutes: 0, ..existing.clone() });
            }
        }
        if categories.is_empty() {
            return Err("这个档位没有任何配了时的项目");
        }
        let order = &self.preferences.categories;
        categories.sort_by_key(|c| order.iter().position(|d| d.id == c.id).unwrap_or(usize::MAX));
        day.categories = categories;
        day.profile_name = profile.name.clone();
        day.profile_id = profile.id.clone();
        Ok(())
    }

    /// 误点开始：直接放弃进行中的学习日，不归档。
    pub fn abandon_day(&mut self) -> RuleResult {
        if self.day.is_none() {
            return Err("今天还没开始");
        }
        self.day = None;
        Ok(())
    }

    /// 收工归档。还在走的专注格按已走时长计入，休息格直接丢弃。
    pub fn end_day(&mut self, now: i64) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        if let Some(timer) = day.timer.take() {
            day.record(timer, true, now);
        }
        day.break_until = None;
        day.close_pause(now);
        let day = self.day.take().expect("guarded above");
        self.history.push(ArchivedDay { day, ended_at: now });
        if self.history.len() > HISTORY_LIMIT {
            let excess = self.history.len() - HISTORY_LIMIT;
            self.history.drain(0..excess);
        }
        Ok(())
    }

    /// 永久删除一天归档（按 started_at 定位；调用方负责二次确认）。
    pub fn delete_history_day(&mut self, started_at: i64) -> RuleResult {
        let before = self.history.len();
        self.history.retain(|d| d.day.started_at != started_at);
        if self.history.len() == before {
            return Err("没有这一天的归档");
        }
        Ok(())
    }

    // ---------- 专注格 ----------

    pub fn start_block(&mut self, category_id: &str, minutes: i64, tasks: Vec<TaskItem>) -> RuleResult {
        self.start_block_with_break(category_id, minutes, tasks, 0)
    }

    /// 开格并指定这一格之后的休息分钟数（0 = 沿用偏好）。正暂停着就顺手继续。
    pub fn start_block_with_break(
        &mut self,
        category_id: &str,
        minutes: i64,
        tasks: Vec<TaskItem>,
        break_minutes: i64,
    ) -> RuleResult {
        let now = self.last_tick;
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        if day.timer.is_some() {
            return Err("已经有一格在走");
        }
        if !day.categories.iter().any(|c| c.id == category_id) {
            return Err("今天没有这个项目");
        }
        // 开格等于结束休息，也等于结束暂停。
        day.break_until = None;
        day.close_pause(now);
        let minutes = minutes.clamp(MIN_BLOCK_MINUTES, MAX_BLOCK_MINUTES);
        let tasks: Vec<TaskItem> = tasks
            .into_iter()
            .filter_map(|t| {
                let text = t.text.trim().to_string();
                if text.is_empty() { None } else { Some(TaskItem { text, done: t.done }) }
            })
            .collect();
        day.timer = Some(BlockTimer {
            category: category_id.into(),
            total_seconds: minutes * 60,
            elapsed_seconds: 0,
            tasks,
            started_at: now,
            break_minutes: break_minutes.clamp(0, 120),
        });
        day.paused_without_block = 0;
        Ok(())
    }

    /// 暂停 / 继续。只有格在走时才有意义——没有格在走本来就是暂停状态。
    pub fn toggle_pause(&mut self, now: i64) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        if day.timer.is_none() {
            return Err("没有在走的格");
        }
        if day.is_paused() {
            day.close_pause(now);
            day.break_until = None;
            day.seated_since_relief = 0;
        } else {
            day.begin_pause(now, false);
        }
        Ok(())
    }

    pub fn extend_block(&mut self, minutes: i64) -> RuleResult {
        let Some(timer) = self.day.as_mut().and_then(|d| d.timer.as_mut()) else {
            return Err("没有在走的计时");
        };
        let total_minutes = (timer.total_seconds / 60 + minutes.max(1)).min(MAX_BLOCK_MINUTES * 2);
        timer.total_seconds = total_minutes * 60;
        Ok(())
    }

    /// 提前结束这一格：走过的时间直接计入，随后进入暂停（带上休息倒计时）。
    pub fn finish_block(&mut self, now: i64) -> RuleResult {
        let break_seconds = self.preferences.break_minutes * 60;
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        let Some(timer) = day.timer.take() else { return Err("没有在走的计时") };
        let rest = if timer.break_minutes > 0 { timer.break_minutes * 60 } else { break_seconds };
        day.record(timer, true, now);
        day.begin_pause(now, false);
        day.paused_without_block = 0;
        day.break_until = if rest > 0 { Some(now + rest) } else { None };
        Ok(())
    }

    /// 放弃正在走的这一格：走过的时间记为未计入，不给休息，直接进暂停。
    pub fn abandon_block(&mut self, now: i64) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        let Some(timer) = day.timer.take() else { return Err("没有在走的计时") };
        day.record(timer, false, now);
        day.begin_pause(now, false);
        day.paused_without_block = 0;
        day.break_until = None;
        Ok(())
    }

    /// 不休息了：把休息标签摘掉，人仍然停在暂停里，等着开下一格。
    pub fn end_break(&mut self) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        if day.break_until.take().is_none() {
            return Err("现在不在休息");
        }
        Ok(())
    }

    /// 勾掉 / 取消勾掉这一格里的第 index 条任务。
    pub fn toggle_task(&mut self, index: usize) -> RuleResult {
        let Some(timer) = self.day.as_mut().and_then(|d| d.timer.as_mut()) else {
            return Err("没有在走的计时");
        };
        let Some(task) = timer.tasks.get_mut(index) else { return Err("没有这条任务") };
        task.done = !task.done;
        Ok(())
    }

    /// 计时中补一条任务。
    pub fn add_task(&mut self, text: &str) -> RuleResult {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("任务不能为空");
        }
        let Some(timer) = self.day.as_mut().and_then(|d| d.timer.as_mut()) else {
            return Err("没有在走的计时");
        };
        timer.tasks.push(TaskItem { text, done: false });
        Ok(())
    }

    // ---------- 杂项 ----------

    pub fn drink_water(&mut self) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        day.cups += 1;
        day.seated_since_water = 0;
        Ok(())
    }

    /// 撤销一杯（点错了）。
    pub fn undo_water(&mut self) -> RuleResult {
        let Some(day) = &mut self.day else { return Err("今天还没开始") };
        if day.cups <= 0 {
            return Err("今天还没记过水");
        }
        day.cups -= 1;
        Ok(())
    }
}

// ---------- 「下一格」建议（与界面里的 TS 版同一套规则，菜单栏用） ----------

const SITTING_ALARM: i64 = 3 * 3600;
const FRESH_WINDOW: i64 = 3 * 3600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub category: String,
    pub minutes: i64,
    pub reason: String,
    pub pressing: bool,
}

/// 句子里的时长：45 秒 / 25 分钟 / 2 小时 / 5 小时 35 分。
pub fn duration_text(seconds: i64) -> String {
    let safe = seconds.max(0);
    if safe > 0 && safe < 60 {
        return format!("{safe} 秒");
    }
    let minutes = (safe as f64 / 60.0).round() as i64;
    let (h, m) = (minutes / 60, minutes % 60);
    if h == 0 {
        format!("{m} 分钟")
    } else if m == 0 {
        format!("{h} 小时")
    } else {
        format!("{h} 小时 {m} 分")
    }
}

pub fn remaining_seconds(c: &CategoryState) -> i64 {
    (c.quota_minutes * 60 - c.accepted_seconds).max(0)
}

fn role_of<'a>(prefs: &'a Preferences, id: &str) -> &'a str {
    prefs.categories.iter().find(|c| c.id == id).map(|c| c.role.as_str()).unwrap_or("general")
}

/// 统一块长开着就用它，否则用项目各自的默认块长。
pub fn effective_block_minutes(prefs: &Preferences, id: &str) -> i64 {
    if prefs.uniform_block_minutes > 0 {
        return prefs.uniform_block_minutes;
    }
    prefs.categories.iter().find(|c| c.id == id).map(|c| c.default_block_minutes).unwrap_or(50)
}

pub fn block_minutes(c: &CategoryState, prefs: &Preferences) -> i64 {
    let preferred = effective_block_minutes(prefs, &c.id);
    let remaining_minutes = (remaining_seconds(c) + 59) / 60;
    if remaining_minutes <= 0 {
        return preferred;
    }
    preferred.min(remaining_minutes).max(MIN_BLOCK_MINUTES)
}

/// 距离上一次验收通过的锻炼（或今天开始）以来的连续久坐时间，离开的时段扣掉。
pub fn sitting_streak_seconds(day: &Day, prefs: &Preferences, now: i64) -> i64 {
    let mut since = day.started_at;
    for entry in &day.ledger {
        if entry.accepted && role_of(prefs, &entry.category) == "movement" && entry.ended_at > since {
            since = entry.ended_at;
        }
    }
    let elapsed = (now - since).max(0);
    let paused_after: i64 = day
        .pauses
        .iter()
        .filter(|p| p.started_at >= since)
        .map(|p| p.seconds(now))
        .sum();
    (elapsed - paused_after).max(0)
}

fn last_category(day: &Day) -> Option<&str> {
    day.ledger.last().map(|l| l.category.as_str())
}

fn score(c: &CategoryState, day: &Day, prefs: &Preferences, now: i64, candidate_count: usize) -> f64 {
    let target = (c.quota_minutes * 60) as f64;
    if target <= 0.0 {
        return 0.0;
    }
    let mut value = remaining_seconds(c) as f64 / target;
    if last_category(day) == Some(c.id.as_str()) && candidate_count > 1 {
        value *= 0.55;
    }
    match role_of(prefs, &c.id) {
        "deepWork" => {
            if day.seated_seconds < FRESH_WINDOW {
                value *= 1.3;
            }
        }
        "exploration" => {
            let scheduled_deep: Vec<&CategoryState> = day
                .categories
                .iter()
                .filter(|x| role_of(prefs, &x.id) == "deepWork" && x.quota_minutes > 0)
                .collect();
            if !scheduled_deep.is_empty() && !scheduled_deep.iter().any(|x| x.accepted_seconds > 0) {
                value *= 0.5;
            }
        }
        "movement" => {
            let sitting = sitting_streak_seconds(day, prefs, now) as f64;
            value *= 0.5 + (sitting / SITTING_ALARM as f64).min(1.5);
        }
        _ => {}
    }
    value
}

/// 每条建议都带着产生它的那一句理由。
pub fn suggest(day: &Day, prefs: &Preferences, now: i64) -> Option<Suggestion> {
    let candidates: Vec<&CategoryState> = day.categories.iter().filter(|c| remaining_seconds(c) > 0).collect();
    let first = *candidates.first()?;
    let make = |c: &CategoryState, reason: String, pressing: bool| Suggestion {
        category: c.id.clone(),
        minutes: block_minutes(c, prefs),
        reason,
        pressing,
    };

    let sitting = sitting_streak_seconds(day, prefs, now);
    if sitting >= SITTING_ALARM {
        if let Some(movement) = candidates.iter().find(|c| role_of(prefs, &c.id) == "movement") {
            return Some(make(movement, format!("连坐 {}，该动一动。", duration_text(sitting)), true));
        }
    }

    let total_remaining: i64 = candidates.iter().map(|c| remaining_seconds(c)).sum();
    let last = last_category(day);
    let floor = candidates
        .iter()
        .filter(|c| role_of(prefs, &c.id) == "dailyFloor" && last != Some(c.id.as_str()))
        .max_by_key(|c| remaining_seconds(c));
    if let Some(floor) = floor {
        if total_remaining > 0 && remaining_seconds(floor) as f64 / total_remaining as f64 >= 0.5 {
            return Some(make(
                floor,
                format!("缺口最大，还剩 {}。", duration_text(remaining_seconds(floor))),
                true,
            ));
        }
    }

    let mut best = first;
    let mut best_score = score(first, day, prefs, now, candidates.len());
    for c in candidates.iter().skip(1) {
        let s = score(c, day, prefs, now, candidates.len());
        let better = if s != best_score { s > best_score } else { remaining_seconds(c) > remaining_seconds(best) };
        if better {
            best = c;
            best_score = s;
        }
    }
    let remaining = remaining_seconds(best);
    let mut reason = format!("落后最多，还差 {}。", duration_text(remaining));
    if role_of(prefs, &best.id) == "deepWork" && day.seated_seconds < FRESH_WINDOW {
        reason = format!("趁清醒先做，还差 {}。", duration_text(remaining));
    }
    Some(make(best, reason, false))
}

// ---------- hosts 屏蔽规则（纯变换，落盘与提权由外壳负责） ----------

pub const HOSTS_BEGIN: &str = "# BEGIN sitzfleisch-managed";
pub const HOSTS_END: &str = "# END sitzfleisch-managed";

/// 把托管段从 hosts 内容里剥掉，其余行原样保留。
fn strip_managed(current: &str) -> Vec<String> {
    let mut kept = Vec::new();
    let mut inside = false;
    for line in current.lines() {
        if line.trim() == HOSTS_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == HOSTS_END {
            inside = false;
            continue;
        }
        if !inside {
            kept.push(line.to_string());
        }
    }
    kept
}

/// 生成新的 hosts 内容：enable 且列表非空时在文件尾部维护一段托管区，
/// 否则确保托管区不存在。除托管区外一个字节都不动。幂等。
pub fn render_hosts(current: &str, hosts: &[String], enable: bool) -> String {
    let mut lines = strip_managed(current);
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    if enable && !hosts.is_empty() {
        lines.push(String::new());
        lines.push(HOSTS_BEGIN.to_string());
        for host in hosts {
            // hosts 文件不支持通配，所以裸域与 www 各写一份——多数站点默认就在 www 上，
            // 只写裸域等于没挡住。更深的子域挡不了，界面文案也照实说。
            lines.push(format!("127.0.0.1 {host}"));
            lines.push(format!("::1 {host}"));
            if !host.starts_with("www.") {
                lines.push(format!("127.0.0.1 www.{host}"));
                lines.push(format!("::1 www.{host}"));
            }
        }
        lines.push(HOSTS_END.to_string());
    }
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

/// 只读校验：托管段是否在场。
pub fn hosts_section_present(current: &str) -> bool {
    current.lines().any(|l| l.trim() == HOSTS_BEGIN)
}

// ---------- 序列化 ----------

pub fn to_json(state: &State) -> String {
    serde_json::to_string_pretty(state).expect("state serializes")
}

/// v2 → v3：验收取消、离开并进暂停、开格意图变成任务清单。
/// 只搬运，不臆造：读不懂的字段一律丢掉，绝不改动原文件。
fn migrate_v2(value: &mut serde_json::Value) {
    fn tasks_from_intention(entry: &mut serde_json::Value) {
        let text = entry
            .get("intention")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        if let Some(map) = entry.as_object_mut() {
            map.remove("intention");
            map.remove("note");
            map.remove("running");
            map.remove("paused_by_away");
            map.remove("phase");
            if !map.contains_key("tasks") && !text.is_empty() {
                map.insert(
                    "tasks".into(),
                    serde_json::json!([{ "text": text, "done": false }]),
                );
            }
        }
    }

    fn migrate_day(day: &mut serde_json::Value) {
        let Some(map) = day.as_object_mut() else { return };
        // 待验收的那一格：按已走时长直接计入台账，不再拦人。
        if let Some(review) = map.remove("pending_review").filter(|v| !v.is_null()) {
            let seconds = review.get("focus_seconds").and_then(|v| v.as_i64()).unwrap_or(0);
            let category = review.get("category").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let started_at = review.get("started_at").and_then(|v| v.as_i64()).unwrap_or(0);
            if seconds > 0 && !category.is_empty() {
                let mut entry = serde_json::json!({
                    "category": category.clone(),
                    "seconds": seconds,
                    "accepted": true,
                    "started_at": started_at,
                    "ended_at": started_at + seconds,
                    "intention": review.get("intention").cloned().unwrap_or_default(),
                });
                tasks_from_intention(&mut entry);
                if let Some(ledger) = map.get_mut("ledger").and_then(|v| v.as_array_mut()) {
                    ledger.push(entry);
                }
                if let Some(categories) = map.get_mut("categories").and_then(|v| v.as_array_mut()) {
                    for c in categories.iter_mut() {
                        if c.get("id").and_then(|v| v.as_str()) == Some(category.as_str()) {
                            let done = c.get("accepted_seconds").and_then(|v| v.as_i64()).unwrap_or(0);
                            c["accepted_seconds"] = serde_json::json!(done + seconds);
                        }
                    }
                }
            }
        }
        // 离开 → 暂停。
        let aways = map.remove("aways").unwrap_or(serde_json::Value::Null);
        let mut pauses: Vec<serde_json::Value> = Vec::new();
        if let Some(list) = aways.as_array() {
            for a in list {
                pauses.push(serde_json::json!({
                    "started_at": a.get("started_at").and_then(|v| v.as_i64()).unwrap_or(0),
                    "ended_at": a.get("ended_at").cloned().unwrap_or(serde_json::Value::Null),
                    "auto": false,
                }));
            }
        }
        map.insert("pauses".into(), serde_json::Value::Array(pauses));
        if let Some(idle) = map.remove("seated_without_block") {
            map.insert("paused_without_block".into(), idle);
        }
        if let Some(away) = map.remove("away_seconds") {
            map.insert("paused_seconds".into(), away);
        }
        map.remove("away_reason");
        if let Some(timer) = map.get_mut("timer").filter(|v| !v.is_null()) {
            tasks_from_intention(timer);
        }
        if let Some(ledger) = map.get_mut("ledger").and_then(|v| v.as_array_mut()) {
            for entry in ledger.iter_mut() {
                tasks_from_intention(entry);
            }
        }
    }

    if let Some(day) = value.get_mut("day").filter(|v| !v.is_null()) {
        migrate_day(day);
    }
    if let Some(history) = value.get_mut("history").and_then(|v| v.as_array_mut()) {
        for archived in history.iter_mut() {
            if let Some(day) = archived.get_mut("day") {
                migrate_day(day);
            }
        }
    }
    value["schema"] = serde_json::json!(SCHEMA_VERSION);
}

/// 读档。损坏或更高版本一律拒绝解析——调用方必须保留原文件，绝不覆盖。
pub fn from_json(raw: &str) -> Result<State, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("unreadable: {e}"))?;
    let schema = value.get("schema").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if schema > SCHEMA_VERSION {
        return Err(format!("newer schema {schema} > {SCHEMA_VERSION}"));
    }
    if schema < SCHEMA_VERSION {
        migrate_v2(&mut value);
    }
    serde_json::from_value(value).map_err(|e| format!("unreadable: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> State {
        let mut state = State::new(1_000);
        state.start_day("standard", 1_000).unwrap();
        state
    }

    /// 以 60 秒一跳走到目标时刻，避免触发挂起判定。
    fn walk_to(state: &mut State, target: i64) {
        while state.last_tick < target {
            let next = (state.last_tick + 60).min(target);
            state.tick(next);
        }
    }

    fn task(text: &str) -> TaskItem {
        TaskItem { text: text.into(), done: false }
    }

    #[test]
    fn a_day_starts_paused_because_no_block_is_running() {
        let state = started();
        let day = state.day.as_ref().unwrap();
        assert!(day.is_paused(), "还没开格，这段挑项目的时间算暂停");
        assert_eq!(day.pauses.len(), 1);
    }

    #[test]
    fn every_second_is_either_in_a_block_or_paused() {
        let mut state = started();
        walk_to(&mut state, 1_300); // 挑项目 5 分钟
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_600); // 走了 5 分钟
        state.finish_block(1_600).unwrap();
        walk_to(&mut state, 1_900); // 结束后又挂着 5 分钟
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.seated_seconds, 300, "只有格里的时间算在座");
        assert_eq!(day.paused_seconds, 600, "格外的每一秒都进了暂停");
        assert_eq!(day.seated_seconds + day.paused_seconds, 900, "整天没有缝");
        assert!(day.is_paused());
    }

    #[test]
    fn finishing_a_block_pauses_and_starts_a_rest_countdown() {
        let mut state = started();
        state.start_block("main", 25, vec![task("任务三")]).unwrap();
        walk_to(&mut state, 1_600);
        state.finish_block(1_600).unwrap();
        let day = state.day.as_ref().unwrap();
        assert!(day.timer.is_none());
        assert!(day.is_paused(), "结束就进暂停，不用等人点");
        assert_eq!(day.break_until, Some(1_600 + 10 * 60));
        assert!(day.resting(1_700));
        assert!(!day.resting(1_600 + 10 * 60));
        assert_eq!(day.categories[0].accepted_seconds, 600);
        assert_eq!(day.ledger.len(), 1);
        assert!(day.ledger[0].accepted);
    }

    #[test]
    fn natural_completion_counts_pauses_and_rests() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_000 + 25 * 60);
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.categories[0].accepted_seconds, 25 * 60);
        assert!(day.timer.is_none());
        assert!(day.is_paused());
        assert!(day.break_until.is_some());
    }

    #[test]
    fn rest_is_just_a_pause_with_a_deadline() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_600);
        state.finish_block(1_600).unwrap();
        walk_to(&mut state, 1_600 + 10 * 60);
        assert!(state.take_due_break(state.last_tick), "休息到点提醒一次");
        assert!(!state.take_due_break(state.last_tick), "只提醒一次");
        let day = state.day.as_ref().unwrap();
        assert!(day.is_paused(), "休息完仍然是暂停，直到开下一格");
        assert_eq!(day.seated_seconds, 600, "休息不算在座");
    }

    #[test]
    fn ending_the_rest_early_keeps_the_day_paused() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.finish_block(1_100).unwrap();
        state.end_break().unwrap();
        assert!(state.day.as_ref().unwrap().break_until.is_none());
        assert!(state.day.as_ref().unwrap().is_paused());
        assert!(state.end_break().is_err());
    }

    #[test]
    fn zero_break_leaves_no_countdown() {
        let mut state = started();
        state.preferences.break_minutes = 0;
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_600);
        state.finish_block(1_600).unwrap();
        let day = state.day.as_ref().unwrap();
        assert!(day.is_paused());
        assert!(day.break_until.is_none());
    }

    #[test]
    fn pause_only_makes_sense_while_a_block_runs() {
        let mut state = started();
        assert!(state.toggle_pause(1_100).is_err(), "没有格时本来就是暂停");
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_160);
        state.toggle_pause(1_160).unwrap();
        walk_to(&mut state, 1_460);
        {
            let day = state.day.as_ref().unwrap();
            assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 160, "暂停时格不走");
            assert_eq!(day.seated_seconds, 160);
            assert_eq!(day.paused_seconds, 300);
        }
        state.toggle_pause(1_460).unwrap();
        walk_to(&mut state, 1_520);
        assert_eq!(state.day.as_ref().unwrap().timer.as_ref().unwrap().elapsed_seconds, 220);
    }

    #[test]
    fn suspend_gap_credits_nothing_and_pauses() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_030);
        state.tick(1_030 + 3_600); // 一小时休眠
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.seated_seconds, 30);
        assert_eq!(day.suspend_seconds, 3_600);
        assert_eq!(day.paused_seconds, 3_600, "空档也是暂停，读数要看得见");
        assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 30);
        assert!(day.is_paused());
        assert!(day.pauses.last().unwrap().auto);
    }

    #[test]
    fn suspend_gap_is_covered_by_a_pause_span() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_030);
        state.tick(1_030 + 3_600);
        let day = state.day.as_ref().unwrap();
        let last = day.pauses.last().unwrap();
        assert!(last.auto);
        assert_eq!(last.started_at, 1_030, "暂停段要从空档开始那一刻算起，否则运行图会把休眠画成在做事");
        assert_eq!(last.ended_at, None);
    }

    #[test]
    fn invariant_holds_across_suspend() {
        let mut state = started();
        walk_to(&mut state, 1_300); // 挑项目 5 分钟
        state.start_block("main", 25, vec![]).unwrap();
        walk_to(&mut state, 1_600); // 走了 5 分钟
        state.tick(1_600 + 3_600); // 一小时休眠
        walk_to(&mut state, 1_600 + 3_600 + 300); // 醒来后又挂着 5 分钟
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.seated_seconds, 300);
        assert_eq!(
            day.seated_seconds + day.paused_seconds,
            state.last_tick - day.started_at,
            "有休眠的一天也不许留缝"
        );
        assert_eq!(day.suspend_seconds, 3_600, "休眠仍然单独记一笔，作为已暂停里的细分");
    }

    #[test]
    fn restart_gap_is_suspend_even_when_short() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.resume_after_restart(1_030);
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.suspend_seconds, 30);
        assert_eq!(day.paused_seconds, 30);
        assert!(day.is_paused());
    }

    #[test]
    fn restart_gap_is_covered_too() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_060);
        state.resume_after_restart(1_060 + 7_200);
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.suspend_seconds, 7_200);
        assert_eq!(day.paused_seconds, 7_200);
        assert_eq!(day.pauses.last().unwrap().started_at, 1_060, "进程不在的那一段也要有暂停段覆盖");
        assert!(day.pauses.last().unwrap().auto);
        assert_eq!(
            day.seated_seconds + day.paused_seconds,
            state.last_tick - day.started_at
        );
    }

    #[test]
    fn a_block_that_overruns_a_tick_lands_the_rest_in_pause() {
        let mut state = started();
        state.start_block("main", 5, vec![]).unwrap(); // 5 分钟 = 300 秒
        walk_to(&mut state, 1_240);
        state.tick(1_340); // 一跳 100 秒，格在第 60 秒就走完了
        let day = state.day.as_ref().unwrap();
        assert!(day.timer.is_none());
        assert_eq!(day.seated_seconds, 300, "只有真正在格里的 300 秒算在座");
        assert_eq!(day.paused_seconds, 40, "格走完之后余下的 40 秒是暂停");
        assert_eq!(day.ledger[0].ended_at, 1_300, "台账按格真正走完的时刻收口");
        assert_eq!(day.pauses.last().unwrap().started_at, 1_300);
        assert_eq!(day.break_until, Some(1_300 + 10 * 60));
    }

    #[test]
    fn abandoning_a_block_records_but_does_not_count() {
        let mut state = started();
        state.start_block("main", 25, vec![task("任务三")]).unwrap();
        walk_to(&mut state, 1_120);
        state.abandon_block(1_120).unwrap();
        let day = state.day.as_ref().unwrap();
        assert!(day.timer.is_none());
        assert!(day.is_paused());
        assert!(day.break_until.is_none(), "放弃不给休息");
        assert_eq!(day.ledger.len(), 1);
        assert!(!day.ledger[0].accepted);
        assert_eq!(day.categories[0].accepted_seconds, 0);
        assert!(state.abandon_block(1_120).is_err());
    }

    #[test]
    fn tasks_can_be_ticked_and_added_mid_block() {
        let mut state = started();
        state.start_block("reading", 45, vec![task("听力"), task("单词")]).unwrap();
        state.toggle_task(1).unwrap();
        state.add_task("再做点阅读").unwrap();
        assert!(state.add_task("   ").is_err());
        assert!(state.toggle_task(9).is_err());
        walk_to(&mut state, 1_300);
        state.finish_block(1_300).unwrap();
        let entry = &state.day.as_ref().unwrap().ledger[0];
        assert_eq!(entry.tasks.len(), 3);
        assert_eq!(entry.done_tasks(), 1);
        assert!(entry.tasks[1].done);
    }

    #[test]
    fn blocks_can_start_with_no_tasks() {
        let mut state = started();
        state.start_block("reading", 45, vec![]).unwrap();
        assert!(state.day.as_ref().unwrap().timer.as_ref().unwrap().tasks.is_empty());
        state.abandon_block(1_100).unwrap();
        state.start_block("reading", 45, vec![task("  "), task("听力")]).unwrap();
        assert_eq!(state.day.as_ref().unwrap().timer.as_ref().unwrap().tasks.len(), 1);
    }

    #[test]
    fn starting_a_block_ends_the_pause_and_the_rest() {
        let mut state = started();
        state.start_block("main", 25, vec![]).unwrap();
        state.finish_block(1_100).unwrap();
        assert!(state.day.as_ref().unwrap().resting(1_200));
        state.start_block("main", 25, vec![]).unwrap();
        let day = state.day.as_ref().unwrap();
        assert!(!day.is_paused());
        assert!(day.break_until.is_none());
    }

    #[test]
    fn end_day_archives_running_block_and_abandon_does_not() {
        let mut state = started();
        state.start_block("main", 5, vec![]).unwrap();
        walk_to(&mut state, 1_060);
        state.end_day(1_060).unwrap();
        assert_eq!(state.history.len(), 1);
        assert!(state.day.is_none());
        let archived = &state.history[0].day;
        assert_eq!(archived.ledger.len(), 1);
        assert!(archived.ledger[0].accepted);
        assert_eq!(archived.ledger[0].seconds, 60);
        assert!(archived.pauses.iter().all(|p| p.ended_at.is_some()), "收工要把暂停收口");

        state.start_day("light", 6_000).unwrap();
        state.abandon_day().unwrap();
        assert_eq!(state.history.len(), 1);
    }

    #[test]
    fn delete_history_targets_one_day() {
        let mut state = started();
        state.end_day(2_000).unwrap();
        state.start_day("light", 3_000).unwrap();
        state.end_day(4_000).unwrap();
        assert_eq!(state.history.len(), 2);
        state.delete_history_day(3_000).unwrap();
        assert_eq!(state.history.len(), 1);
        assert!(state.delete_history_day(9_999).is_err());
    }

    #[test]
    fn switch_profile_keeps_progress_and_parks_missing_categories() {
        let mut state = State::new(1_000);
        state.preferences.profiles[0].quotas = vec![quota("main", 120), quota("reading", 60)];
        state.start_day("standard", 1_000).unwrap();
        state.start_block("writing", 25, vec![]).unwrap();
        walk_to(&mut state, 1_500);
        state.finish_block(1_500).unwrap();
        state.switch_profile("light").unwrap();
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.profile_name, "轻量");
        let ids: Vec<&str> = day.categories.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["main", "reading", "writing"]);
        let writing = day.categories.iter().find(|c| c.id == "writing").unwrap();
        assert_eq!(writing.quota_minutes, 0);
        assert_eq!(writing.accepted_seconds, 500);
        assert!(state.switch_profile("nope").is_err());
    }

    #[test]
    fn preferences_validation_and_zero_quota_skipped() {
        let mut state = State::new(0);
        state.preferences.profiles[1].quotas = vec![quota("main", 240), quota("reading", 0)];
        state.start_day("standard", 0).unwrap();
        assert_eq!(state.day.as_ref().unwrap().categories.len(), 1);

        let mut prefs = builtin_preferences();
        prefs.categories[0].name = "  ".into();
        assert!(validate_preferences(&prefs).is_err());
        let mut prefs = builtin_preferences();
        prefs.profiles[0].quotas = vec![quota("ghost", 60)];
        assert!(validate_preferences(&prefs).is_err());
        let mut prefs = builtin_preferences();
        prefs.break_minutes = 999;
        assert!(validate_preferences(&prefs).is_err());
    }

    #[test]
    fn water_and_stretch_only_fire_inside_a_block() {
        let mut state = started();
        state.preferences.water_reminder_enabled = true;
        state.preferences.water_reminder_minutes = 1;
        // 还没开格：一直暂停，不催喝水
        walk_to(&mut state, 1_000 + 600);
        assert!(!state.take_due_reminders().0, "暂停时不催喝水");
        state.start_block("main", 25, vec![]).unwrap();
        let started_at = state.last_tick;
        walk_to(&mut state, started_at + 61);
        assert!(state.take_due_reminders().0, "格里走满就催");
    }

    #[test]
    fn idle_reminder_fires_while_paused_without_a_block() {
        let mut state = started();
        state.preferences.idle_reminder_enabled = true;
        state.preferences.idle_reminder_minutes = 1;
        walk_to(&mut state, 1_000 + 61);
        assert!(state.take_due_reminders().2, "暂停这么久还没开格，该催了");
        state.start_block("main", 25, vec![]).unwrap();
        let now = state.last_tick;
        walk_to(&mut state, now + 300);
        assert!(!state.take_due_reminders().2, "格在走就不催");
    }

    #[test]
    fn water_can_be_logged_and_undone() {
        let mut state = started();
        assert!(state.undo_water().is_err());
        state.drink_water().unwrap();
        state.drink_water().unwrap();
        state.undo_water().unwrap();
        assert_eq!(state.day.as_ref().unwrap().cups, 1);
    }

    #[test]
    fn suggestion_explains_itself() {
        let mut state = State::new(1_000);
        state.preferences.categories.push(def("gym", "锻炼", 45, "dumbbell", "movement"));
        state.preferences.profiles[1].quotas.push(quota("gym", 60));
        state.start_day("standard", 1_000).unwrap();
        let s = suggest(state.day.as_ref().unwrap(), &state.preferences, 1_000).unwrap();
        assert_eq!(s.category, "main");
        assert!(s.reason.starts_with("趁清醒先做"), "{}", s.reason);
        assert!(!s.pressing);
        assert_eq!(duration_text(5 * 3600 + 35 * 60), "5 小时 35 分");
        assert_eq!(duration_text(45), "45 秒");
    }

    #[test]
    fn sitting_streak_ignores_paused_time() {
        let mut state = started();
        state.start_block("main", 90, vec![]).unwrap();
        walk_to(&mut state, 1_000 + 600);
        let paused_at = state.last_tick;
        state.toggle_pause(paused_at).unwrap();
        walk_to(&mut state, paused_at + 1_200);
        let resumed_at = state.last_tick;
        state.toggle_pause(resumed_at).unwrap();
        let now = state.last_tick;
        let day = state.day.as_ref().unwrap();
        assert_eq!(sitting_streak_seconds(day, &state.preferences, now), 600, "暂停的 20 分钟不算连坐");
    }

    #[test]
    fn hosts_render_is_idempotent_and_preserves_rest() {
        let original = "127.0.0.1 localhost\n255.255.255.255 broadcasthost\n";
        let hosts = vec!["weibo.com".to_string()];
        let once = render_hosts(original, &hosts, true);
        assert!(once.contains("127.0.0.1 weibo.com"));
        assert!(once.contains("::1 weibo.com"));
        assert!(once.contains("127.0.0.1 www.weibo.com"), "不带 www 的规则挡不住多数站点");
        assert!(once.contains("::1 www.weibo.com"));
        assert_eq!(render_hosts(&once, &hosts, true), once, "幂等");
        let off = render_hosts(&once, &hosts, false);
        assert!(!hosts_section_present(&off));
        assert_eq!(off.trim_end(), original.trim_end(), "解除后恢复原样");
    }

    #[test]
    fn host_validation_normalizes_and_rejects() {
        assert_eq!(validate_host(" HTTPS://Weibo.com/ ").unwrap(), "weibo.com");
        assert_eq!(validate_host("weibo.com/hot?tab=1#top").unwrap(), "weibo.com", "路径与查询串剥掉");
        assert_eq!(validate_host("www.zhihu.com").unwrap(), "zhihu.com", "存储形态统一去掉 www");
        assert_eq!(validate_host("http://example.com:8080/x").unwrap(), "example.com", "端口剥掉");
        assert!(validate_host("localhost").is_err());
        assert!(validate_host("127.0.0.1").is_err(), "IP 不能写进屏蔽列表");
        assert!(validate_host("192.168.1.1").is_err());
        assert!(validate_host("ftp://example.com").is_err(), "只支持 http/https");
        assert!(validate_host("user@example.com").is_err());
        assert!(validate_host("中文.com").is_err());
        assert!(validate_host("").is_err());
    }

    #[test]
    fn render_hosts_covers_www_variant() {
        let hosts = vec!["zhihu.com".to_string()];
        let out = render_hosts("", &hosts, true);
        let lines: Vec<&str> = out.lines().filter(|l| l.contains("zhihu.com")).collect();
        assert_eq!(lines.len(), 4, "每个域名四行：两个协议族 × 裸域与 www");
        assert_eq!(render_hosts(&out, &hosts, true), out, "幂等");
    }

    #[test]
    fn json_roundtrip_and_newer_schema_rejected() {
        let mut state = started();
        state.start_block("main", 25, vec![task("任务三")]).unwrap();
        walk_to(&mut state, 1_300);
        state.toggle_pause(1_300).unwrap();
        let raw = to_json(&state);
        let back = from_json(&raw).unwrap();
        assert_eq!(back.schema, SCHEMA_VERSION);
        assert!(back.day.as_ref().unwrap().is_paused());
        assert_eq!(back.day.as_ref().unwrap().timer.as_ref().unwrap().tasks[0].text, "任务三");
        assert!(from_json("{ not json").is_err());
        let newer = raw.replace(
            &format!("\"schema\": {SCHEMA_VERSION}"),
            &format!("\"schema\": {}", SCHEMA_VERSION + 1),
        );
        assert!(from_json(&newer).is_err(), "更高版本拒读");
    }

    #[test]
    fn v2_state_migrates_to_v3() {
        let raw = r#"{
          "schema": 2,
          "last_tick": 5000,
          "preferences": {
            "categories": [
              { "id": "deep", "name": "深度工作", "default_block_minutes": 90, "icon": "brain", "role": "deepWork", "block_rationale": "" },
              { "id": "browse", "name": "浏览", "default_block_minutes": 45, "icon": "newspaper", "role": "exploration", "block_rationale": "" }
            ],
            "profiles": [
              { "id": "standard", "name": "标准", "subtitle": "", "quotas": [{ "category": "deep", "minutes": 240 }, { "category": "browse", "minutes": 60 }] }
            ],
            "break_minutes": 10
          },
          "day": {
            "profile_name": "标准",
            "profile_id": "standard",
            "started_at": 1000,
            "categories": [
              { "id": "deep", "name": "深度工作", "quota_minutes": 240, "accepted_seconds": 600 },
              { "id": "browse", "name": "浏览", "quota_minutes": 60, "accepted_seconds": 0 }
            ],
            "timer": { "category": "deep", "phase": "focus", "running": true, "total_seconds": 1500, "elapsed_seconds": 300, "intention": "任务三", "started_at": 4000, "break_minutes": 10, "paused_by_away": false },
            "pending_review": { "category": "browse", "focus_seconds": 900, "intention": "任务二", "started_at": 2000, "break_minutes": 0 },
            "ledger": [
              { "category": "deep", "seconds": 600, "accepted": true, "intention": "任务一", "note": "做完了", "started_at": 1000, "ended_at": 1600 }
            ],
            "seated_seconds": 3000,
            "away_seconds": 400,
            "suspend_seconds": 0,
            "away_reason": "吃饭",
            "cups": 3,
            "seated_without_block": 120,
            "aways": [{ "reason": "吃饭", "started_at": 4500, "ended_at": null }]
          },
          "history": []
        }"#;
        let state = from_json(raw).unwrap();
        assert_eq!(state.schema, SCHEMA_VERSION);
        assert_eq!(state.preferences.categories[0].name, "深度工作", "迁移不改名字，只改结构");
        let day = state.day.as_ref().unwrap();
        assert_eq!(day.paused_seconds, 400);
        assert!(day.is_paused());
        assert_eq!(day.paused_without_block, 120);
        assert_eq!(day.categories[1].accepted_seconds, 900, "待验收的一格补记进来");
        assert_eq!(day.ledger.len(), 2);
        assert_eq!(day.ledger[0].tasks[0].text, "任务一");
        assert_eq!(day.timer.as_ref().unwrap().tasks[0].text, "任务三");
    }
}
