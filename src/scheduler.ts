// 「下一格」建议：每条都带一句能说出口的理由，界面直接照着念。
// 每条建议都带着产生它的那一句理由——没有理由的建议是噪音。

import type { CategoryState, Day, Preferences } from "./types";
import { duration } from "./format";

export interface Suggestion {
  category: string;
  minutes: number;
  reason: string;
  pressing: boolean;
}

const SITTING_ALARM = 3 * 3600;
const FRESH_WINDOW = 3 * 3600;

export function remainingSeconds(c: CategoryState): number {
  return Math.max(0, c.quota_minutes * 60 - c.accepted_seconds);
}

export function roleOf(prefs: Preferences, id: string): string {
  return prefs.categories.find((c) => c.id === id)?.role ?? "general";
}

/** 上一个做过的项目，用来在建议里换个脑子。 */
export function lastCategory(day: Day): string | null {
  return day.ledger.length ? day.ledger[day.ledger.length - 1].category : null;
}

/** 距离上一次锻炼（或今天开始）以来的连续久坐时间，暂停的时段扣掉。 */
export function sittingStreakSeconds(day: Day, prefs: Preferences, now: number): number {
  let since = day.started_at;
  for (const entry of day.ledger) {
    if (entry.accepted && roleOf(prefs, entry.category) === "movement" && entry.ended_at > since) {
      since = entry.ended_at;
    }
  }
  const elapsed = Math.max(0, now - since);
  const pausedAfter = day.pauses
    .filter((p) => p.started_at >= since)
    .reduce((sum, p) => sum + Math.max(0, (p.ended_at ?? now) - p.started_at), 0);
  return Math.max(0, elapsed - pausedAfter);
}

/** 统一块长开着就用它，否则用项目各自的默认块长。 */
export function effectiveBlockMinutes(prefs: Preferences, id: string): number {
  if (prefs.uniform_block_minutes > 0) return prefs.uniform_block_minutes;
  return prefs.categories.find((c) => c.id === id)?.default_block_minutes ?? 50;
}

export function blockMinutes(category: CategoryState, prefs: Preferences): number {
  const preferred = effectiveBlockMinutes(prefs, category.id);
  const remainingMinutes = Math.ceil(remainingSeconds(category) / 60);
  if (remainingMinutes <= 0) return preferred;
  return Math.max(5, Math.min(preferred, remainingMinutes));
}

interface Scored {
  category: CategoryState;
  score: number;
  remaining: number;
}

/** 右侧只在明确更好时替换；完全平手保留计划中更靠前的类别。 */
function preferred(current: Scored, next: Scored): Scored {
  if (next.score !== current.score) return next.score > current.score ? next : current;
  if (next.remaining !== current.remaining) return next.remaining > current.remaining ? next : current;
  return current;
}

function score(category: CategoryState, day: Day, prefs: Preferences, now: number, candidateCount: number): number {
  const target = category.quota_minutes * 60;
  if (target <= 0) return 0;
  let value = remainingSeconds(category) / target;
  if (category.id === lastCategory(day) && candidateCount > 1) value *= 0.55;
  switch (roleOf(prefs, category.id)) {
    case "deepWork":
      if (day.seated_seconds < FRESH_WINDOW) value *= 1.3;
      break;
    case "exploration": {
      const scheduledDeep = day.categories.filter((c) => roleOf(prefs, c.id) === "deepWork" && c.quota_minutes > 0);
      if (scheduledDeep.length && !scheduledDeep.some((c) => c.accepted_seconds > 0)) value *= 0.5;
      break;
    }
    case "movement": {
      const sitting = sittingStreakSeconds(day, prefs, now);
      value *= 0.5 + Math.min(1.5, sitting / SITTING_ALARM);
      break;
    }
    default:
      break;
  }
  return value;
}

export function suggest(day: Day, prefs: Preferences, now: number): Suggestion | null {
  const candidates = day.categories.filter((c) => remainingSeconds(c) > 0);
  if (!candidates.length) return null;
  const make = (c: CategoryState, reason: string, pressing: boolean): Suggestion => ({
    category: c.id,
    minutes: blockMinutes(c, prefs),
    reason,
    pressing,
  });

  const sitting = sittingStreakSeconds(day, prefs, now);
  if (sitting >= SITTING_ALARM) {
    const movement = candidates.find((c) => roleOf(prefs, c.id) === "movement");
    if (movement) return make(movement, `连坐 ${duration(sitting)}，该动一动。`, true);
  }

  const totalRemaining = candidates.reduce((sum, c) => sum + remainingSeconds(c), 0);
  const last = lastCategory(day);
  const floors = candidates.filter((c) => roleOf(prefs, c.id) === "dailyFloor" && last !== c.id);
  const floor = floors.reduce<CategoryState | null>(
    (best, c) => (!best || remainingSeconds(c) > remainingSeconds(best) ? c : best),
    null
  );
  if (totalRemaining > 0 && floor && remainingSeconds(floor) / totalRemaining >= 0.5) {
    return make(floor, `缺口最大，还剩 ${duration(remainingSeconds(floor))}。`, true);
  }

  const scored = candidates.map((c) => ({
    category: c,
    score: score(c, day, prefs, now, candidates.length),
    remaining: remainingSeconds(c),
  }));
  const best = scored.slice(1).reduce(preferred, scored[0]).category;
  const remaining = remainingSeconds(best);
  let reason = `落后最多，还差 ${duration(remaining)}。`;
  if (roleOf(prefs, best.id) === "deepWork" && day.seated_seconds < FRESH_WINDOW) {
    reason = `趁清醒先做，还差 ${duration(remaining)}。`;
  }
  return make(best, reason, false);
}

/** 按现在的节奏，把剩余配额按各自块长切格，格间按休息时长算，得到还要坐多久（不含暂停）。 */
export function estimatedRemainingWallSeconds(day: Day, prefs: Preferences): number {
  let focusSeconds = 0;
  let blockCount = 0;
  for (const c of day.categories) {
    const remaining = remainingSeconds(c);
    if (remaining <= 0) continue;
    const block = Math.max(60, effectiveBlockMinutes(prefs, c.id) * 60);
    const rounded = Math.ceil(remaining / 60) * 60;
    focusSeconds += rounded;
    blockCount += Math.ceil(rounded / block);
  }
  const breaks = Math.max(0, blockCount - 1);
  return focusSeconds + breaks * prefs.break_minutes * 60;
}
