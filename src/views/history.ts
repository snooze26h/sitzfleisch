// 「历史」：一本月历。最近 10 周按日历排成一面墙，每天一轮那天的月亮：
// 满月是圆满的一天，亮了一半是学了一半，那天没有记录就是一轮没亮的新月；右边一栏是这面墙的图例兼计数。
// 下面逐日归档，每一行开头是同一轮小月；可展开看配额、台账，复制 Markdown，永久删除。

import type { ArchivedDay, Day } from "../types";
import { dayLabel, duration, esc, meter, wallRange } from "../format";
import { icon } from "../icons";
import { archiveMoon, crescentArt, dayFull, dayMoon, keyMoon, newMoon } from "../moon";
import { btn, quotaRows, sectionLabel, sessionRow } from "../components";
import { HISTORY_LIMIT, day, history, netSeconds, quotaSeconds, ui } from "../state";

export function historyEmpty(): boolean {
  return history().length === 0;
}

export function historyPage(): string {
  const days = [...history()].reverse();
  if (!days.length) {
    return `<section class="history-empty">${crescentArt("empty-art")}<div><h1>还没有归档</h1><p>收工后自动留存，最多 ${HISTORY_LIMIT} 天。</p></div></section>`;
  }
  return `<header class="page-heading"><div class="heading-copy"><h1>历史</h1><p>已归档 ${esc(days.length)} 天</p></div></header>` + moonWall(days) + archiveList(days);
}

const WALL_WEEKS = 10;
const WEEKDAY_MARKS = ["一", "二", "三", "四", "五", "六", "日"];

/** 本地日历上的哪一天。学习日按开始的那天算，过了午夜才收工也还是那一天。 */
function dateKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

function midnight(unix: number): Date {
  const date = new Date(unix * 1000);
  date.setHours(0, 0, 0, 0);
  return date;
}

/**
 * 月历墙：一列是一周（周一在上），最右一列是这一周。每一格是一天——
 * 归档过的是那天的月亮（满月就是圆满），最早一条记录以来没有记录的日子是一轮没亮的新月；
 * 更早的（那时还没有记录，或已超出保留天数）和还没到的日子都空着，不放要人去猜的记号。
 * 学习日正在进行的那一格画今天这一轮，亮到此刻为止。
 */
function moonWall(days: ArchivedDay[]): string {
  const weeks = ui.compact ? 6 : WALL_WEEKS;
  const today = midnight(ui.now);
  const first = new Date(today);
  first.setDate(today.getDate() - ((today.getDay() + 6) % 7) - 7 * (weeks - 1));

  // 同一天归档过两次（收工后又开了一天），墙上放学得多的那一次，列表里两条都在。
  const byDate = new Map<string, ArchivedDay>();
  for (const entry of days) {
    const key = dateKey(new Date(entry.day.started_at * 1000));
    const held = byDate.get(key);
    if (!held || netSeconds(entry.day) > netSeconds(held.day)) byDate.set(key, entry);
  }
  const oldest = days.length ? midnight(days[days.length - 1].day.started_at).getTime() : Infinity;
  const live = day();
  const liveKey = live ? dateKey(new Date(live.started_at * 1000)) : null;

  let full = 0;
  let partial = 0;
  let missing = 0;
  const nets: number[] = [];
  const columns: string[] = [`<span class="wall-corner"></span>`, ...WEEKDAY_MARKS.map((w) => `<span class="wall-wd">${w}</span>`)];
  const cursor = new Date(first);
  let lastMonth = -1;
  for (let week = 0; week < weeks; week++) {
    // 一列归它周一所在的月。第一列若紧挨着就换月，不再写，两个月份不挤在一起。
    const month = cursor.getMonth();
    const nextMonday = new Date(cursor);
    nextMonday.setDate(cursor.getDate() + 7);
    const crowded = week === 0 && nextMonday.getMonth() !== month;
    columns.push(`<span class="wall-mo">${month !== lastMonth && !crowded ? `${month + 1}月` : ""}</span>`);
    lastMonth = month;
    for (let i = 0; i < 7; i++) {
      const key = dateKey(cursor);
      const time = cursor.getTime();
      const entry = byDate.get(key);
      // 墙上一格只放一炉：同一天既有归档又有正在进行的学习日时，格子画正在烧的那一炉，
      // 归档的那一次照样算进右边的计数。
      if (entry) {
        nets.push(netSeconds(entry.day));
        if (dayFull(entry.day)) full++;
        else partial++;
      }
      if (live && key === liveKey) {
        const label = `${dayLabel(live.started_at)}，学习日进行中，已学${duration(netSeconds(live))}，回到今天`;
        columns.push(`<button class="day-cell live" data-action="tab" data-view="today" title="${esc(label)}" aria-label="${esc(label)}">${dayMoon(live, "tile")}</button>`);
      } else if (entry) {
        const d = entry.day;
        const done = netSeconds(d);
        const stamp = d.started_at;
        const selected = ui.selectedHistoryDay === stamp;
        const label = `${new Date(stamp * 1000).getFullYear()}年${dayLabel(stamp)}，已学${duration(done)}，目标${duration(quotaSeconds(d))}，查看归档详情`;
        columns.push(`<button class="day-cell${selected ? " on" : ""}" id="wall-day-${esc(stamp)}" data-action="open-chart-day" data-id="${esc(stamp)}" title="${esc(label)}" aria-label="${esc(label)}" aria-pressed="${selected}" aria-expanded="${ui.expandedDays.has(stamp)}" aria-controls="archive-detail-${esc(stamp)}">${archiveMoon(d, "tile")}</button>`);
      } else if (time >= today.getTime()) {
        columns.push(`<span class="day-cell"></span>`);
      } else if (time >= oldest) {
        missing++;
        const label = `${dayLabel(time / 1000)}，没有记录`;
        columns.push(ui.compact
          ? `<button class="day-cell miss" data-action="history-gap" data-id="${time / 1000}" aria-label="${esc(label)}">${newMoon("tile")}</button>`
          : `<span class="day-cell miss" title="${esc(label)}">${newMoon("tile")}</span>`);
      } else {
        columns.push(`<span class="day-cell"></span>`);
      }
      cursor.setDate(cursor.getDate() + 1);
    }
  }

  const avg = nets.length ? meter(nets.reduce((a, b) => a + b, 0) / nets.length) : "—";
  const best = nets.length ? meter(Math.max(...nets)) : "—";
  const row = (mark: string, label: string, value: string, unit = "") =>
    `<div class="k"><dt><span class="k-mark">${mark}</span>${label}</dt><dd>${esc(value)}${unit ? `<span>${unit}</span>` : ""}</dd></div>`;
  const legend = `<dl class="wall-key">
      ${row(keyMoon(1), "圆满", String(full), "天")}
      ${row(keyMoon(0.5), "未满", String(partial), "天")}
      ${row(newMoon("key"), "没有记录", String(missing), "天")}
      ${row("", "平均", avg)}
      ${row("", "最好", best)}
    </dl>`;
  return `<section class="wall-block" id="wall">${sectionLabel(`最近 ${weeks} 周`)}
    <div class="wall-body">
      <div class="wall-side"><div class="wall" role="group" aria-label="最近 ${weeks} 周，每天一轮月">${columns.join("")}</div><p class="t-note">点一轮月，查看当天的配额和记录。</p></div>
      ${legend}
    </div>
  </section>`;
}

