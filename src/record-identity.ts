import type { BlockTimer, Day } from "./types";

/** 两台设备可以同秒开工；有同步 ID 时，时间戳只能用于显示，不能再当主键。 */
export function studyDayKey(day: Day): string {
  return day.sync_id ? `sync-${day.sync_id}` : `time-${day.started_at}`;
}

export function completionEntryIndex(day: Day, entryId: string | undefined, index: number, endedAt: number): number {
  const found = entryId ? day.ledger.findIndex((entry) => entry.sync_id === entryId) : index;
  if (!Number.isSafeInteger(found) || found < 0) return -1;
  const entry = day.ledger[found];
  if (!entry?.accepted || entry.ended_at !== endedAt) return -1;
  // 旧弹窗不能在登录/迁移后按同一个数组下标落到另一条共享记录上。
  if (!entryId && entry.sync_id) return -1;
  return found;
}

export function sameBlockTimer(left: BlockTimer, right: BlockTimer): boolean {
  if (left.sync_id || right.sync_id) return !!left.sync_id && left.sync_id === right.sync_id;
  return left.started_at === right.started_at && left.category === right.category;
}
