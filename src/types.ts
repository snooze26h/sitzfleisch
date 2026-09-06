// 与 core 的 serde 输出一一对应的类型。字段名保持 snake_case，和 Rust 端同名。

export interface CategoryDef {
  id: string;
  name: string;
  short_name: string;
  default_block_minutes: number;
  icon: string;
  role: string;
  block_rationale: string;
}

export interface ProfileQuota {
  category: string;
  minutes: number;
}

export interface ProfileDef {
  id: string;
  name: string;
  subtitle: string;
  quotas: ProfileQuota[];
}

export interface Preferences {
  categories: CategoryDef[];
  profiles: ProfileDef[];
  break_minutes: number;
  hydration_goal_cups: number;
  water_reminder_enabled: boolean;
  water_reminder_minutes: number;
  stretch_reminder_enabled: boolean;
  stretch_reminder_minutes: number;
  idle_reminder_enabled: boolean;
  idle_reminder_minutes: number;
  blocked_hosts: string[];
  uniform_block_minutes: number;
  default_profile_id: string;
  sound_enabled: boolean;
}

export interface TaskItem {
  text: string;
  done: boolean;
}

export interface BlockTimer {
  category: string;
  total_seconds: number;
  elapsed_seconds: number;
  tasks: TaskItem[];
  started_at: number;
  break_minutes: number;
}

export interface CategoryState {
  id: string;
  name: string;
  quota_minutes: number;
  accepted_seconds: number;
}

export interface LedgerEntry {
  category: string;
  seconds: number;
  accepted: boolean;
  tasks: TaskItem[];
  started_at: number;
  ended_at: number;
}

/** 一段暂停：手动按的，或休眠、锁屏自动判定的（auto）。 */
export interface PauseSpan {
  started_at: number;
  ended_at: number | null;
  auto: boolean;
}

export interface Day {
  profile_name: string;
  profile_id: string;
  started_at: number;
  categories: CategoryState[];
  timer: BlockTimer | null;
  ledger: LedgerEntry[];
  seated_seconds: number;
  paused_seconds: number;
  suspend_seconds: number;
  cups: number;
  seated_since_water: number;
  seated_since_relief: number;
  paused_without_block: number;
  /** 休息到几点。休息只是一段带截止时刻的暂停。 */
  break_until: number | null;
  pauses: PauseSpan[];
}

export interface ArchivedDay {
  day: Day;
  ended_at: number;
}

export interface BlockingStatus {
  active: boolean;
  busy: boolean;
  error: string | null;
}

/** 每秒推过来的那部分：偏好 + 今天。历史另走一条路，见 `Snapshot.history`。 */
export interface AppState {
  schema: number;
  last_tick: number;
  preferences: Preferences;
  day: Day | null;
}

export interface Snapshot {
  revision: number;
  state: AppState;
  /**
   * 历史满 60 天有 270 KB 上下，每秒推一遍是白烧电。
   * 心跳推送里是 null，界面沿用上一次的（见 `applySnapshot`）；
   * 首次拉取和每个命令的返回值一定带上。
   */
  history: ArchivedDay[] | null;
  write_protected: string | null;
  blocking: BlockingStatus;
  /** 上一次保存失败的系统原因，成功一次就变回 null。只走 IPC，不进 state.json。 */
  save_error: string | null;
  state_path: string;
  /** QA 启动参数指定的首屏；平时为空。 */
  initial_view?: string | null;
  initial_scroll?: number;
}

export type View = "today" | "history" | "settings";

/** 调度性质：id → 界面名。顺序即设置页里的展示顺序。 */
export const ROLES: [string, string, string][] = [
  ["general", "普通", "按剩余比例参与下一格排序。"],
  ["deepWork", "深度优先", "一天刚开始时更优先。"],
  ["exploration", "容易滑走", "块长压得短一些，逼你按时收口。"],
  ["dailyFloor", "每日底线", "缺口占比过大时优先守住。"],
  ["movement", "运动提醒", "连续久坐过久时优先提醒。"],
];

export function roleLabel(role: string): string {
  return ROLES.find(([id]) => id === role)?.[1] ?? "普通";
}

/** 设置页图标选择器：Lucide 名 → 中文名。 */
export const ICON_NAMED: [string, string][] = [
  ["flask-conical", "烧瓶"],
  ["compass", "指南针"],
  ["languages", "语言"],
  ["binary", "代码"],
  ["dumbbell", "锻炼"],
  ["ruler", "尺子"],
  ["activity", "活动"],
  ["heart-pulse", "健康"],
  ["chart-column", "图表"],
  ["graduation-cap", "学习"],
  ["timer", "计时"],
  ["book-open", "书本"],
  ["pen-line", "写作"],
  ["newspaper", "资讯"],
  ["message-square", "聊天"],
  ["globe", "网页"],
  ["code", "代码"],
  ["brain", "思考"],
  ["music", "音乐"],
  ["coffee", "杯子"],
  ["bike", "骑行"],
  ["leaf", "叶子"],
  ["mountain", "山"],
];

export const MIN_BLOCK_MINUTES = 5;
export const MAX_BLOCK_MINUTES = 180;
export const SUSPEND_GAP_SECONDS = 120;
