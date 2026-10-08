// 浏览器里的模拟后台：把 core 的规则用 TS 复刻一遍，让界面能在没有 Tauri 的地方
// 跑起来、点起来、截图验收。只在非 Tauri 环境被动态加载，不进正式包的主路径。
// 场景通过 URL hash 选：#qa=start|fresh|chooser|completed|finishing|running|paused|suspended|resting|done|protected|savefail|nohistory
// 可加 &platform=android 预览平台能力，&clock=<带时区的 ISO 时间> 固定截图时钟；手机下 &system=blocked|lowered 预览提醒受限。
// 应用屏蔽（手机）：&apps=off|none 预览开关关着或从没设置过，&a11y=off|stalled 预览无障碍服务没开或开着却没在运行，
// &applist=limited 预览系统只给部分应用，&notice=<包名> 预览刚被送回坐功的提示（也可在控制台设 qaBlockNotice），
// &bg=restricted 预览后台活动没被允许（划掉就会被强行停止）。

import type {
  AppBlocking,
  AppState,
  ArchivedDay,
  CategoryState,
  Day,
  PlatformInfo,
  Preferences,
  InstalledApp,
  ProfileDef,
  Snapshot,
  SystemStatus,
  TaskItem,
} from "../types";
import { MAX_BLOCK_MINUTES, MIN_BLOCK_MINUTES, SUSPEND_GAP_SECONDS, MAX_COMPLETION_NOTE_CHARS, NOTIFICATION_CHANNELS, SYSTEM_SETTINGS_TARGETS, type SystemSettingsTarget } from "../types";
import { version } from "../../package.json";
import { MAX_BLOCK_RULES, normalizeHost, normalizeUrl } from "../blocking";
import { MAX_BLOCKED_APPS, validPackageName } from "../app-blocking";

