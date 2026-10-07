// 与外壳的唯一通道。在 Tauri 里走 invoke / event；在普通浏览器里（开发与截图验收）
// 换成 dev/mock 的内存实现，界面代码对此毫无感知。

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { InstalledApps, PlatformInfo, Snapshot, SystemSettingsTarget, SystemStatus } from "./types";
import { NOTIFICATION_CHANNELS, SYSTEM_SETTINGS_TARGETS } from "./types";
import { validPackageName } from "./app-blocking";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let mockModule: Promise<typeof import("./dev/mock")> | null = null;
function mock() {
  return (mockModule ??= import("./dev/mock"));
}

export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) return tauriInvoke<T>(command, args);
  return (await mock()).mockInvoke<T>(command, args);
}

let platformPromise: Promise<PlatformInfo> | null = null;
type Feature = keyof PlatformInfo["features"];
const DESKTOP_FEATURES: Feature[] = [
  "tray", "website_blocking", "browser_extension", "autostart", "reveal_state_file",
  "window_title", "quit_flow", "in_app_sound_toggle",
];
const ANDROID_FEATURES: Feature[] = ["system_settings", "exact_alarm_status", "app_blocking"];

/**
 * 外壳给的能力表缺键、格式不对或干脆取不到时，按平台推出与外壳一致的默认值，
 * 而不是让整个界面停在「无法启动」：能力表只决定显示哪些入口，不值得为它拒绝启动。
 */
function normalizePlatformInfo(value: unknown): PlatformInfo {
  const raw = (value !== null && typeof value === "object" ? value : {}) as Partial<PlatformInfo>;
  const os = typeof raw.os === "string" && /^[a-z][a-z0-9_-]{0,31}$/.test(raw.os)
    ? raw.os
    : /Android/i.test(navigator.userAgent) ? "android" : "unknown";
  const mobile = typeof raw.mobile === "boolean" ? raw.mobile : os === "android" || os === "ios";
  const reported = raw.features !== null && typeof raw.features === "object" ? raw.features : undefined;
  const fallback = (feature: Feature) => DESKTOP_FEATURES.includes(feature) ? !mobile : os === "android";
  const features = Object.fromEntries([...DESKTOP_FEATURES, ...ANDROID_FEATURES].map((feature) => {
    const value = reported?.[feature];
    return [feature, typeof value === "boolean" ? value : fallback(feature)];
  })) as PlatformInfo["features"];
  return { os, mobile, features };
}

/** 能力跟着外壳走，窗口变窄不代表它变成了手机。 */
export function platformInfo(): Promise<PlatformInfo> {
  return (platformPromise ??= invoke<unknown>("platform_info").catch(() => null).then((value) => {
    const info = normalizePlatformInfo(value);
    document.documentElement.dataset.platform = info.os;
    return info;
  }));
}

function validSystemStatus(value: unknown): value is SystemStatus {
  if (value === null || typeof value !== "object") return false;
  const s = value as Partial<SystemStatus>;
  return Number.isInteger(s.sdkInt) && s.sdkInt! >= 1 && s.sdkInt! <= 1000
    && typeof s.manufacturer === "string" && s.manufacturer.length <= 128
    && typeof s.notificationsEnabled === "boolean" && typeof s.canScheduleExactAlarms === "boolean"
    && typeof s.ignoringBatteryOptimizations === "boolean" && typeof s.appBlockServiceEnabled === "boolean"
    && Array.isArray(s.channels) && s.channels.length <= 16
    && s.channels.every((c) => c !== null && typeof c === "object" && NOTIFICATION_CHANNELS.includes(c.id)
      && typeof c.name === "string" && c.name.length <= 128 && typeof c.enabled === "boolean"
      && Number.isInteger(c.importance) && c.importance >= -1000 && c.importance <= 1000 && typeof c.vibration === "boolean"
      && (c.sound === null || (typeof c.sound === "string" && c.sound.length <= 2048)))
    && new Set(s.channels.map((c) => c.id)).size === s.channels.length;
}

