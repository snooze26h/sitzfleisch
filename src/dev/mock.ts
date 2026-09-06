// 浏览器里的模拟后台：把 core 的规则用 TS 复刻一遍，让界面能在没有 Tauri 的地方
// 跑起来、点起来、截图验收。只在非 Tauri 环境被动态加载，不进正式包的主路径。
// 场景通过 URL hash 选：#qa=start|fresh|chooser|running|paused|break|done|protected|savefail|nohistory

import type {
  AppState,
  ArchivedDay,
  CategoryState,
  Day,
  Preferences,
  ProfileDef,
  Snapshot,
  TaskItem,
} from "../types";
import { MAX_BLOCK_MINUTES, MIN_BLOCK_MINUTES, SUSPEND_GAP_SECONDS } from "../types";

const HISTORY_LIMIT = 60;

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

// ---------- 出厂计划（用户真实的项目与档位） ----------

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
      { id: "minimum", name: "保底", subtitle: "有突发事情的日子，也守得住的下限", quotas: [q("deep", 120), q("browse", 0), q("reading", 240), q("practice", 60), q("move", 60)] },
      { id: "standard", name: "标准", subtitle: "大部分日子的默认目标，留得住余量", quotas: [q("deep", 240), q("browse", 30), q("reading", 180), q("practice", 0), q("move", 60)] },
      { id: "sprint", name: "冲刺", subtitle: "状态好时的满配", quotas: [q("deep", 300), q("browse", 30), q("reading", 240), q("practice", 120), q("move", 60)] },
    ],
    break_minutes: 10,
    hydration_goal_cups: 8,
    water_reminder_enabled: true,
    water_reminder_minutes: 45,
    stretch_reminder_enabled: true,
    stretch_reminder_minutes: 50,
    idle_reminder_enabled: true,
    idle_reminder_minutes: 15,
    blocked_hosts: ["weibo.com", "bilibili.com"],
    uniform_block_minutes: 0,
    default_profile_id: "standard",
    sound_enabled: true,
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
  }
}

let state: MockState;
let snapshotRevision = 0;
let writeProtected: string | null = null;
let saveError: string | null = null;
const blocking = { active: false, busy: false, error: null as string | null };
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
  });
}