function archiveList(days: ArchivedDay[]): string {
  return `<section class="archive" id="archive-list">${days.map(archiveRow).join("")}</section>`;
}

function compositionStrip(d: Day): string {
  const segments = d.categories
    .map((c) => {
      const target = c.quota_minutes * 60;
      const done = c.accepted_seconds;
      const live = target > 0 || done > 0;
      const ratio = target > 0 ? Math.min(1, done / target) : done > 0 ? 1 : 0;
      const title = target > 0 ? `${c.name} ${duration(done)} / ${meter(target)}` : done > 0 ? `${c.name} ${duration(done)}（当前目标为 0）` : `${c.name}（那天没排）`;
      return `<i class="${live ? "" : "idle"}" title="${esc(title)}"><b style="width:${(ratio * 100).toFixed(1)}%"></b></i>`;
    })
    .join("");
  return `<span class="comp">${segments}</span>`;
}

function archiveRow(entry: ArchivedDay): string {
  const d = entry.day;
  const open = ui.expandedDays.has(d.started_at);
  const accepted = d.ledger.filter((l) => l.accepted).length;
  const header = `<button class="archive-row${ui.selectedHistoryDay === d.started_at ? " on" : ""}${open ? " open" : ""}" id="archive-${d.started_at}" data-action="toggle-day" data-id="${d.started_at}" aria-expanded="${open}" aria-controls="archive-detail-${d.started_at}" aria-label="${open ? "折叠" : "展开"} ${esc(dayLabel(d.started_at))} 的归档详情">
      ${icon(open ? "chevron-down" : "chevron-right", 14, "chev")}
      <span class="archive-moon">${archiveMoon(d, "row")}</span>
      <span class="titles"><b>${esc(dayLabel(d.started_at))}</b><span>${esc(wallRange(d.started_at, entry.ended_at))} · ${accepted} 格计入</span></span>
      ${compositionStrip(d)}
      <span class="nums"><b>${esc(meter(netSeconds(d)))}</b><span>目标 ${esc(meter(quotaSeconds(d)))}</span></span>
    </button>`;
  if (!open) return header;
  const sessions = d.ledger.length ? `<div class="sessions">${d.ledger.map((l) => sessionRow(l, d)).join("")}</div>` : "";
  const detail = `<div class="archive-detail" id="archive-detail-${d.started_at}">
      ${quotaRows(d, { compact: true })}
      ${sessions}
      <div class="actions">
        ${btn("复制这一天的 Markdown", { kind: "plate", action: "copy-md", data: { id: d.started_at } })}
        <span class="spacer"></span>
        ${btn("永久删除这一天", { kind: "danger", action: "delete-day", data: { id: d.started_at }, title: "将先显示确认提示" })}
      </div>
    </div>`;
  return header + detail;
}
