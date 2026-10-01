// 组件：分区标题、读数、横香（配额/这一格）、选择器、按钮与原生风格控件、台账行。
// 全部返回 HTML 字符串，由 morphdom 打到 DOM 上。

import type { Day, LedgerEntry } from "./types";
import { esc, meter, duration, wallClock } from "./format";
import { icon } from "./icons";
import { iconOf, shortName, remainingOf, def } from "./state";

export function attrs(data: Record<string, string | number | boolean | undefined> = {}): string {
  return Object.entries(data)
    .filter(([, v]) => v !== undefined && v !== false)
    .map(([k, v]) => ` ${k}="${esc(String(v === true ? "" : v))}"`)
    .join("");
}

export function hair(cls = ""): string {
  return `<div class="hair${cls ? ` ${cls}` : ""}"></div>`;
}

/** 分区只靠一行字名和一条细线分开，不再套板子。 */
export function sectionLabel(title: string, opts: { note?: string; trailing?: string; cls?: string } = {}): string {
  return `<header class="section-head${opts.cls ? ` ${opts.cls}` : ""}">
    <h2 class="section-title">${esc(title)}</h2>${opts.note ? `<span class="section-note">${esc(opts.note)}</span>` : ""}${opts.trailing ? `<span class="trail">${opts.trailing}</span>` : ""}
  </header>`;
}

export function reading(label: string, value: string, opts: { trailing?: boolean } = {}): string {
  return `<div class="reading${opts.trailing ? " trailing" : ""}">
    <span class="engraved">${esc(label)}</span>
    <span class="val">${esc(value)}</span>
  </div>`;
}

export function bigReading(value: string, unit: string, opts: { tint?: "caution" | "muted" } = {}): string {
  return `<div class="big-reading${opts.tint ? ` ${opts.tint}` : ""}">
    <span class="val">${esc(value)}</span>
    <span class="unit">${esc(unit)}</span>
  </div>`;
}

/**
 * 今日配额：每个项目一行，一炷横放的香。所有行用同一把尺，香的长短就是配额，整点一道刻痕，香尾一道长的是目标；
 * 从左往右烧：骨白的灰是已计入，朱红是这一格正在烧的部分（按停时是赭石），没烧的只剩一道细槽。
 * 上一版是一排竖香：几根细棍隔得很开，看着孤零零的，悬停时整列套一块圆角底也怪，所以改成一行一个项目。
 */
export function quotaRows(d: Day, opts: {
  pulse?: string | null;
  selectable?: boolean;
  selected?: string | null;
  /** 正在走（或被按停）的那一格属于哪个项目，以及它已经烧了多少秒。 */
  active?: { category: string; elapsed: number; held: boolean } | null;
  compact?: boolean;
} = {}): string {
  const longest = Math.max(1, ...d.categories.map((c) => c.quota_minutes));
  const pct = (v: number) => `${(v * 100).toFixed(2)}%`;
  const rows = d.categories.map((c) => {
    const target = c.quota_minutes * 60;
    const done = c.accepted_seconds;
    const doneRatio = target > 0 ? Math.min(1, done / target) : 0;
    const active = opts.active?.category === c.id ? opts.active : null;
    const liveRatio = active && target > 0 ? Math.min(1 - doneRatio, active.elapsed / target) : 0;
    const complete = target > 0 && done >= target;
    let ticks = "";
    for (let hour = 3600; hour < target; hour += 3600) ticks += `<i class="q-tick" style="left:${pct(hour / target)}"></i>`;
    const cls = [
      "q-row",
      opts.selectable ? "selectable" : "",
      opts.selectable && opts.selected === c.id ? "on" : "",
      complete ? "complete" : "",
      opts.pulse === c.id ? "pulse" : "",
      active ? (active.held ? "held" : "burning") : "",
    ].filter(Boolean).join(" ");
    // 灰和正在烧的一段始终占位：结构每秒不变，只有宽度在动。
    const inner = `<span class="q-name">${icon(iconOf(c.id), 16)}<span class="nm">${esc(c.name)}</span></span>
      <span class="q-track">${target > 0 ? `<span class="q-stick" style="width:${pct(c.quota_minutes / longest)}">${ticks}<i class="q-ash" style="width:${pct(doneRatio)}"></i><i class="q-live" style="left:${pct(doneRatio)};width:${pct(liveRatio)}"></i></span>` : ""}</span>
      ${complete
        ? `<span class="q-num full">${icon("check", 14, "done-check")}<span class="done-tag">已满</span><b>${esc(meter(done))}</b></span>`
        : `<span class="q-num"><b>${esc(meter(done))}</b><span>/ ${esc(meter(target))}</span></span>`}`;
    if (!opts.selectable) return `<div class="${cls}">${inner}</div>`;
    const label = `选择${c.name}作为下一格，已完成${duration(done)}，目标${duration(target)}`;
    return `<button class="${cls}" id="today-quota-${esc(c.id)}" data-action="choose-cat" data-id="${esc(c.id)}" aria-pressed="${opts.selected === c.id}" aria-label="${esc(label)}">${inner}</button>`;
  });
  return `<div class="quota-rows${opts.compact ? " compact" : ""}">${rows.join("")}</div>`;
}