function tick(now: number) {
  const delta = now - state.last_tick;
  state.last_tick = now;
  if (delta <= 0) return;
  const day = state.day;
  if (!day) return;
  if (delta > SUSPEND_GAP_SECONDS) {
    // 与 core 同一条规矩：空档一秒不补，但暂停段从空档开始那一刻起算。
    day.suspend_seconds += delta;
    day.paused_seconds += delta;
    beginPause(day, now - delta, true);
    return;
  }
  if (paused(day)) {
    day.paused_seconds += delta;
    if (!day.timer) day.paused_without_block += delta;
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
  day.seated_since_water += worked;
  day.seated_since_relief += worked;
  if (timer.elapsed_seconds < timer.total_seconds) return;
  const endedAt = now - overflow;
  day.timer = null;
  const rest = (timer.break_minutes > 0 ? timer.break_minutes : state.preferences.break_minutes) * 60;
  record(day, timer, true, endedAt);
  beginPause(day, endedAt, false);
  day.paused_seconds += overflow;
  day.paused_without_block = overflow;
  day.break_until = rest > 0 ? endedAt + rest : null;
}

function needDay(): Day {
  if (!state.day) throw "今天还没开始";
  return state.day;
}

const ops = {
  start_day(profileId: string) {
    if (state.day) throw "今天已经开始了";
    const profile = state.preferences.profiles.find((p) => p.id === profileId);
    if (!profile) throw "没有这个档位";
    const day = dayFromProfile(state.preferences, profile, state.last_tick);
    if (!day.categories.length) throw "这个档位没有任何配了时的项目";
    beginPause(day, state.last_tick, false);
    state.day = day;
  },
  switch_profile(profileId: string) {
    const profile = state.preferences.profiles.find((p) => p.id === profileId);
    if (!profile) throw "没有这个档位";
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
    if (!categories.length) throw "这个档位没有任何配了时的项目";
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
    day.break_until = null;
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
    const day = needDay();
    if (day.timer) throw "已经有一格在走";
    if (!day.categories.some((c) => c.id === categoryId)) throw "今天没有这个项目";
    day.break_until = null;
    closePause(day, state.last_tick);
    const clamped = Math.min(MAX_BLOCK_MINUTES, Math.max(MIN_BLOCK_MINUTES, minutes));
    day.timer = {
      category: categoryId,
      total_seconds: clamped * 60,
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
      day.break_until = null;
      day.seated_since_relief = 0;
    } else {
      beginPause(day, now, false);
    }
  },
  extend_block(minutes: number) {
    const timer = state.day?.timer;
    if (!timer) throw "没有在走的计时";
    const total = Math.min(MAX_BLOCK_MINUTES * 2, timer.total_seconds / 60 + Math.max(1, minutes));
    timer.total_seconds = total * 60;
  },
  finish_block(now: number) {
    const day = needDay();
    const timer = day.timer;
    if (!timer) throw "没有在走的计时";
    day.timer = null;
    const rest = (timer.break_minutes > 0 ? timer.break_minutes : state.preferences.break_minutes) * 60;
    record(day, timer, true, now);
    beginPause(day, now, false);
    day.paused_without_block = 0;
    day.break_until = rest > 0 ? now + rest : null;
  },
  abandon_block(now: number) {
    const day = needDay();
    const timer = day.timer;
    if (!timer) throw "没有在走的计时";
    day.timer = null;
    record(day, timer, false, now);
    beginPause(day, now, false);
    day.paused_without_block = 0;
    day.break_until = null;
  },
  end_break() {
    const day = needDay();
    if (day.break_until === null) throw "现在不在休息";
    day.break_until = null;
  },
  toggle_task(index: number) {
    const timer = state.day?.timer;
    if (!timer) throw "没有在走的计时";
    const item = timer.tasks[index];
    if (!item) throw "没有这条任务";
    item.done = !item.done;
  },
  add_task(text: string) {
    const trimmed = text.trim();
    if (!trimmed) throw "任务不能为空";
    const timer = state.day?.timer;
    if (!timer) throw "没有在走的计时";
    timer.tasks.push({ text: trimmed, done: false });
  },
  drink_water() {
    const day = needDay();
    day.cups += 1;
    day.seated_since_water = 0;
  },
  undo_water() {
    const day = needDay();
    if (day.cups <= 0) throw "今天还没记过水";
    day.cups -= 1;
  },
  update_preferences(prefs: Preferences) {
    if (!prefs.categories.length) throw "至少要有一个项目";
    if (!prefs.profiles.length) throw "至少要有一个档位";
    if (prefs.break_minutes < 0 || prefs.break_minutes > 120) throw "休息时长要在 0–120 分钟之间";
    if (prefs.hydration_goal_cups < 1 || prefs.hydration_goal_cups > 24) throw "喝水目标要在 1–24 杯之间";
    for (const m of [prefs.water_reminder_minutes, prefs.stretch_reminder_minutes, prefs.idle_reminder_minutes]) {
      if (m < 5 || m > 240) throw "提醒间隔要在 5–240 分钟之间";
    }
    for (const c of prefs.categories) {
      if (!c.id.trim() || !c.name.trim()) throw "项目名不能为空";
      if (c.default_block_minutes < MIN_BLOCK_MINUTES || c.default_block_minutes > MAX_BLOCK_MINUTES) throw "默认块时长要在 5–180 分钟之间";
    }
    for (const p of prefs.profiles) {
      if (!p.name.trim()) throw "档位名不能为空";
      if (!p.quotas.some((x) => x.minutes > 0)) throw "每个档位至少要给一个项目配时";
      for (const x of p.quotas) {
        if (!prefs.categories.some((c) => c.id === x.category)) throw "档位引用了不存在的项目";
        if (x.minutes < 0 || x.minutes > 24 * 60) throw "配额要在 0–24 小时之间";
      }
    }
    state.preferences = structuredClone(prefs);
  },
};

export function validateHost(raw: string): string {
  let host = raw.trim().toLowerCase();
  host = host.replace(/^https?:\/\//, "").replace(/\/+$/, "");
  if (!host) throw "域名不能为空";
  if (/[/ :]/.test(host)) throw "只要域名本身，不要带路径或端口";
  if (!host.includes(".") || host.startsWith(".") || host.endsWith(".")) throw "这不像一个域名";
  if (!/^[a-z0-9.-]+$/.test(host)) throw "域名只能包含字母、数字、点和连字符";
  return host;
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
    { category: "deep", seconds: 90 * 60, accepted: true, tasks: [task("把引言重写一遍", true), task("补图 1")], started_at: m(426), ended_at: m(336) },
    { category: "browse", seconds: 30 * 60, accepted: true, tasks: [task("刷一圈论坛", true)], started_at: m(330), ended_at: m(300) },
    { category: "reading", seconds: 30 * 60, accepted: false, tasks: [task("听力")], started_at: m(280), ended_at: m(250) },
    { category: "reading", seconds: 45 * 60, accepted: true, tasks: [task("背 40 个单词", true), task("复习昨天的", true)], started_at: m(200), ended_at: m(155) },
    // 这一格中间被暂停了 20 分钟：墙钟 90 分钟，计入 70 分钟。
    { category: "deep", seconds: 70 * 60, accepted: true, tasks: [], started_at: m(150), ended_at: m(60) },
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
    tasks: [task("听力 20 分钟", true), task("阅读 20 分钟"), task("背单词")],
    started_at: minutesAgo(now, 26),
    break_minutes: 10,
  };
}

function historyFixture(now: number, prefs: Preferences): ArchivedDay[] {
  const days: ArchivedDay[] = [];
  const ratios = [0.92, 0.78, 1.0, 0.85, 0.64, 0.95, 0.71, 0.83, 1.02, 0.6, 0.88, 0.97, 0.75, 0.9];
  const profileIds = ["standard", "sprint", "standard", "minimum", "standard", "sprint", "standard"];
  for (let i = 14; i >= 1; i--) {
    const start = new Date(now * 1000);
    start.setDate(start.getDate() - i);
    start.setHours(9, 5, 0, 0);
    const startedAt = Math.floor(start.getTime() / 1000);
    const endedAt = startedAt + (13 * 60 + 25) * 60;
    const profile = prefs.profiles.find((p) => p.id === profileIds[i % profileIds.length])!;
    const day = dayFromProfile(prefs, profile, startedAt);
    const ratio = ratios[i - 1];
    let cursor = startedAt;
    for (const c of day.categories) {
      const def = prefs.categories.find((d) => d.id === c.id)!;
      let remaining = Math.round(c.quota_minutes * ratio) * 60;
      c.accepted_seconds = remaining;
      while (remaining > 0) {
        const seconds = Math.min(remaining, def.default_block_minutes * 60);
        day.ledger.push({ category: c.id, seconds, accepted: true, tasks: [], started_at: cursor, ended_at: cursor + seconds });
        cursor += seconds + 10 * 60;
        remaining -= seconds;
      }
    }
    if (i % 3 === 0) {
      day.ledger.push({ category: day.categories[0].id, seconds: 20 * 60, accepted: false, tasks: [], started_at: cursor, ended_at: cursor + 20 * 60 });
    }
    day.pauses = [
      { started_at: startedAt + 3 * 3600 + 25 * 60, ended_at: startedAt + 4 * 3600 + 5 * 60, auto: false },
      { started_at: startedAt + 9 * 3600 + 25 * 60, ended_at: startedAt + 10 * 3600 + 5 * 60, auto: false },
    ];
    day.seated_seconds = day.categories.reduce((sum, c) => sum + c.accepted_seconds, 0);
    day.paused_seconds = endedAt - startedAt - day.seated_seconds;
    day.cups = 6 + (i % 3);
    days.push({ day, ended_at: endedAt });
  }
  return days;
}

function buildScenario(name: string): MockState {
  const now = nowUnix();
  const prefs = prefsFixture();
  const base: MockState = { schema: 3, last_tick: now, preferences: prefs, day: null, history: historyFixture(now, prefs) };
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
    case "chooser":
      base.day = midDay(now, prefs);
      return base;
    case "running":
      base.day = midDay(now, prefs);
      base.day.timer = focusTimer(now);
      // 开格就把最后那段暂停收口。
      base.day.pauses[base.day.pauses.length - 1].ended_at = minutesAgo(now, 26);
      return base;
    case "paused":
      // 格在走时被按停：走过的时间只到按下暂停那一刻。
      base.day = midDay(now, prefs);
      base.day.timer = { ...focusTimer(now), elapsed_seconds: 14 * 60 };
      return base;
    case "suspended": {
      // 休眠 20 分钟：暂停段必须从合眼那一刻起算，运行图才不会把这 20 分钟画成在做事。
      base.day = midDay(now, prefs);
      base.day.timer = { ...focusTimer(now), elapsed_seconds: 4 * 60 };
      const last = base.day.pauses[base.day.pauses.length - 1];
      last.auto = true;
      last.started_at = minutesAgo(now, 20);
      base.day.suspend_seconds = 20 * 60;
      base.day.paused_seconds += 20 * 60;
      return base;
    }
    case "resting":
      base.day = midDay(now, prefs);
      base.day.break_until = now + 7 * 60;
      return base;
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
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
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
    blocking: { ...blocking },
    save_error: saveError,
    state_path: "/Users/you/Library/Application Support/com.snooze26h.sitzfleisch.x/state.json",
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
    case "get_snapshot":
      return snapshot() as T;
    case "start_day":
      return mutate(() => ops.start_day(a.profileId)) as T;
    case "switch_profile":
      return mutate(() => ops.switch_profile(a.profileId)) as T;
    case "start_block":
      return mutate(() => ops.start_block(a.categoryId, a.minutes, a.tasks ?? [], (a.breakMinutes as number | undefined) ?? 0)) as T;
    case "toggle_task":
      return mutate(() => ops.toggle_task(a.index)) as T;
    case "add_task":
      return mutate(() => ops.add_task(a.text)) as T;
    case "toggle_pause":
      return mutate(() => ops.toggle_pause(nowUnix())) as T;
    case "extend_block":
      return mutate(() => ops.extend_block(a.minutes)) as T;
    case "finish_block":
      return mutate(() => ops.finish_block(nowUnix())) as T;
    case "end_break":
      return mutate(() => ops.end_break()) as T;
    case "abandon_block":
      return mutate(() => ops.abandon_block(nowUnix())) as T;
    case "drink_water":
      return mutate(() => ops.drink_water()) as T;
    case "undo_water":
      return mutate(() => ops.undo_water()) as T;
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
    case "retry_save":
      saveError = null;
      broadcast();
      return snapshot() as T;
    case "quit_after_save":
      // 浏览器里没有进程可退：写不进去就把失败原样端回对话框，写得进去就当已经退出。
      throw saveError ? "（mock）仍然写不进去" : "（mock）已退出";
    case "quit_without_saving":
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
      return "granted" as T;
    case "request_notification_permission":
      return "granted" as T;
    case "open_notification_settings":
      return undefined as T;
    case "test_notification":
      return undefined as T;
    case "reveal_state_file":
      return undefined as T;
    case "app_version":
      return "0.8.1-mock" as unknown as T;
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

window.setInterval(() => {
  tick(nowUnix());
  broadcast();
}, 1000);
