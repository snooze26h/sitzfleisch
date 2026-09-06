// 一天的 Markdown 总结：只有用户点复制时才进剪贴板。

import type { Day } from "./types";
import { dayLabel, duration, meter, wallClock } from "./format";
import { netSeconds, quotaSeconds } from "./state";

/** 短于这个的暂停不单独列：格与格之间总要喘口气，一条条列出来只会淹没真正停下来的那几段。 */
const SHORT_PAUSE_SECONDS = 5 * 60;

export function markdownForDay(day: Day, endedAt: number | null, now: number): string {
  const net = netSeconds(day);
  const end = endedAt ?? now;
  const lines: string[] = [];
  lines.push(`# ${dayLabel(day.started_at)} · ${day.profile_name}档`);
  lines.push("");
  lines.push(`- ${wallClock(day.started_at)} 坐下，${endedAt ? "收工于" : "截至"} ${wallClock(end)}`);
  lines.push(`- 已学 ${meter(net)} / 目标 ${meter(quotaSeconds(day))} · 暂停 ${meter(day.paused_seconds)} · 水 ${day.cups} 杯`);
  lines.push("");
  lines.push("## 各项目");
  for (const c of day.categories) {
    lines.push(`- ${c.name}：${meter(c.accepted_seconds)} / ${meter(c.quota_minutes * 60)}`);
  }
  if (day.ledger.length) {
    lines.push("");
    lines.push("## 记录");
    for (const l of day.ledger) {
      const startedAt = l.started_at > 0 ? l.started_at : l.ended_at - l.seconds;
      const head = `- ${wallClock(startedAt)}–${wallClock(l.ended_at)} ${categoryName(day, l.category)} ${duration(l.seconds)}${l.accepted ? "" : "（未计入）"}`;
      lines.push(head);
      for (const t of l.tasks) lines.push(`  - [${t.done ? "x" : " "}] ${t.text}`);
    }
  }
  // 0.8.0 之后每两格之间都有一段暂停，全列出来就是十几行流水账。
  // 只列真正停下来过的，剩下的用一行汇总带过。
  const spans = day.pauses.map((p) => ({ ...p, seconds: (p.ended_at ?? end) - p.started_at })).filter((p) => p.seconds > 0);
  const long = spans.filter((p) => p.seconds >= SHORT_PAUSE_SECONDS);
  const short = spans.filter((p) => p.seconds < SHORT_PAUSE_SECONDS);
  if (spans.length) {
    lines.push("");
    lines.push("## 暂停");
    for (const p of long) {
      const label = p.auto ? "（休眠）" : "";
      lines.push(`- ${wallClock(p.started_at)}–${p.ended_at ? wallClock(p.ended_at) : "…"} ${duration(p.seconds)}${label}`);
    }
    if (short.length) {
      const total = short.reduce((sum, p) => sum + p.seconds, 0);
      lines.push(`- 另有 ${short.length} 段 ${SHORT_PAUSE_SECONDS / 60} 分钟以内的短暂停，合计 ${duration(total)}`);
    }
  }
  return lines.join("\n");
}

function categoryName(day: Day, id: string): string {
  return day.categories.find((c) => c.id === id)?.name ?? id;
}