export async function systemStatus(): Promise<SystemStatus> {
  const status = await invoke<unknown>("system_status");
  if (!validSystemStatus(status)) throw new Error("系统提醒状态数据格式无效");
  return status;
}

function validInstalledApps(value: unknown): value is InstalledApps {
  if (value === null || typeof value !== "object") return false;
  const list = value as Partial<InstalledApps>;
  return typeof list.limited === "boolean" && typeof list.canRequestFullList === "boolean"
    && Array.isArray(list.apps) && list.apps.length <= 1000
    && list.apps.every((app) => app !== null && typeof app === "object" && validPackageName(app.packageName)
      && typeof app.label === "string" && app.label.trim().length > 0 && app.label.length <= 80);
}

/** 应用屏蔽的选择器：手机上能从桌面打开的应用。 */
export async function installedApps(): Promise<InstalledApps> {
  const list = await invoke<unknown>("installed_apps");
  if (!validInstalledApps(list)) throw new Error("应用列表数据格式无效");
  return list;
}

/** 屏蔽服务刚把人送回坐功时，是因为哪个应用；没有就是 null。 */
export async function takeBlockNotice(): Promise<string | null> {
  const name = await invoke<unknown>("take_block_notice");
  return validPackageName(name) ? name : null;
}

export async function openSystemSettings(target: SystemSettingsTarget, channelId?: string): Promise<void> {
  if (!SYSTEM_SETTINGS_TARGETS.includes(target)
    || (target === "channel" ? !NOTIFICATION_CHANNELS.includes(channelId as typeof NOTIFICATION_CHANNELS[number]) : channelId !== undefined)) {
    throw new Error("请选择有效的系统设置入口和通知渠道");
  }
  await invoke("open_system_settings", { target, channelId });
}

export async function onSnapshot(callback: (snapshot: Snapshot) => void): Promise<void> {
  if (inTauri) {
    await listen<Snapshot>("state://update", (event) => callback(event.payload));
    return;
  }
  (await mock()).mockSubscribe(callback);
}

export interface Reminder {
  title: string;
  body: string;
  /** 系统通知是否真的发出去了；false 时界面要自己顶上。 */
  system: boolean;
}

export async function onReminder(callback: (reminder: Reminder) => void): Promise<void> {
  if (!inTauri) return;
  await listen<Reminder>("reminder://show", (event) => callback(event.payload));
}

export async function onExtendRequested(callback: () => void): Promise<void> {
  if (inTauri) await listen("timer://extend", callback);
}

export async function onBackRequested(callback: () => void | Promise<void>): Promise<void> {
  if (inTauri) {
    const { onBackButtonPress } = await import("@tauri-apps/api/app");
    // 注册后原生会把返回键交给界面，按界面层级退回，不走 WebView 历史。
    await onBackButtonPress(() => { void callback(); });
    return;
  }
  (await mock()).mockBackButton(callback);
}

/**
 * 退出前那次保存没写进去：外壳不退出，把原因推过来让界面问用户。
 * 浏览器里没有进程可退，触发口由 mock 挂到控制台（`qaQuitBlocked()`）。
 */
export async function onQuitBlocked(callback: (reason: string) => void): Promise<void> {
  if (inTauri) {
    await listen<string>("save://quit-blocked", (event) => callback(event.payload));
    return;
  }
  (await mock()).mockQuitBlocked(callback);
}

export async function onQuitBlockingFailed(callback: (reason: string) => void): Promise<void> {
  if (inTauri) {
    await listen<string>("blocking://quit-blocked", (event) => callback(event.payload));
    return;
  }
  (await mock()).mockQuitBlockingFailed(callback);
}

export async function setWindowTitle(title: string): Promise<void> {
  if (!inTauri) {
    document.title = title;
    return;
  }
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTitle(title);
  } catch {
    // 没拿到窗口权限就算了，标题不是功能。
  }
}