const HISTORY_LIMIT = 60;
const params = new URLSearchParams(location.hash.replace(/^#/, ""));
const mobile = params.get("platform") === "android";

// 只接受带时区的完整时间；非法日期不能被 Date.parse 悄悄滚到下个月。
const clock = params.get("clock");
if (clock !== null && clock.length <= 35
  && /^\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\d|3[01])T([01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d{1,3})?(?:Z|[+-]([01]\d|2[0-3]):[0-5]\d)$/.test(clock)) {
  const date = new Date(`${clock.slice(0, 10)}T00:00:00Z`);
  const fixedNow = Date.parse(clock);
  if (Number.isFinite(fixedNow) && date.toISOString().slice(0, 10) === clock.slice(0, 10)) {
    Date.now = () => fixedNow;
  }
}

const platform: PlatformInfo = {
  os: mobile ? "android" : "macos",
  mobile,
  features: {
    tray: !mobile,
    website_blocking: !mobile,
    browser_extension: !mobile,
    autostart: !mobile,
    reveal_state_file: !mobile,
    window_title: !mobile,
    quit_flow: !mobile,
    in_app_sound_toggle: !mobile,
    system_settings: mobile,
    exact_alarm_status: mobile,
    app_blocking: mobile,
  },
};

let notificationPermission = mobile && params.get("permission") === "denied" ? "denied"
  : mobile && params.get("permission") === "unknown" ? "unknown" : "granted";
const blockedSystem = params.get("system") === "blocked";
// 荣耀等系统给新应用的提醒渠道降一级（不弹横幅）：#system=lowered。
const loweredSystem = params.get("system") === "lowered";
const systemFixture: SystemStatus = {
  sdkInt: 36,
  manufacturer: "HONOR",
  notificationsEnabled: notificationPermission === "granted",
  channels: NOTIFICATION_CHANNELS.map((id) => ({
    id, name: { timer: "计时", body: "身体提醒", water: "喝水", status: "进行中" }[id],
    enabled: !blockedSystem, importance: blockedSystem ? 0 : id === "status" ? 2 : loweredSystem ? 3 : 4,
    vibration: !blockedSystem && id !== "status",
    sound: blockedSystem || id === "status" ? null : id === "water" ? "android.resource://com.snooze26h.sitzfleisch.x.debug/raw/water" : "content://settings/system/notification_sound",
  })),
  canScheduleExactAlarms: !blockedSystem,
  ignoringBatteryOptimizations: false,
  appBlockServiceEnabled: params.get("a11y") !== "off",
  appBlockServiceRunning: params.get("a11y") !== "off" && params.get("a11y") !== "stalled",
  backgroundRestricted: params.get("bg") === "restricted",
};

// 预览用的手机应用；名字只为看排版和搜索，不读取任何真实设备。
const installedFixture: InstalledApp[] = [
  ["com.ss.android.ugc.aweme", "抖音"], ["tv.danmaku.bili", "哔哩哔哩"], ["com.xingin.xhs", "小红书"],
  ["com.tencent.mm", "微信"], ["com.tencent.mobileqq", "QQ"], ["com.sina.weibo", "微博"],
  ["com.zhihu.android", "知乎"], ["com.smile.gifmaker", "快手"], ["com.taobao.taobao", "淘宝"],
  ["com.jingdong.app.mall", "京东"], ["com.netease.cloudmusic", "网易云音乐"], ["com.tencent.qqmusic", "QQ音乐"],
  ["com.dragon.read", "番茄免费小说"], ["com.ss.android.article.news", "今日头条"], ["com.baidu.searchbox", "百度"],
  ["com.eg.android.AlipayGphone", "支付宝"], ["com.sankuai.meituan", "美团"], ["com.autonavi.minimap", "高德地图"],
  ["com.douban.frodo", "豆瓣"], ["com.tencent.tmgp.sgame", "王者荣耀"], ["com.miHoYo.Yuanshen", "原神"],
  ["com.hihonor.camera", "相机"], ["com.hihonor.photos", "图库"], ["com.android.calendar", "日历"],
  ["com.hihonor.notepad", "备忘录"], ["com.google.android.youtube", "YouTube"], ["com.twitter.android", "X"],
].map(([packageName, label]) => ({ packageName, label }));
let appListGranted = params.get("applist") !== "limited";
if (mobile && params.get("notice")) window.qaBlockNotice = params.get("notice") ?? undefined;
if (mobile) window.qaSystemStatus = structuredClone(systemFixture);

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

// ---------- 示例计划（仅供预览，不读取真实用户数据） ----------

function prefsFixture(): Preferences {
  return {
    categories: [
      { id: "deep", name: "深度工作", short_name: "深度工作", default_block_minutes: 90, icon: "brain", role: "deepWork", block_rationale: "进入状态本身要花 15 分钟，切太碎等于一直在热身。" },
      { id: "browse", name: "浏览", short_name: "浏览", default_block_minutes: 30, icon: "newspaper", role: "general", block_rationale: "论坛、博客、聊天记录都算这里，短块能逼你按时收口。" },
      { id: "reading", name: "阅读", short_name: "阅读", default_block_minutes: 45, icon: "book-open", role: "dailyFloor", block_rationale: "输入类任务注意力衰减快，短块多轮比一小时硬撑有效。" },
      { id: "practice", name: "练习", short_name: "练习", default_block_minutes: 60, icon: "binary", role: "general", block_rationale: "一道题从读题到写清思路，通常就是这个量级。" },
      { id: "move", name: "运动", short_name: "运动", default_block_minutes: 45, icon: "dumbbell", role: "movement", block_rationale: "含热身和拉伸，够一次完整训练。" },
    ],
    profiles: [
      { id: "standard", name: "今天", subtitle: "", quotas: [q("deep", 240), q("browse", 30), q("reading", 180), q("practice", 0), q("move", 60)] },
    ],
    break_minutes: 10,
    hydration_goal_cups: 8,
    water_reminder_enabled: true,
    water_reminder_minutes: 30,
    stretch_reminder_enabled: true,
    stretch_reminder_minutes: 50,
    idle_reminder_enabled: true,
    idle_reminder_minutes: 15,
    blocked_hosts: ["weibo.com", "bilibili.com"],
    blocked_urls: [],
    uniform_block_minutes: 0,
    default_profile_id: "standard",
    sound_enabled: true,
    ...(mobile && params.get("apps") !== "none" ? { app_blocking: appBlockingFixture() } : {}),
  };
}

function appBlockingFixture(): AppBlocking {
  return {
    enabled: params.get("apps") !== "off",
    apps: [{ package_name: "com.ss.android.ugc.aweme", label: "抖音" }, { package_name: "tv.danmaku.bili", label: "哔哩哔哩" }],
  };
}

function q(category: string, minutes: number) {
  return { category, minutes };
}

function task(text: string, done = false): TaskItem {
  return { text, done };
}

// ---------- 规则（core 的 TS 复刻） ----------

/** 模拟后台自己要留着历史；真实的 Snapshot 把历史单独拎出去了，见 `snapshot()`。 */
type MockState = AppState & { history: ArchivedDay[] };

declare global {
  interface Window {
    /** QA：浏览器里没有「退出」这个动作，退出前保存失败的对话框从控制台叫出来。 */
    qaQuitBlocked?: () => void;
    qaQuitBlockingFailed?: () => void;
    qaBackButton?: () => void | Promise<void>;
    qaBackgrounded?: number;
    qaSystemStatus?: SystemStatus;
    qaSystemRequests?: { target: SystemSettingsTarget; channelId?: string }[];
    qaPermissionRequests?: number;
    qaTestNotifications?: string[];
    /** QA：下一次回到前台时，「被送回坐功」的是哪个应用（包名）。 */
    qaBlockNotice?: string;
  }
}

let state: MockState;
let snapshotRevision = 0;
let writeProtected: string | null = null;
let saveError: string | null = null;
const blocking = {
  active: false, busy: false, error: null as string | null,
  browser: { available: false, connected: false, synced: false, supports_hosts: false, error: null as string | null },
};
let autostart = false;
const listeners: ((snapshot: Snapshot) => void)[] = [];

function dayFromProfile(prefs: Preferences, profile: ProfileDef, now: number): Day {
  const categories: CategoryState[] = [];
  for (const quota of profile.quotas) {
    if (quota.minutes <= 0) continue;
    const def = prefs.categories.find((c) => c.id === quota.category);
    if (def) categories.push({ id: def.id, name: def.name, quota_minutes: quota.minutes, accepted_seconds: 0 });
  }
  return {
    profile_name: profile.name,
    profile_id: profile.id,
    started_at: now,
    categories,
    timer: null,
    ledger: [],
    seated_seconds: 0,
    paused_seconds: 0,
    suspend_seconds: 0,
    cups: 0,
    seated_since_water: 0,
    seated_since_relief: 0,
    paused_without_block: 0,
    break_until: null,
    pauses: [],
    rests: [],
  };
}

function paused(day: Day): boolean {
  const last = day.pauses[day.pauses.length - 1];
  return !!last && last.ended_at === null;
}

function beginPause(day: Day, now: number, auto: boolean) {
  if (!paused(day)) day.pauses.push({ started_at: now, ended_at: null, auto });
}

function closePause(day: Day, now: number) {
  const last = day.pauses[day.pauses.length - 1];
  if (last && last.ended_at === null) last.ended_at = Math.max(last.started_at, now);
}

/** 与 core 的 begin_rest 同一条：休息标签和休息记录一起落下。 */
function beginRest(day: Day, from: number, seconds: number) {
  if (seconds > 0) {
    day.break_until = from + seconds;
    day.rests.push({ started_at: from, ended_at: from + seconds });
  } else {
    day.break_until = null;
  }
}

/** 与 core 的 cut_rest 同一条：提前结束的休息截到这一刻，自然到点的不动。 */
function cutRest(day: Day, now: number) {
  if (day.break_until === null) return;
  day.break_until = null;
  const last = day.rests[day.rests.length - 1];
  if (last && last.ended_at > now) last.ended_at = Math.max(last.started_at, now);
}

function record(day: Day, timer: NonNullable<Day["timer"]>, accepted: boolean, now: number) {
  const seconds = Math.max(0, timer.elapsed_seconds);
  if (seconds === 0) return;
  if (accepted) {
    const category = day.categories.find((c) => c.id === timer.category);
    if (category) category.accepted_seconds += seconds;
  }
  day.ledger.push({
    category: timer.category,
    seconds,
    accepted,
    tasks: timer.tasks,
    started_at: timer.started_at > 0 ? timer.started_at : now - seconds,
    ended_at: now,
    completion_note: accepted ? null : "",
  });
}

function tick(now: number) {
  const delta = now - state.last_tick;
  state.last_tick = now;
  if (delta <= 0) return;
  const day = state.day;
  if (!day) return;
  if (!mobile && delta > SUSPEND_GAP_SECONDS) {
    // 与 core 同一条规矩：空档一秒不补，但暂停段从空档开始那一刻起算。
    day.suspend_seconds += delta;
    day.paused_seconds += delta;
    beginPause(day, now - delta, true);
    return;
  }
  if (paused(day)) {
    day.paused_seconds += delta;
    // 与 core 同一条：闲置提醒只数休息以外的暂停。
    if (!day.timer && !(day.break_until !== null && now < day.break_until)) day.paused_without_block += delta;
    return;
  }
  const timer = day.timer;
  if (!timer) {
    beginPause(day, now - delta, false);
    day.paused_seconds += delta;
    day.paused_without_block += delta;
    return;
  }
  const room = Math.max(0, timer.total_seconds - timer.elapsed_seconds);
  const worked = Math.min(delta, room);
  const overflow = delta - worked;
  timer.elapsed_seconds += worked;
  day.seated_seconds += worked;
  day.seated_since_relief += worked;
  if (timer.elapsed_seconds < timer.total_seconds) return;
  const endedAt = now - overflow;
  day.timer = null;
  const rest = restSeconds(timer);
  record(day, timer, true, endedAt);
  beginPause(day, endedAt, false);
  day.paused_seconds += overflow;
  day.paused_without_block = overflow;
  beginRest(day, endedAt, rest);
}

/** 与 core 的 rest_seconds 同一条：开格时定下的休息分钟数说了算，0 就是不休息；早期存档没定下来的（-1）才看偏好。 */
function restSeconds(timer: NonNullable<Day["timer"]>): number {
  return (timer.break_minutes >= 0 ? timer.break_minutes : state.preferences.break_minutes) * 60;
}

function needDay(): Day {
  if (!state.day) throw "今天还没开始";
  return state.day;
}

const ops = {
  start_day(profileId: string) {
    if (state.day) throw "今天已经开始了";
    const profile = state.preferences.profiles.find((p) => p.id === profileId);
    if (!profile) throw "没有这个计划";
    const day = dayFromProfile(state.preferences, profile, state.last_tick);
    if (!day.categories.length) throw "这个计划没有任何配了时的项目";
    beginPause(day, state.last_tick, false);
    state.day = day;
  },
  switch_profile(profileId: string) {
    const profile = state.preferences.profiles.find((p) => p.id === profileId);
    if (!profile) throw "没有这个计划";
    const day = needDay();
    const categories: CategoryState[] = [];
    for (const quota of profile.quotas) {
      if (quota.minutes <= 0) continue;
      const existing = day.categories.find((c) => c.id === quota.category);
      if (existing) categories.push({ ...existing, quota_minutes: quota.minutes });
      else {
        const def = state.preferences.categories.find((c) => c.id === quota.category);
        if (def) categories.push({ id: def.id, name: def.name, quota_minutes: quota.minutes, accepted_seconds: 0 });
      }
    }
    for (const existing of day.categories) {
      const touched =
        existing.accepted_seconds > 0 ||
        day.ledger.some((l) => l.category === existing.id) ||
        day.timer?.category === existing.id;
      if (!categories.some((c) => c.id === existing.id) && touched) categories.push({ ...existing, quota_minutes: 0 });
    }
    if (!categories.length) throw "这个计划没有任何配了时的项目";
    const order = state.preferences.categories.map((c) => c.id);
    categories.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
    day.categories = categories;
    day.profile_name = profile.name;
    day.profile_id = profile.id;
  },
  abandon_day() {
    needDay();
    state.day = null;
  },
  end_day(now: number) {
    const day = needDay();
    const timer = day.timer;
    day.timer = null;
    if (timer) record(day, timer, true, now);
    cutRest(day, now);
    closePause(day, now);
    state.history.push({ day, ended_at: now });
    state.day = null;
    if (state.history.length > HISTORY_LIMIT) state.history.splice(0, state.history.length - HISTORY_LIMIT);
  },
  delete_history_day(startedAt: number) {
    const before = state.history.length;
    state.history = state.history.filter((d: ArchivedDay) => d.day.started_at !== startedAt);
    if (state.history.length === before) throw "没有这一天的归档";
  },
  start_block(categoryId: string, minutes: number, tasks: TaskItem[], breakMinutes: number) {
    if (!Number.isInteger(minutes) || minutes < MIN_BLOCK_MINUTES || minutes > MAX_BLOCK_MINUTES) throw "专注时长要在 1–180 分钟之间";
    const day = needDay();
    if (day.timer) throw "已经有一格在走";
    if (!day.categories.some((c) => c.id === categoryId)) throw "今天没有这个项目";
    // 与 core 同一条：两格之间停够 5 分钟，就当起来活动过。
    const open = day.pauses[day.pauses.length - 1];
    if (open && open.ended_at === null && state.last_tick - open.started_at >= 5 * 60) day.seated_since_relief = 0;
    cutRest(day, state.last_tick);
    closePause(day, state.last_tick);
    day.timer = {
      category: categoryId,
      total_seconds: minutes * 60,
      elapsed_seconds: 0,
      tasks: tasks.map((t) => ({ text: t.text.trim(), done: t.done })).filter((t) => t.text),
      started_at: state.last_tick,
      break_minutes: Math.min(120, Math.max(0, breakMinutes)),
    };
    day.paused_without_block = 0;
  },
  toggle_pause(now: number) {
    const day = needDay();
    if (!day.timer) throw "没有在走的格";
    if (paused(day)) {
      closePause(day, now);
      cutRest(day, now);
      day.seated_since_relief = 0;
    } else {
      beginPause(day, now, false);
    }
  },
  extend_block(minutes: number) {
    const timer = state.day?.timer;
    if (!timer) throw "没有在走的计时";
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > MAX_BLOCK_MINUTES * 2) throw "增加时长要在 1–360 分钟之间";
    const total = timer.total_seconds / 60 + minutes;
    if (total > MAX_BLOCK_MINUTES * 2) throw "一格最多延长到 360 分钟";
    timer.total_seconds = total * 60;
  },
  finish_block(now: number) {
    const day = needDay();
    const timer = day.timer;
    if (!timer) throw "没有在走的计时";
    day.timer = null;
    // 一秒都没走就点了结束：台账里什么也没记，也就没有休息可给。
    const rest = timer.elapsed_seconds > 0 ? restSeconds(timer) : 0;
    record(day, timer, true, now);
    beginPause(day, now, false);
    day.paused_without_block = 0;
    beginRest(day, now, rest);
  },
  abandon_block(now: number) {
    const day = needDay();
    const timer = day.timer;
    if (!timer) throw "没有在走的计时";
    day.timer = null;
    record(day, timer, false, now);
    beginPause(day, now, false);
    day.paused_without_block = 0;
    cutRest(day, now);
  },
  end_break() {
    const day = needDay();
    if (day.break_until === null) throw "现在不在休息";
    cutRest(day, state.last_tick);
  },
  set_completion_note(dayStartedAt: number, entryIndex: number, endedAt: number, note: string) {
    if (typeof note !== "string" || [...note].length > MAX_COMPLETION_NOTE_CHARS) throw "完成记录最多 2000 字";
    if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/.test(note)) throw "完成记录含有不支持的控制字符";
    const target = [state.day, ...state.history.map((a) => a.day)].find((d) => d?.started_at === dayStartedAt);
    const entry = target?.ledger[entryIndex];
    if (!target) throw "这一天的记录已不存在";
    if (!Number.isInteger(entryIndex) || !entry?.accepted || entry.ended_at !== endedAt) throw "这段记录已改变，请重新打开";
    entry.completion_note = note.replace(/\r\n?/g, "\n").trim();
  },
  update_preferences(prefs: Preferences) {
    // 旧开发场景没有精确网址字段，按空列表接入，保留原来的整站规则。
    prefs.blocked_urls ??= [];
    if (prefs.blocked_hosts.length + prefs.blocked_urls.length > MAX_BLOCK_RULES) throw `最多添加 ${MAX_BLOCK_RULES} 条网站屏蔽规则`;
    prefs.blocked_hosts = [...new Set(prefs.blocked_hosts.map(validateHost))];
    prefs.blocked_urls = [...new Set(prefs.blocked_urls.map(validateUrl))];
    if (!prefs.categories.length) throw "至少要有一个项目";
    if (!prefs.profiles.length) throw "至少要有一份计划";
    if (prefs.uniform_block_minutes !== 0 && (!Number.isInteger(prefs.uniform_block_minutes) || prefs.uniform_block_minutes < MIN_BLOCK_MINUTES || prefs.uniform_block_minutes > MAX_BLOCK_MINUTES)) throw "统一块长要在 1–180 分钟之间";
    if (prefs.break_minutes < 0 || prefs.break_minutes > 120) throw "休息时长要在 0–120 分钟之间";
    for (const m of [prefs.stretch_reminder_minutes, prefs.idle_reminder_minutes]) {
      if (m < 5 || m > 240) throw "提醒间隔要在 5–240 分钟之间";
    }
    for (const c of prefs.categories) {
      if (!c.id.trim() || !c.name.trim()) throw "项目名不能为空";
      if (!Number.isInteger(c.default_block_minutes) || c.default_block_minutes < MIN_BLOCK_MINUTES || c.default_block_minutes > MAX_BLOCK_MINUTES) throw "默认块时长要在 1–180 分钟之间";
    }
    for (const p of prefs.profiles) {
      if (!p.name.trim()) throw "计划名不能为空";
      if (!p.quotas.some((x) => x.minutes > 0)) throw "至少要给一个项目配时";
      for (const x of p.quotas) {
        if (!prefs.categories.some((c) => c.id === x.category)) throw "计划引用了不存在的项目";
        if (x.minutes < 0 || x.minutes > 24 * 60) throw "配额要在 0–24 小时之间";
      }
    }
    if (prefs.app_blocking) {
      const { apps } = prefs.app_blocking;
      if (apps.length > MAX_BLOCKED_APPS) throw "屏蔽的应用不能超过 200 个";
      if (apps.some((app) => !validPackageName(app.package_name))) throw "屏蔽名单里有不合法的应用包名";
      if (new Set(apps.map((app) => app.package_name)).size !== apps.length) throw "屏蔽名单里有重复的应用";
      if (apps.some((app) => !app.label.trim() || [...app.label.trim()].length > 80)) throw "屏蔽名单里的应用名为空、过长或含控制字符";
      // 与 core 一致：从没开过（关着、名单为空）就不出现在快照里。
      if (!prefs.app_blocking.enabled && !apps.length) delete prefs.app_blocking;
    }
    if (state.day && prefs.water_reminder_enabled !== state.preferences.water_reminder_enabled) state.day.seated_since_water = 0;
    state.preferences = structuredClone(prefs);
  },
};

