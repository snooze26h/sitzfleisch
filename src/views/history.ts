// 「历史」：最近 14 天的柱图 + 归档清单（可展开看配额、台账，复制 Markdown，永久删除）。

import type { ArchivedDay, Day } from "../types";
import { dayLabel, dayOfMonth, duration, esc, meter, wallClock } from "../format";
import { icon } from "../icons";
import { btn, emptyState, hair, plate, quotaLane, reading, sectionLabel, sessionRow } from "../components";
import { HISTORY_LIMIT, history, netSeconds, quotaSeconds, ui } from "../state";

export function historyEmpty(): boolean {
  return history().length === 0;
}

export function historyPage(): string {
  const days = [...history()].reverse();
  if (!days.length) {
    return emptyState("archive", "还没有归档的日子", `结束第一个学习日之后，成果和净投入会保存在这里，最多留 ${HISTORY_LIMIT} 天。`);
  }
  return `<header class="page-heading"><h1>历史</h1><p>已归档 ${esc(days.length)} 天</p></header>` + runChart(days) + archiveList(days);
}

function runChart(days: ArchivedDay[]): string {
  const recent = days.slice(0, 14).reverse();
  const progress = recent.map((d) => netSeconds(d.day));
  const targets = recent.map((d) => quotaSeconds(d.day));
  const ceiling = Math.max(3600, ...progress, ...targets);
  const avg = progress.reduce((a, b) => a + b, 0) / Math.max(1, progress.length);
  const best = Math.max(0, ...progress);
  const columns = recent
    .map((entry, i) => {
      const done = progress[i];
      const target = targets[i];
      const height = Math.max(2, (96 * done) / ceiling);
      const targetY = (96 * target) / ceiling;
      const key = entry.day.started_at;
      const selected = ui.selectedHistoryDay === key;
      const title = `${new Date(key * 1000).getFullYear()}年${dayLabel(key)}，净投入${duration(done)}，目标${duration(target)}，查看归档详情`;
      return `<button class="colm${selected ? " on" : ""}" id="chart-day-${esc(key)}" data-action="open-chart-day" data-id="${esc(key)}" title="${esc(title)}" aria-label="${esc(title)}" aria-pressed="${selected}" aria-expanded="${ui.expandedDays.has(key)}" aria-controls="archive-detail-${esc(key)}">
        <span class="box" aria-hidden="true"><i class="base"></i><i class="bar${target > 0 && done >= target ? " met" : ""}" style="height:${height.toFixed(1)}px"></i>${target > 0 ? `<i class="target" style="bottom:${targetY.toFixed(1)}px"></i>` : ""}</span>
        <span class="day">${esc(dayOfMonth(entry.day.started_at))}</span>
      </button>`;
    })
    .join("");
  const trailing = `${reading("平均", meter(avg), { size: 16, trailing: true })}${reading("最好", meter(best), { size: 16, trailing: true })}`;
  const legend = `<span class="legend"><span><i></i>净投入</span><span><i class="line"></i>目标</span></span>`;
  return plate(`<div class="chart">${sectionLabel(`最近 ${recent.length} 天`, { trailing: `${legend}<span style="display:flex;gap:22px">${trailing}</span>` })}<div class="cols">${columns}</div><p class="t-note">点击日期柱查看当天的配额和记录。</p></div>`);
}

function archiveList(days: ArchivedDay[]): string {
  return plate(`<div class="archive">${days.map(archiveRow).join(hair())}</div>`, "flush");
}

function compositionStrip(d: Day): string {
  const segments = d.categories
    .map((c) => {
      const target = c.quota_minutes * 60;
      const done = c.accepted_seconds;
      const live = target > 0 || done > 0;
      const ratio = target > 0 ? Math.min(1, done / target) : done > 0 ? 1 : 0;
      const title = target > 0 ? `${c.name} ${duration(done)} / ${meter(target)}` : done > 0 ? `${c.name} ${duration(done)}（当前目标为 0）` : `${c.name}（这一档不排）`;
      return `<i class="${live ? "" : "idle"}" title="${esc(title)}"><b style="width:${(ratio * 100).toFixed(1)}%"></b></i>`;
    })
    .join("");
  return `<span class="comp">${segments}</span>`;
}

function archiveRow(entry: ArchivedDay): string {
  const d = entry.day;
  const open = ui.expandedDays.has(d.started_at);
  const accepted = d.ledger.filter((l) => l.accepted).length;
  const header = `<button class="archive-row${ui.selectedHistoryDay === d.started_at ? " on" : ""}" id="archive-${d.started_at}" data-action="toggle-day" data-id="${d.started_at}" aria-expanded="${open}" aria-controls="archive-detail-${d.started_at}" aria-label="${open ? "折叠" : "展开"} ${esc(dayLabel(d.started_at))} 的归档详情">
      ${icon(open ? "chevron-down" : "chevron-right", 12)}
      <span class="titles"><b>${esc(dayLabel(d.started_at))}</b><span>${esc(wallClock(d.started_at))}–${esc(wallClock(entry.ended_at))} · ${accepted} 格通过</span></span>
      ${compositionStrip(d)}
      <span class="nums"><b>${esc(meter(netSeconds(d)))}</b><span>目标 ${esc(meter(quotaSeconds(d)))}</span></span>
    </button>`;
  if (!open) return header;
  const lanes = d.categories.map((c) => quotaLane(c, { day: d })).join("");
  const sessions = d.ledger.length ? `${hair("mt4")}<div class="sessions">${d.ledger.map((l) => sessionRow(l, d)).join(hair())}</div>` : "";
  const detail = `<div class="archive-detail" id="archive-detail-${d.started_at}">
      ${hair()}
      ${lanes}
      ${sessions}
      <div class="actions">
        ${btn("复制这一天的 Markdown", { kind: "plate", action: "copy-md", data: { id: d.started_at } })}
        <span class="spacer"></span>
        ${btn("永久删除这一天", { kind: "danger", action: "delete-day", data: { id: d.started_at }, title: "将先显示确认提示" })}
      </div>
    </div>`;
  return header + detail;
}
