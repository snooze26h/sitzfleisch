// 实时状态和历史分别按快照序号合并，防止迟到的推送覆盖刚保存的内容。

import type { ArchivedDay, Snapshot } from "./types";

export function mergeSnapshot(current: Snapshot | null, history: ArchivedDay[], historyRevision: number, incoming: Snapshot) {
  const snapshot = !current || incoming.revision > current.revision ? incoming : current;
  if (incoming.history !== null && incoming.revision > historyRevision) {
    return { snapshot, history: incoming.history, historyRevision: incoming.revision };
  }
  return { snapshot, history, historyRevision };
}