/** 这一格是一炷横放的香：左边烧过，火头往右走，刻度每五分钟一道、一刻钟一道长的。 */
export function blockStick(opts: { elapsed: number; planned: number; running: boolean; startedAt: number; projectedEnd: number }): string {
  const ratio = opts.planned > 0 ? Math.max(0, Math.min(1, opts.elapsed / opts.planned)) : 0;
  const minutes = Math.max(1, Math.round(opts.planned / 60));
  const step = minutes > 100 ? 10 : 5;
  let ticks = "";
  for (let minute = step; minute < minutes; minute += step) {
    const at = minute / minutes;
    ticks += `<i class="bs-tick${minute % 15 === 0 ? " major" : ""}${at <= ratio ? " passed" : ""}" style="left:${(at * 100).toFixed(3)}%"></i>`;
  }
  const at = (ratio * 100).toFixed(3);
  return `<div class="block-stick-wrap${opts.running ? "" : " held"}">
    <div class="block-stick" role="progressbar" aria-label="这一格的进度" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(ratio * 100)}" aria-valuetext="已专注 ${esc(duration(opts.elapsed))}">
      <i class="bs-groove"></i>${ticks}<i class="bs-ash" style="width:${at}%"></i><i class="bs-ember" style="left:clamp(7px, ${at}%, calc(100% - 7px))"></i>
    </div>
    <div class="strip-foot">
      <span>${esc(wallClock(opts.startedAt))}</span>
      <span class="mid">计划 ${minutes} 分钟</span>
      <span>预计 ${esc(wallClock(opts.projectedEnd))}</span>
    </div>
  </div>`;
}

export function categoryPicker(d: Day, selection: string | null): string {
  const cells = d.categories
    .map((c) => {
      const full = remainingOf(c) <= 0;
      const active = selection === c.id;
      const cls = ["cell", active ? "on" : "", full ? "full" : ""].filter(Boolean).join(" ");
      const title = full ? `${c.name}今天的配额已经满了` : c.name;
      // 配额满了：项目图标照旧（换成勾会认不出是哪个项目），名字后面补一个骨白的勾。
      // 以前是划一道线，隔着书桌看不清。
      return `<button class="${cls}" data-action="choose-cat" data-id="${esc(c.id)}" title="${esc(title)}" aria-pressed="${active}">
        ${icon(iconOf(c.id), 17)}
        <span class="label">${esc(shortName(c.id, d))}</span>${full ? icon("check", 15, "full-check") : ""}
      </button>`;
    })
    .join("");
  // 列数交给 CSS 变量：一行装完，宁可每格窄一点，也不让最后一个项目落单。
  return `<div class="picker" style="--cells:${d.categories.length}">${cells}</div>`;
}

export type ButtonKind = "primary" | "plate" | "quiet" | "danger" | "quiet-danger";

export function btn(label: string, opts: { kind?: ButtonKind; action: string; data?: Record<string, string | number>; cls?: string; disabled?: boolean; icon?: string; title?: string } ): string {
  const kind = opts.kind ?? "plate";
  const data: Record<string, string | number> = { "data-action": opts.action };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  return `<button class="btn btn-${kind}${opts.cls ? ` ${opts.cls}` : ""}"${attrs(data)}${opts.disabled ? " disabled" : ""}${opts.title ? ` title="${esc(opts.title)}"` : ""}>${opts.icon ? icon(opts.icon, 14) : ""}<span>${esc(label)}</span></button>`;
}

export function select(opts: { change: string; value: string | number; options: { value: string | number; label: string }[]; data?: Record<string, string | number>; width?: number; disabled?: boolean; label?: string }): string {
  const data: Record<string, string | number> = { "data-change": opts.change };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  const items = opts.options
    .map((o) => `<option value="${esc(String(o.value))}"${String(o.value) === String(opts.value) ? " selected" : ""}>${esc(o.label)}</option>`)
    .join("");
  return `<select class="select"${attrs(data)}${opts.width ? ` style="width:${opts.width}px"` : ""}${opts.disabled ? " disabled" : ""}${opts.label ? ` aria-label="${esc(opts.label)}"` : ""}>${items}</select>`;
}

export function toggle(opts: { change: string; checked: boolean; data?: Record<string, string | number>; label?: string; disabled?: boolean }): string {
  const data: Record<string, string | number> = { "data-change": opts.change };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  return `<label class="switch"><input type="checkbox"${attrs(data)}${opts.checked ? " checked" : ""}${opts.disabled ? " disabled" : ""}${opts.label ? ` aria-label="${esc(opts.label)}"` : ""} /><span class="knob"></span></label>`;
}