export function validateHost(raw: string): string {
  const result = normalizeHost(raw);
  if ("error" in result) throw result.error;
  return result.host;
}

function validateUrl(raw: string): string {
  const result = normalizeUrl(raw);
  if ("error" in result) throw result.error;
  return result.url;
}

// ---------- 场景 ----------

function minutesAgo(now: number, minutes: number): number {
  return now - minutes * 60;
}

function midDay(now: number, prefs: Preferences): Day {
  const profile = prefs.profiles.find((p) => p.id === "standard")!;
  const day = dayFromProfile(prefs, profile, minutesAgo(now, 426));
  const m = (min: number) => minutesAgo(now, min);
  day.ledger = [
    { category: "deep", seconds: 90 * 60, accepted: true, completion_note: "", tasks: [task("把引言重写一遍", true), task("补图 1")], started_at: m(426), ended_at: m(336) },
    { category: "browse", seconds: 30 * 60, accepted: true, completion_note: "", tasks: [task("刷一圈论坛", true)], started_at: m(330), ended_at: m(300) },
    { category: "reading", seconds: 30 * 60, accepted: false, completion_note: "", tasks: [task("听力")], started_at: m(280), ended_at: m(250) },
    { category: "reading", seconds: 45 * 60, accepted: true, completion_note: "", tasks: [task("背 40 个单词", true), task("复习昨天的", true)], started_at: m(200), ended_at: m(155) },
    // 这一格中间被暂停了 20 分钟：墙钟 90 分钟，计入 70 分钟。
    { category: "deep", seconds: 70 * 60, accepted: true, completion_note: "", tasks: [], started_at: m(150), ended_at: m(60) },
  ];
  // 每个格之间的空档都得是一段暂停：一天没有缝。
  day.pauses = [
    { started_at: m(336), ended_at: m(330), auto: false },
    { started_at: m(300), ended_at: m(280), auto: false },
    { started_at: m(250), ended_at: m(200), auto: false },
    { started_at: m(155), ended_at: m(150), auto: false },
    // 这一段落在最后那格深度工作的中间：用来盯住「线不能倒着走」这个 bug。
    { started_at: m(120), ended_at: m(100), auto: false },
    { started_at: m(60), ended_at: null, auto: false },
  ];
  // 走完的格后面跟着休息（放弃的那格不给休息）；休息照样落在上面那几段暂停里。
  day.rests = [
    { started_at: m(336), ended_at: m(331) },
    { started_at: m(300), ended_at: m(290) },
    { started_at: m(155), ended_at: m(150) },
    { started_at: m(60), ended_at: m(50) },
  ];
  const done: Record<string, number> = { deep: 160 * 60, browse: 30 * 60, reading: 45 * 60 };
  for (const c of day.categories) c.accepted_seconds = done[c.id] ?? 0;
  day.seated_seconds = 265 * 60;
  day.paused_seconds = 161 * 60;
  day.cups = 5;
  day.seated_since_water = 20 * 60;
  day.seated_since_relief = 200 * 60;
  day.paused_without_block = 60 * 60;
  return day;
}

