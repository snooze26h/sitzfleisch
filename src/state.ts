// 界面自己的状态（不落盘）：当前页、草稿、展开项、菜单与对话框。

import type { ArchivedDay, CategoryDef, CategoryState, Day, Preferences, Snapshot, TaskItem, View } from "./types";
import { nowUnix, shortNameFrom } from "./format";

/** 处理完这一下之后要不要留着框：返回 `"keep"` 就留着（重试还没成功），否则关掉。 */
export type DialogOutcome = void | "keep";

export interface Dialog {
  /** 这一次打开的唯一标识，`ask()` 自己递增，外部不传。await 之后靠它认「还是不是原来那个框」。 */
  token: number;
  /** 同一个 id 再来一次只更新正文，不叠新框。不填就是每次都新开一个。 */
  id?: string;
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel: string;
  destructive: boolean;
  /** 打开时把焦点放到哪个键；不填就维持现状。 */
  focus?: "confirm" | "cancel";
  /** 收到的是自己这一个实例：await 之后要认它，别去动期间换上来的新框。 */
  onConfirm: (self: Dialog) => DialogOutcome | Promise<DialogOutcome>;
  /** 主按钮之外的第三个动作（红色）。只有退出前保存失败用到。 */
  alt?: { label: string; onAlt: (self: Dialog) => DialogOutcome | Promise<DialogOutcome> };
}

/** 退出前保存失败那个框的 id。它要压过解除屏蔽的两步确认，所以得有个名字。 */
export const QUIT_DIALOG_ID = "quit-blocked";

export interface Removal {
  host: string;
  step: 1 | 2;
  typed: string;
}

export const ui = {
  view: "today" as View,
  snap: null as Snapshot | null,
  // 历史不在每秒的推送里，单独存一份，收到带历史的快照时才换。
  history: [] as ArchivedDay[],
  now: nowUnix(),
  // 今天
  selectedCategory: null as string | null,
  minutesDraft: null as number | null,
  breakDraft: null as number | null,
  taskDraft: "",
  addTaskDraft: "",
  addingTask: false,
  creditPulse: null as string | null,
  // 历史
  expandedDays: new Set<number>(),
  selectedHistoryDay: null as number | null,
  // 设置
  pendingPrefs: 0,
  expandedProject: null as string | null,
  hostDraft: "",
  notificationStatus: "checking",
  autostart: null as boolean | null,
  appVersion: null as string | null,
  removal: null as Removal | null,
  // 覆盖层
  dialog: null as Dialog | null,
  /** 正在忙的是哪一个对话框（token）；null = 不忙。跟着实例走，旧操作不会禁用新框。 */
  dialogBusy: null as number | null,
  menu: null as string | null,
  toast: null as { message: string } | null,
};

/**
 * 当前这个对话框正在跑一次异步动作吗。**显示与事件处理必须共用这一条判据**：
 * 只看 `ui.dialogBusy !== null` 的话，旧实例还在 pending 时新框会画成可点的样子，
 * 点下去却被拒——按钮看着能用、其实没反应。
 */
export function dialogIsBusy(): boolean {
  return ui.dialog !== null && ui.dialogBusy === ui.dialog.token;
}

/**
 * 最上层的覆盖层是哪一个。渲染顺序与键盘处理共用它，
 * 否则会出现「Esc 关掉了看不见的那一个」。
 */
export function topOverlay(): "dialog" | "removal" | null {
  // 退出被拦是外壳推过来、用户正等着的答复，压过解除屏蔽的两步确认。
  if (ui.dialog?.id === QUIT_DIALOG_ID) return "dialog";
  if (ui.removal) return "removal";
  if (ui.dialog) return "dialog";
  return null;
}

export function prefs(): Preferences {
  return ui.snap!.state.preferences;
}

export function day(): Day | null {
  return ui.snap?.state.day ?? null;
}

/** 归档的日子，新的在后面。心跳推送不带它，所以从这里读，不要读 `ui.snap`。 */
export function history(): ArchivedDay[] {
  return ui.history;
}

export function def(id: string): CategoryDef | undefined {
  return ui.snap?.state.preferences.categories.find((c) => c.id === id);
}

export function iconOf(id: string): string {
  return def(id)?.icon ?? "ruler";
}

export function nameOf(id: string, d: Day | null = day()): string {
  return d?.categories.find((c) => c.id === id)?.name ?? def(id)?.name ?? id;
}

/**
 * 侧栏、运行图、菜单栏上的短名。设置里填了就立刻用它——短名是显示偏好，
 * 和图标一样不进台账；为空才按显示名自动回退（学习日进行中，显示名是当天冻结的那个）。
 */
