// 与 core 的 serde 输出一一对应的类型。字段名保持 snake_case，和 Rust 端同名。

/** 启动时查询一次的平台能力，不随每秒快照重复推送。 */
export interface PlatformInfo {
  os: string;
  mobile: boolean;
  features: {
    tray: boolean;
    website_blocking: boolean;
    browser_extension: boolean;
    autostart: boolean;
    reveal_state_file: boolean;
    window_title: boolean;
    quit_flow: boolean;
    in_app_sound_toggle: boolean;
    system_settings: boolean;
    exact_alarm_status: boolean;
    app_blocking: boolean;
  };
}

export const NOTIFICATION_CHANNELS = ["timer", "body", "water", "status"] as const;
export const SYSTEM_SETTINGS_TARGETS = ["app_notifications", "channel", "exact_alarm", "battery", "app_details", "accessibility", "startup"] as const;
export type SystemSettingsTarget = typeof SYSTEM_SETTINGS_TARGETS[number];

/** 与外壳 platform::SystemStatus 的 camelCase 输出对应，独立于计时快照。 */
export interface SystemStatus {
  sdkInt: number;
  manufacturer: string;
  notificationsEnabled: boolean;
  channels: {
    id: typeof NOTIFICATION_CHANNELS[number];
    name: string;
    enabled: boolean;
    importance: number;
    vibration: boolean;
    sound: string | null;
  }[];
  canScheduleExactAlarms: boolean;
  ignoringBatteryOptimizations: boolean;
  /** 坐功的无障碍服务在系统里开着没有；应用屏蔽靠它生效。 */
  appBlockServiceEnabled: boolean;
}

/** 应用选择器里的一项：手机上能从桌面打开的应用（外壳 platform::InstalledApp）。 */
export interface InstalledApp {
  packageName: string;
  label: string;
}

export interface InstalledApps {
  apps: InstalledApp[];
  /** 厂商的「获取应用列表」权限没给，系统只交出了一部分应用。 */
  limited: boolean;
  canRequestFullList: boolean;
}

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
  blocked_urls: string[];
  uniform_block_minutes: number;
  default_profile_id: string;
  sound_enabled: boolean;
  /** 手机上的应用屏蔽；从没开过时外壳不发这个字段。 */
  app_blocking?: AppBlocking;
}

/** 手动开关的应用屏蔽，与学习日无关。 */
export interface AppBlocking {
  enabled: boolean;
  apps: BlockedApp[];
}

export interface BlockedApp {
  package_name: string;
  /** 选中那一刻系统给的应用名，只用来显示。 */
  label: string;
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
  completion_note: string | null;
}

/** 一段暂停：手动按的，或休眠、锁屏自动判定的（auto）。 */
export interface PauseSpan {
  started_at: number;
  ended_at: number | null;
  auto: boolean;
}

/** 一段休息：结束一格后按设定休息的那段时间；提前结束就截到那一刻。休息本身也落在暂停里。 */
export interface RestSpan {
  started_at: number;
  ended_at: number;
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
  /** 今天每一段休息的起止：休息结束后，时间轴和读数还认得出这段是休息。 */
  rests: RestSpan[];
}

export interface ArchivedDay {
  day: Day;
  ended_at: number;
}

export interface BlockingStatus {
  active: boolean;
  busy: boolean;
  error: string | null;
  browser: {
    available: boolean;
    connected: boolean;
    synced: boolean;
    supports_hosts: boolean;
    error: string | null;
  };
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


/** 通用图标目录；专属项目图标优先展示，仍兼容这里的旧标识。 */
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

export const MIN_BLOCK_MINUTES = 1;
export const MAX_BLOCK_MINUTES = 180;
export const SUSPEND_GAP_SECONDS = 120;
export const MAX_COMPLETION_NOTE_CHARS = 2000;