function focusTimer(now: number): NonNullable<Day["timer"]> {
  return {
    category: "reading",
    total_seconds: 45 * 60,
    elapsed_seconds: 25 * 60 + 54,
    tasks: [],
    started_at: minutesAgo(now, 26),
    break_minutes: 10,
  };
}

/** 示例也保持「专注 + 暂停 = 墙钟跨度」，避免用不可能的暂停顺序验收时间轴。 */
function attachTimer(day: Day, timer: NonNullable<Day["timer"]>, now: number, pausedSeconds = 0, auto = false) {
  const stoppedAt = now - pausedSeconds;
  timer.started_at = stoppedAt - timer.elapsed_seconds;
  day.pauses[day.pauses.length - 1].ended_at = timer.started_at;
  if (pausedSeconds > 0) day.pauses.push({ started_at: stoppedAt, ended_at: null, auto });
  day.seated_seconds += timer.elapsed_seconds;
  day.paused_seconds -= timer.elapsed_seconds;
  day.paused_without_block = 0;
  day.timer = timer;
}

function historyFixture(now: number, prefs: Preferences): ArchivedDay[] {
  const days: ArchivedDay[] = [];
  // 下标 i-1 是 i 天前那一天完成了目标的几成；0 表示那天没有坐。最近两周天天都坐了，
  // 更早的四周断断续续——历史页的月历墙上才看得到圆满、未满、没有记录和更早无记录四种格子。
  const ratios = [
    0.92, 0.78, 1.0, 0.85, 0.64, 0.95, 0.71, 0.83, 1.02, 0.6, 0.88, 0.97, 0.75, 0.9,
    1.0, 0.7, 0, 0.86, 1.03, 0.52, 0.94, 0, 0, 1.0, 0.81, 0.66, 1.01, 0.9,
    0, 0.73, 1.0, 0.58, 0.88, 0, 1.02, 0.79, 0.95, 0.6, 0, 0, 0.84, 1.0,
  ];
  for (let i = ratios.length; i >= 1; i--) {
    if (ratios[i - 1] === 0) continue;
    const start = new Date(now * 1000);
    start.setDate(start.getDate() - i);
    start.setHours(9, 5, 0, 0);
    const startedAt = Math.floor(start.getTime() / 1000);
    const endedAt = startedAt + (13 * 60 + 25) * 60;
    const profile = prefs.profiles[0];
    const day = dayFromProfile(prefs, profile, startedAt);
    const ratio = ratios[i - 1];
    let cursor = startedAt;
    for (const c of day.categories) {
      const def = prefs.categories.find((d) => d.id === c.id)!;
      let remaining = Math.round(c.quota_minutes * ratio) * 60;
      c.accepted_seconds = remaining;
      while (remaining > 0) {
        const seconds = Math.min(remaining, def.default_block_minutes * 60);
        day.ledger.push({ category: c.id, seconds, accepted: true, completion_note: "", tasks: [], started_at: cursor, ended_at: cursor + seconds });
        day.rests.push({ started_at: cursor + seconds, ended_at: cursor + seconds + 10 * 60 });
        cursor += seconds + 10 * 60;
        remaining -= seconds;
      }
    }
    if (i % 3 === 0) {
      day.ledger.push({ category: day.categories[0].id, seconds: 20 * 60, accepted: false, completion_note: "", tasks: [], started_at: cursor, ended_at: cursor + 20 * 60 });
    }
    // 格与格之间的空档都是暂停（休息也落在暂停里），收工前最后一段也是：一天没有缝，
    // 预览里复制某天的 Markdown，暂停合计和列出来的暂停才对得上。
    day.pauses = [];
    let edge = startedAt;
    for (const e of [...day.ledger].sort((a, b) => a.started_at - b.started_at)) {
      if (e.started_at > edge) day.pauses.push({ started_at: edge, ended_at: e.started_at, auto: false });
      edge = Math.max(edge, e.ended_at);
    }
    if (endedAt > edge) day.pauses.push({ started_at: edge, ended_at: endedAt, auto: false });
    day.seated_seconds = day.ledger.reduce((sum, e) => sum + e.seconds, 0);
    day.paused_seconds = endedAt - startedAt - day.seated_seconds;
    day.cups = 6 + (i % 3);
    days.push({ day, ended_at: endedAt });
  }
  return days;
}