export function stepper(opts: { bind: string; value: number; label: string; min: number; max: number; step: number; data?: Record<string, string | number>; ariaLabel?: string }): string {
  const data: Record<string, string | number> = { "data-bind": opts.bind, "data-min": opts.min, "data-max": opts.max, "data-step": opts.step, "data-value": opts.value };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  return `<span class="stepper"${opts.ariaLabel ? ` aria-label="${esc(opts.ariaLabel)}"` : ""}>
    <button data-action="step" data-delta="-1"${attrs(data)}${opts.value <= opts.min ? " disabled" : ""} aria-label="减少">${icon("minus", 12)}</button>
    <span class="val">${esc(opts.label)}</span>
    <button data-action="step" data-delta="1"${attrs(data)}${opts.value >= opts.max ? " disabled" : ""} aria-label="增加">${icon("plus", 12)}</button>
  </span>`;
}

export function labelled(title: string, control: string): string {
  return `<span class="labelled"><span class="engraved">${esc(title)}</span>${control}</span>`;
}

/** 同一处输入支持键入和逐分钟微调，按钮与输入框共用边界。 */
export function minuteInput(opts: { change: string; value: number; min: number; max: number; label: string; data?: Record<string, string | number> }): string {
  const data: Record<string, string | number> = {};
  for (const [key, value] of Object.entries(opts.data ?? {})) data[`data-${key}`] = value;
  const key = `${opts.change}-${Object.values(opts.data ?? {}).map((value) => encodeURIComponent(String(value))).join("-")}`;
  const adjust = (delta: number, glyph: string, verb: string) => `<button type="button" data-action="step" data-bind="${esc(opts.change)}" data-delta="${delta}" data-step="1" data-min="${opts.min}" data-max="${opts.max}" data-value="${opts.value}"${attrs(data)}${delta < 0 ? opts.value <= opts.min ? " disabled" : "" : opts.value >= opts.max ? " disabled" : ""} aria-label="${esc(opts.label)}${verb} 1 分钟">${icon(glyph, 12)}</button>`;
  return `<span class="minute-input">${adjust(-1, "minus", "减少")}<input id="minutes-${esc(key)}" class="field" type="number" inputmode="numeric" min="${opts.min}" max="${opts.max}" step="1" value="${opts.value}" data-change="${esc(opts.change)}"${attrs(data)} aria-label="${esc(opts.label)}" /><span class="unit">分</span>${adjust(1, "plus", "增加")}</span>`;
}

export function menuButton(id: string, open: string | null, label: string, items: string, opts: { iconOnly?: boolean; left?: boolean; ariaLabel?: string } = {}): string {
  const isOpen = open === id;
  return `<span class="anchor">
    <button class="menu-trigger${opts.iconOnly ? " icon-only" : ""}${isOpen ? " open" : ""}" data-action="menu" data-id="${esc(id)}"${opts.ariaLabel ? ` aria-label="${esc(opts.ariaLabel)}"` : ""} aria-expanded="${isOpen}">${label}</button>
    ${isOpen ? `<div class="menu${opts.left ? " left" : ""}" role="menu">${items}</div>` : ""}
  </span>`;
}

export function menuItem(label: string, action: string, opts: { data?: Record<string, string | number>; icon?: string; danger?: boolean } = {}): string {
  const data: Record<string, string | number> = { "data-action": action };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  return `<button class="menu-item${opts.danger ? " danger" : ""}" role="menuitem"${attrs(data)}>${opts.icon ? icon(opts.icon, 14) : ""}<span>${esc(label)}</span></button>`;
}

/** 台账的一行。放弃了、只记在账上的那一格名字淡一档，后面写明「未计入」；行首不再放要人去猜的方块。 */
export function sessionRow(entry: LedgerEntry, d: Day): string {
  const startedAt = entry.started_at > 0 ? entry.started_at : entry.ended_at - entry.seconds;
  const definition = def(entry.category);
  const name = d.categories.find((c) => c.id === entry.category)?.name ?? definition?.name ?? entry.category;
  const tasks = entry.tasks.length
    ? `<div class="session-tasks">${entry.tasks
        .map((t) => `<span class="${t.done ? "done" : ""}">${icon(t.done ? "check" : "minus", 12)}${esc(t.text)}</span>`)
        .join("")}</div>`
    : "";
  const completion = entry.completion_note?.trim()
    ? `<p class="session-note">${esc(entry.completion_note)}</p>` : "";
  const edit = entry.accepted ? btn(completion ? "编辑完成记录" : "补充完成记录", {
    kind: "quiet", action: "edit-completion", cls: "session-edit",
    data: { day: d.started_at, index: d.ledger.indexOf(entry), ended: entry.ended_at },
  }) : "";
  return `<div class="session${entry.accepted ? "" : " rejected"}">
    <div class="body">
      <div class="meta">
        <span class="nm">${esc(name)}</span>
        <span class="dur">${esc(duration(entry.seconds))}</span>
        <span class="at">${esc(wallClock(startedAt))}–${esc(wallClock(entry.ended_at))}</span>
        ${entry.accepted ? "" : `<span class="badge">未计入</span>`}
      </div>
      ${completion}${tasks}${edit}
    </div>
  </div>`;
}
