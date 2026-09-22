import type { PauseSpan } from "./types";

/** 只投影实际工作的墙钟区间；暂停是空隙，不能拉长成一整条工作记录。 */
export function activeSpans(start: number, end: number, pauses: PauseSpan[]): { start: number; end: number }[] {
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return [];
  const gaps = pauses
    .map((pause) => ({ start: Math.max(start, pause.started_at), end: Math.min(end, pause.ended_at ?? end) }))
    .filter((gap) => gap.end > gap.start)
    .sort((a, b) => a.start - b.start);
  const spans: { start: number; end: number }[] = [];
  let cursor = start;
  for (const gap of gaps) {
    if (gap.start > cursor) spans.push({ start: cursor, end: gap.start });
    cursor = Math.max(cursor, gap.end);
  }
  if (cursor < end) spans.push({ start: cursor, end });
  return spans;
}
