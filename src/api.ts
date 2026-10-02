// 与外壳的唯一通道。在 Tauri 里走 invoke / event；在普通浏览器里（开发与截图验收）
// 换成 dev/mock 的内存实现，界面代码对此毫无感知。

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PlatformInfo, Snapshot } from "./types";

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
const PLATFORM_FEATURES: (keyof PlatformInfo["features"])[] = [
  "tray", "website_blocking", "browser_extension", "autostart", "reveal_state_file",
  "window_title", "quit_flow", "in_app_sound_toggle", "system_settings", "exact_alarm_status",
];

function validPlatformInfo(value: unknown): value is PlatformInfo {
  if (value === null || typeof value !== "object") return false;
  const info = value as Partial<PlatformInfo>;
  return typeof info.os === "string" && /^[a-z][a-z0-9_-]{0,31}$/.test(info.os)
    && typeof info.mobile === "boolean" && info.features !== null && typeof info.features === "object"
    && PLATFORM_FEATURES.every((feature) => typeof info.features?.[feature] === "boolean");
}

/** 能力跟着外壳走，窗口变窄不代表它变成了手机。 */
export function platformInfo(): Promise<PlatformInfo> {
  return (platformPromise ??= invoke<unknown>("platform_info").then((info) => {
    if (!validPlatformInfo(info)) throw new Error("平台能力数据格式无效");
    document.documentElement.dataset.platform = info.os;
    return info;
  }));
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