function buildScenario(name: string): MockState {
  const now = nowUnix();
  const prefs = prefsFixture();
  const base: MockState = { schema: 5, last_tick: now, preferences: prefs, day: null, history: historyFixture(now, prefs) };
  switch (name) {
    case "nohistory":
      base.history = [];
      return base;
    case "fresh": {
      const profile = prefs.profiles.find((p) => p.id === "standard")!;
      base.day = dayFromProfile(prefs, profile, minutesAgo(now, 1));
      base.day.pauses = [{ started_at: minutesAgo(now, 1), ended_at: null, auto: false }];
      base.day.paused_seconds = 60;
      return base;
    }
    case "completed":
      base.day = midDay(now, prefs);
      base.day.ledger[base.day.ledger.length - 1].completion_note = null;
      return base;
    case "finishing":
      base.day = midDay(now, prefs);
      attachTimer(base.day, { ...focusTimer(now), elapsed_seconds: 45 * 60 - 2 }, now);
      return base;
    case "chooser":
      base.day = midDay(now, prefs);
      return base;
    case "running":
      base.day = midDay(now, prefs);
      attachTimer(base.day, focusTimer(now), now);
      return base;
    case "paused":
      // 格在走时被按停：走过的时间只到按下暂停那一刻。
      base.day = midDay(now, prefs);
      attachTimer(base.day, { ...focusTimer(now), elapsed_seconds: 14 * 60 }, now, 20 * 60);
      return base;
    case "suspended": {
      // 休眠 20 分钟：暂停段必须从合眼那一刻起算，运行图才不会把这 20 分钟画成在做事。
      // 手机按墙钟计时，不会出现心跳中断的自动暂停；预览手机时按普通的按停处理。
      base.day = midDay(now, prefs);
      attachTimer(base.day, { ...focusTimer(now), elapsed_seconds: 4 * 60 }, now, 20 * 60, !mobile);
      if (!mobile) base.day.suspend_seconds = 20 * 60;
      return base;
    }
    case "resting": {
      // 刚走完一格阅读、正在休息：休息从这一格结束那一刻算起，还剩 7 分钟。
      base.day = midDay(now, prefs);
      const day = base.day;
      const startedAt = minutesAgo(now, 48);
      const endedAt = minutesAgo(now, 3);
      day.pauses[day.pauses.length - 1].ended_at = startedAt;
      day.ledger.push({ category: "reading", seconds: 45 * 60, accepted: true, completion_note: "", tasks: [], started_at: startedAt, ended_at: endedAt });
      day.categories.find((c) => c.id === "reading")!.accepted_seconds += 45 * 60;
      day.seated_seconds += 45 * 60;
      day.paused_seconds -= 45 * 60;
      day.paused_without_block = 3 * 60;
      day.pauses.push({ started_at: endedAt, ended_at: null, auto: false });
      day.rests.push({ started_at: endedAt, ended_at: endedAt + 10 * 60 });
      day.break_until = endedAt + 10 * 60;
      return base;
    }
    case "done":
      base.day = midDay(now, prefs);
      for (const c of base.day.categories) c.accepted_seconds = c.quota_minutes * 60;
      return base;
    case "protected":
      writeProtected = "状态文件无法读取（unreadable: expected value at line 1 column 1），已进入保护模式：原文件保持原样，本次运行不会写盘。";
      return base;
    case "savefail":
      // 存盘写不进去：改动还在内存里，横条要在三页都看得见。
      base.day = midDay(now, prefs);
      base.day.timer = focusTimer(now);
      base.day.pauses[base.day.pauses.length - 1].ended_at = minutesAgo(now, 26);
      saveError = "No such file or directory (os error 2)";
      return base;
    default:
      return base;
  }
}

