// 组件：板、凹槽、刻字小标、读数、刻度轨、配额轨、走时条、选择器、按钮与原生风格控件。
// 全部返回 HTML 字符串，由 morphdom 打到 DOM 上。

import type { CategoryState, Day, LedgerEntry } from "./types";
import { esc, meter, duration, wallClock } from "./format";
import { icon } from "./icons";
import { iconOf, shortName, remainingOf, def } from "./state";

export function attrs(data: Record<string, string | number | boolean | undefined> = {}): string {
  return Object.entries(data)
    .filter(([, v]) => v !== undefined && v !== false)
    .map(([k, v]) => ` ${k}="${esc(String(v === true ? "" : v))}"`)
    .join("");
}

export function plate(inner: string, cls = "", data: Record<string, string> = {}): string {
  return `<section class="plate${cls ? ` ${cls}` : ""}"${attrs(data)}>${inner}</section>`;
}

export function hair(cls = ""): string {
  return `<div class="hair${cls ? ` ${cls}` : ""}"></div>`;
}

export function sectionLabel(title: string, opts: { note?: string; trailing?: string; cls?: string; icon?: string } = {}): string {
  return `<div class="section${opts.cls ? ` ${opts.cls}` : ""}">
    <div class="row">${opts.icon ? icon(opts.icon, 13, "sec-ic") : ""}<span class="title">${esc(title)}</span>${opts.trailing ? `<span class="trail">${opts.trailing}</span>` : ""}</div>
    ${hair()}
    ${opts.note ? `<span class="note">${esc(opts.note)}</span>` : ""}
  </div>`;
}

export function reading(label: string, value: string, opts: { size?: number; tint?: string; trailing?: boolean } = {}): string {
  const size = opts.size ?? 20;
  return `<div class="reading${opts.trailing ? " trailing" : ""}">
    <span class="engraved">${esc(label)}</span>
    <span class="val" style="font-size:${size}px${opts.tint ? `;color:var(--${opts.tint})` : ""}">${esc(value)}</span>
  </div>`;
}

export function bigReading(value: string, unit: string, opts: { tint?: string } = {}): string {
  return `<div class="big-reading">
    <span class="val"${opts.tint ? ` style="color:var(--${opts.tint})"` : ""}>${esc(value)}</span>
    <span class="unit">${esc(unit)}</span>
  </div>`;
}

/** 带分格刻度的凹槽。分格是真信息：4 小时的轨就有 4 格。 */
export function tickedTrack(ratio: number, divisions: number, opts: { mini?: boolean; fill?: "stroke" | "signal"; pulse?: boolean } = {}): string {
  const clamped = Math.max(0, Math.min(1, ratio));
  let ticks = "";
  if (divisions > 1) {
    for (let i = 1; i < divisions; i++) {
      const at = i / divisions;
      ticks += `<i class="tick ${at < clamped ? "passed" : "ahead"}" style="left:${(at * 100).toFixed(3)}%"></i>`;
    }
  }
  const pulse = opts.pulse === undefined ? "" : `<i class="pulse${opts.pulse ? " on" : ""}" style="left:calc(${(clamped * 100).toFixed(3)}% - 3px)"></i>`;
  return `<span class="track${opts.mini ? " mini" : ""}">${clamped > 0 ? `<i class="fill${opts.fill === "signal" ? " signal" : ""}" style="width:${(clamped * 100).toFixed(3)}%"></i>` : ""}${ticks}${pulse}</span>`;
}