export function shortName(id: string, d: Day | null = day()): string {
  const definition = def(id);
  if (definition?.short_name.trim()) return definition.short_name.trim();
  return shortNameFrom(nameOf(id, d));
}

export function roleOf(id: string): string {
  return def(id)?.role ?? "general";
}

export function remainingOf(c: CategoryState): number {
  return Math.max(0, c.quota_minutes * 60 - c.accepted_seconds);
}

export function netSeconds(d: Day): number {
  return d.categories.reduce((sum, c) => sum + c.accepted_seconds, 0);
}

export function quotaSeconds(d: Day): number {
  return d.categories.reduce((sum, c) => sum + c.quota_minutes * 60, 0);
}

export function isPaused(d: Day): boolean {
  const last = d.pauses[d.pauses.length - 1];
  return !!last && last.ended_at === null;
}

/** 正在暂停的话，这一段已经暂停了多久。 */
export function pauseNowSeconds(d: Day): number {
  const last = d.pauses[d.pauses.length - 1];
  return last && last.ended_at === null ? Math.max(0, ui.now - last.started_at) : 0;
}

/** 正在休息：休息只是一段带截止时刻的暂停。 */
export function resting(d: Day): boolean {
  return d.break_until !== null && ui.now < d.break_until;
}

export function breakRemaining(d: Day): number {
  return d.break_until === null ? 0 : Math.max(0, d.break_until - ui.now);
}

/** 最后一段暂停是不是休眠/锁屏自动按的。 */
export function pausedAutomatically(d: Day): boolean {
  const last = d.pauses[d.pauses.length - 1];
  return !!last && last.ended_at === null && last.auto;
}

export function profileTotalMinutes(p: { quotas: { minutes: number }[] }): number {
  return p.quotas.reduce((sum, q) => sum + Math.max(0, q.minutes), 0);
}

export function defaultProfileId(): string {
  const p = prefs();
  return p.profiles.some((x) => x.id === p.default_profile_id) ? p.default_profile_id : (p.profiles[0]?.id ?? "");
}

/**
 * 精确域名的即时预览。规则与 `core::validate_host` 逐条对齐，但**它才是权威**：
 * 这里只负责在用户还在输入时先说一声，真正入库前一律回问 core。
 */
export function normalizeHost(raw: string): { host: string } | { error: string } {
  let host = raw.trim().toLowerCase();
  if (!host) return { error: "网址不能为空。" };
  if (host.includes("@")) return { error: "网址不能包含用户名或密码。" };
  const scheme = host.match(/^([a-z][a-z0-9+.-]*):\/\//);
  if (scheme && scheme[1] !== "http" && scheme[1] !== "https") return { error: "只支持 http 或 https 网址。" };
  host = host.replace(/^https?:\/\//, "");
  host = host.split(/[/?#]/)[0];
  host = host.replace(/:\d+$/, "");
  // 存储形态一律不带 www：写 hosts 时 core 会自己补上 www 那一份。
  host = host.replace(/^www\./, "");
  if (!host) return { error: "没有识别出安全、完整的域名。" };
  if (host === "localhost" || host.includes(":")) return { error: "不能添加 localhost 或 IP 地址。" };
  if (/[^\x00-\x7f]/.test(host)) return { error: "当前版本只接受英文域名；中文域名请先转换成 punycode。" };
  if (!host.includes(".") || host.startsWith(".") || host.endsWith(".") || !/^[a-z0-9.-]+$/.test(host)) {
    return { error: "没有识别出安全、完整的域名。" };
  }
  // 每一段都是纯数字就是 IP，与 core 同一条判据。
  if (host.split(".").every((label) => label.length > 0 && /^\d+$/.test(label))) {
    return { error: "不能添加 localhost 或 IP 地址。" };
  }
  return { host };
}

/** 一行一条，把输入框里的文字变成任务清单。 */
export function parseTasks(text: string): TaskItem[] {
  return text
    .split("\n")
    .map((line) => line.replace(/^\s*[-*·•]\s*/, "").trim())
    .filter(Boolean)
    .map((t) => ({ text: t, done: false }));
}

export const BLOCK_OPTIONS = [25, 30, 45, 60, 75, 90, 120];
export const BREAK_OPTIONS = [0, 5, 10, 15, 20];
export const IDLE_OPTIONS = [0, 10, 15, 30, 60];
export const MAX_PROJECTS = 12;
export const MAX_HOSTS = 64;
export const SITTING_ALARM = 3 * 3600;
export const HISTORY_LIMIT = 60;

export function withValue(options: number[], current: number): number[] {
  return options.includes(current) ? options : [...options, current].sort((a, b) => a - b);
}