function scenarioName(): string {
  return params.get("qa") ?? "start";
}

state = buildScenario(scenarioName());
if (state.preferences.blocked_hosts.length && state.day) blocking.active = true;

// ---------- 对外 ----------

function snapshot(withHistory = true): Snapshot {
  const { history, ...live } = structuredClone(state);
  return {
    revision: ++snapshotRevision,
    state: live,
    history: withHistory ? history : null,
    write_protected: writeProtected,
    blocking: structuredClone(blocking),
    save_error: saveError,
    state_path: mobile ? "/data/user/0/com.snooze26h.sitzfleisch.x.debug/state.json" : "/Users/you/Library/Application Support/com.snooze26h.sitzfleisch.x/state.json",
    initial_view: null,
    initial_scroll: 0,
  };
}

function broadcast() {
  // 与真实外壳同一条规矩：心跳推送不带历史。
  const snap = snapshot(false);
  for (const listener of listeners) listener(snap);
}

function mutate(op: () => void): Snapshot {
  tick(nowUnix());
  op();
  broadcast();
  return snapshot();
}

export async function mockInvoke<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const a = args as Record<string, never>;
  await new Promise((resolve) => setTimeout(resolve, 8));
  switch (command) {
    case "system_status":
      if (!mobile) throw "系统状态查询仅支持 Android。";
      return structuredClone(window.qaSystemStatus ?? systemFixture) as T;
    case "open_system_settings": {
      const target = args.target;
      const channelId = args.channelId;
      if (typeof target !== "string" || !SYSTEM_SETTINGS_TARGETS.includes(target as SystemSettingsTarget)
        || (target === "channel" ? !NOTIFICATION_CHANNELS.includes(channelId as typeof NOTIFICATION_CHANNELS[number]) : channelId !== undefined)) {
        throw "请选择有效的系统设置入口和通知渠道。";
      }
      if (!mobile) throw "此系统设置入口仅支持 Android。";
      (window.qaSystemRequests ??= []).push({ target: target as SystemSettingsTarget, ...(typeof channelId === "string" ? { channelId } : {}) });
      return undefined as T;
    }
    case "installed_apps":
      if (!mobile) throw "应用屏蔽仅支持 Android。";
      await new Promise((resolve) => setTimeout(resolve, 400));
      return {
        apps: structuredClone(appListGranted ? installedFixture : installedFixture.slice(20)),
        limited: !appListGranted, canRequestFullList: true,
      } as T;
    case "request_app_list_permission":
      if (!mobile) throw "应用屏蔽仅支持 Android。";
      appListGranted = true;
      return true as T;
    case "take_block_notice": {
      const notice = mobile ? window.qaBlockNotice ?? null : null;
      window.qaBlockNotice = undefined;
      return notice as T;
    }
    case "move_task_to_back":
      if (!mobile) throw "退到后台仅支持 Android。";
      window.qaBackgrounded = (window.qaBackgrounded ?? 0) + 1;
      return undefined as T;
    case "platform_info":
      return structuredClone(platform) as T;
    case "get_snapshot":
      return snapshot() as T;
    case "start_day":
      return mutate(() => ops.start_day(a.profileId)) as T;
    case "switch_profile":
      return mutate(() => ops.switch_profile(a.profileId)) as T;
    case "start_block":
      return mutate(() => ops.start_block(a.categoryId, a.minutes, a.tasks ?? [], (a.breakMinutes as number | undefined) ?? 0)) as T;
    case "toggle_pause":
      return mutate(() => ops.toggle_pause(nowUnix())) as T;
    case "extend_block":
      return mutate(() => {
        if (a.timerStartedAt !== undefined && state.day?.timer?.started_at !== a.timerStartedAt) throw "原来的计时已结束，请重新选择延长时间";
        ops.extend_block(a.minutes);
      }) as T;
    case "set_completion_note":
      return mutate(() => ops.set_completion_note(a.dayStartedAt, a.entryIndex, a.endedAt, a.note)) as T;
    case "finish_block":
      return mutate(() => ops.finish_block(nowUnix())) as T;
    case "end_break":
      return mutate(() => ops.end_break()) as T;
    case "abandon_block":
      return mutate(() => ops.abandon_block(nowUnix())) as T;
    case "end_day":
      return mutate(() => ops.end_day(nowUnix())) as T;
    case "abandon_day":
      return mutate(() => ops.abandon_day()) as T;
    case "update_preferences":
      return mutate(() => ops.update_preferences(a.prefs)) as T;
    case "delete_history_day":
      return mutate(() => ops.delete_history_day(a.startedAt)) as T;
    case "normalize_host":
      return validateHost(a.host) as T;
    case "normalize_url":
      return validateUrl(a.url) as T;
    case "retry_save":
      saveError = null;
      broadcast();
      return snapshot() as T;
    case "quit_after_save":
      // 浏览器里没有进程可退：写不进去就把失败原样端回对话框，写得进去就当已经退出。
      throw saveError ? "（mock）仍然写不进去" : "（mock）已退出";
    case "quit_without_saving":
    case "quit_leaving_blocking":
      throw "（mock）已退出";
    case "reapply_blocking":
      blocking.busy = true;
      broadcast();
      setTimeout(() => {
        blocking.busy = false;
        blocking.active = !!state.day && state.preferences.blocked_hosts.length > 0;
        blocking.error = null;
        broadcast();
      }, 900);
      return snapshot() as T;
    case "check_blocking":
      blocking.active = !!state.day && state.preferences.blocked_hosts.length > 0;
      blocking.error = null;
      broadcast();
      return snapshot() as T;
    case "notification_status":
      return notificationPermission as T;
    case "request_notification_permission":
      if (mobile) {
        window.qaPermissionRequests = (window.qaPermissionRequests ?? 0) + 1;
        notificationPermission = params.get("permission") === "denied" ? "denied" : "granted";
        (window.qaSystemStatus ??= structuredClone(systemFixture)).notificationsEnabled = notificationPermission === "granted";
      }
      return notificationPermission as T;
    case "open_notification_settings":
      return undefined as T;
    case "test_water_sound":
    case "test_notification":
      if (mobile) (window.qaTestNotifications ??= []).push(command === "test_water_sound" ? "water" : "timer");
      return undefined as T;
    case "reveal_state_file":
      return undefined as T;
    case "reveal_browser_extension":
      throw "浏览器预览无法打开扩展目录，请在坐功桌面应用中使用这个按钮。";
    case "browser_pairing_code":
    case "reset_browser_pairing":
      throw "浏览器预览无法生成配对码，请在坐功桌面应用中配对。";
    case "app_version":
      return `${version}（预览）` as T;
    case "autostart_status":
      return autostart as T;
    case "set_autostart":
      autostart = a.enabled;
      return autostart as T;
    default:
      throw `模拟后台不认识的命令：${command}`;
  }
}

export function mockSubscribe(callback: (snapshot: Snapshot) => void) {
  listeners.push(callback);
}

/**
 * 真机上「退出前保存失败」由外壳推事件触发；浏览器里没有退出这回事，
 * 所以把触发口挂到控制台：开发者工具里执行 `qaQuitBlocked()` 就能走一遍那个对话框。
 */
export function mockQuitBlocked(callback: (reason: string) => void) {
  window.qaQuitBlocked = () => callback(saveError ?? "No such file or directory (os error 2)");
}

export function mockQuitBlockingFailed(callback: (reason: string) => void) {
  window.qaQuitBlockingFailed = () => callback("（mock）系统授权被取消");
}

export function mockBackButton(callback: () => void | Promise<void>) {
  window.qaBackButton = callback;
}

window.setInterval(() => {
  tick(nowUnix());
  broadcast();
}, 1000);