export function quotaLane(c: CategoryState, opts: { marked?: boolean; pulsing?: boolean; day?: Day | null; selectable?: boolean; selected?: boolean } = {}): string {
  const target = c.quota_minutes * 60;
  const done = c.accepted_seconds;
  const ratio = target > 0 ? Math.min(1, done / target) : 0;
  const complete = target > 0 && done >= target;
  const divisions = target > 0 && target % 3600 === 0 ? target / 3600 : 1;
  const cls = ["lane", opts.marked ? "marked" : "", complete ? "complete" : "", opts.selectable ? "selectable" : "", opts.selected ? "on" : ""].filter(Boolean).join(" ");
  const tag = opts.selectable ? "button" : "div";
  const action = opts.selectable ? ` id="today-quota-${esc(c.id)}" data-action="choose-cat" data-id="${esc(c.id)}" aria-pressed="${!!opts.selected}" aria-label="${esc(`选择${c.name}作为下一格，已完成${duration(done)}，目标${duration(target)}`)}"` : "";
  return `<${tag} class="${cls}"${action}>
    <i class="mark"></i>
    <span class="lane-body">
      <span class="head">
        ${icon(iconOf(c.id), 13)}
        <span class="name">${esc(c.name)}</span>
        <span class="done">${esc(meter(done))}</span>
        <span class="of">/ ${esc(meter(target))}</span>
      </span>
      ${tickedTrack(ratio, divisions, { pulse: opts.pulsing ?? false })}
    </span>
  </${tag}>`;
}

export function runStrip(opts: {
  elapsed: number;
  planned: number;
  running: boolean;
  focus: boolean;
  startedAt: number;
  projectedEnd: number;
}): string {
  const ratio = opts.planned > 0 ? Math.max(0, Math.min(1, opts.elapsed / opts.planned)) : 0;
  const minutes = Math.max(1, Math.round(opts.planned / 60));
  const step = minutes > 100 ? 10 : 5;
  let ticks = "";
  for (let minute = step; minute < minutes; minute += step) {
    const at = minute / minutes;
    const major = minute % 15 === 0;
    const passed = at < ratio;
    const height = major ? 46 : 24;
    const color = passed ? `rgba(0,0,0,${major ? 0.45 : 0.28})` : `rgba(255,255,255,${major ? 0.22 : 0.12})`;
    ticks += `<i class="tick" style="left:${(at * 100).toFixed(3)}%;height:${height}%;background:${color}"></i>`;
  }
  // 填充一律骨白：整条铺朱红会变成一块警报板，而朱红在这套语言里是「记号」，不是「底色」。
  // 真的在跑这件事由旗杆和面板头部的图标承担。
  const fillCls = "fill";
  const flagCls = opts.running ? "flag" : "flag caution";
  const flag = ratio > 0 ? `<i class="${flagCls}" style="left:calc(${(ratio * 100).toFixed(3)}% - 0.75px)"></i>` : "";
  return `<div>
    <div class="strip">${ratio > 0 ? `<i class="${fillCls}" style="width:${(ratio * 100).toFixed(3)}%"></i>` : ""}${ticks}${flag}</div>
    <div class="strip-foot">
      <span>${esc(wallClock(opts.startedAt))}</span>
      <span class="mid${opts.running ? "" : " caution"}">${opts.running ? `计划 ${minutes} 分钟` : "已暂停"}</span>
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
      // 配额满了用划掉表示（.cell.full 的 line-through），不换字形——
      // 换成勾会让人以为这是另一种东西。
      return `<button class="${cls}" data-action="choose-cat" data-id="${esc(c.id)}" title="${esc(title)}" aria-pressed="${active}">
        ${icon(iconOf(c.id), 15)}
        <span class="label">${esc(shortName(c.id, d))}</span>
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
  return `<button class="btn btn-${kind}${opts.cls ? ` ${opts.cls}` : ""}"${attrs(data)}${opts.disabled ? " disabled" : ""}${opts.title ? ` title="${esc(opts.title)}"` : ""}>${opts.icon ? icon(opts.icon, 13) : ""}<span>${esc(label)}</span></button>`;
}

