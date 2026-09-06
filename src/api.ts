// 与外壳的唯一通道。在 Tauri 里走 invoke / event；在普通浏览器里（开发与截图验收）
// 换成 dev/mock 的内存实现，界面代码对此毫无感知。

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Snapshot } from "./types";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let mockModule: Promise<typeof import("./dev/mock")> | null = null;
function mock() {
  return (mockModule ??= import("./dev/mock"));
}

export async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) return tauriInvoke<T>(command, args);
  return (await mock()).mockInvoke<T>(command, args);
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