export function select(opts: { change: string; value: string | number; options: { value: string | number; label: string }[]; data?: Record<string, string | number>; width?: number; disabled?: boolean; mono?: boolean; label?: string }): string {
  const data: Record<string, string | number> = { "data-change": opts.change };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  const items = opts.options
    .map((o) => `<option value="${esc(String(o.value))}"${String(o.value) === String(opts.value) ? " selected" : ""}>${esc(o.label)}</option>`)
    .join("");
  return `<select class="select${opts.mono ? " mono" : ""}"${attrs(data)}${opts.width ? ` style="width:${opts.width}px"` : ""}${opts.disabled ? " disabled" : ""}${opts.label ? ` aria-label="${esc(opts.label)}"` : ""}>${items}</select>`;
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
    <span class="val">${esc(opts.label)}</span>
    <span class="btns">
      <button data-action="step" data-delta="1"${attrs(data)}${opts.value >= opts.max ? " disabled" : ""} aria-label="增加">${icon("chevron-up", 10)}</button>
      <button data-action="step" data-delta="-1"${attrs(data)}${opts.value <= opts.min ? " disabled" : ""} aria-label="减少">${icon("chevron-down", 10)}</button>
    </span>
  </span>`;
}

export function labelled(title: string, control: string): string {
  return `<span class="labelled"><span class="engraved">${esc(title)}</span>${control}</span>`;
}

export function menuButton(id: string, open: string | null, label: string, items: string, opts: { iconOnly?: boolean; left?: boolean; ariaLabel?: string } = {}): string {
  const isOpen = open === id;
  return `<span class="anchor">
    <button class="menu-trigger${opts.iconOnly ? " icon-only" : ""}${isOpen ? " open" : ""}" data-action="menu" data-id="${esc(id)}"${opts.ariaLabel ? ` aria-label="${esc(opts.ariaLabel)}"` : ""} aria-expanded="${isOpen}">${label}</button>
    ${isOpen ? `<div class="menu${opts.left ? " left" : ""}" role="menu">${items}</div>` : ""}
  </span>`;
}

export function menuItem(label: string, action: string, opts: { data?: Record<string, string | number>; icon?: string; danger?: boolean; checked?: boolean } = {}): string {
  const data: Record<string, string | number> = { "data-action": action };
  for (const [k, v] of Object.entries(opts.data ?? {})) data[`data-${k}`] = v;
  const lead = opts.checked !== undefined ? `<span class="check">${opts.checked ? icon("check", 12) : ""}</span>` : opts.icon ? icon(opts.icon, 14) : "";
  return `<button class="menu-item${opts.danger ? " danger" : ""}" role="menuitem"${attrs(data)}>${lead}<span>${esc(label)}</span></button>`;
}

export function sessionRow(entry: LedgerEntry, d: Day): string {
  const startedAt = entry.started_at > 0 ? entry.started_at : entry.ended_at - entry.seconds;
  const definition = def(entry.category);
  const name = d.categories.find((c) => c.id === entry.category)?.name ?? definition?.name ?? entry.category;
  const tasks = entry.tasks.length
    ? `<div class="session-tasks">${entry.tasks
        .map((t) => `<span class="${t.done ? "done" : ""}">${icon(t.done ? "check" : "minus", 11)}${esc(t.text)}</span>`)
        .join("")}</div>`
    : "";
  return `<div class="session${entry.accepted ? "" : " rejected"}">
    ${icon(entry.accepted ? "check" : "x", 13)}
    <div class="body">
      <div class="meta">
        <span class="nm">${esc(name)}</span>
        <span class="dur">${esc(duration(entry.seconds))}</span>
        <span class="at">${esc(wallClock(startedAt))}–${esc(wallClock(entry.ended_at))}</span>
        ${entry.accepted ? "" : `<span class="badge">未计入</span>`}
      </div>
      ${tasks}
    </div>
  </div>`;
}

export function emptyState(iconName: string, title: string, detail: string): string {
  return `<div class="empty">${icon(iconName, 30)}<b>${esc(title)}</b><p>${esc(detail)}</p></div>`;
}
