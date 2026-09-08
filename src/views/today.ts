// 「今天」：没开始时是开始页（三档、两条规矩、上一天），开始后是运行台。

import type { BlockTimer, Day } from "../types";
import { clock, duration, esc, meter, wallClock } from "../format";
import { icon } from "../icons";
import {
  bigReading,
  btn,
  categoryPicker,
  hair,
  labelled,
  menuButton,
  menuItem,
  plate,
  quotaLane,
  runStrip,
  sectionLabel,
  select,
  sessionRow,
} from "../components";
import { blockMinutes, estimatedRemainingWallSeconds, remainingSeconds, suggest } from "../scheduler";
import {
  BLOCK_OPTIONS,
  BREAK_OPTIONS,
  breakRemaining,
  day,
  history,
  iconOf,
  isPaused,
  nameOf,
  netSeconds,
  pauseNowSeconds,
  pausedAutomatically,
  prefs,
  profileTotalMinutes,
  quotaSeconds,
  resting,
  ui,
  withValue,
} from "../state";

export function todayPage(): string {
  const d = day();
  return d ? activeBoard(d) : startBoard();
}

// ---------- 开始页 ----------

function startBoard(): string {
  const p = prefs();
  const plan = p.profiles[0];
  const days = history();
  const last = days.length ? days[days.length - 1] : null;
  if (!plan) {
    return `<div class="masthead"><h1 class="t-masthead">一天从你坐下的那一刻算起</h1>
      <p class="sub">先去「设置 · 项目」建一个项目。</p></div>`;
  }
  const rows = p.categories
    .map((c) => {
      const minutes = plan.quotas.find((q) => q.category === c.id)?.minutes ?? 0;
      const key = `${encodeURIComponent(plan.id)}:${encodeURIComponent(c.id)}`;
      const data = `data-profile="${esc(plan.id)}" data-category="${esc(c.id)}"`;
      const step = (delta: number, glyph: string, verb: string) =>
        `<button id="plan-${glyph}-${esc(key)}" data-action="step" data-bind="quota" data-delta="${delta}" data-step="15" data-min="0" data-max="1440" ${data} ${delta < 0 ? minutes <= 0 ? "disabled" : "" : minutes >= 1440 ? "disabled" : ""} aria-label="${esc(c.name)}${verb} 15 分钟">${icon(glyph, 12)}</button>`;
      return `<div class="plan-row${minutes === 0 ? " off" : ""}">
        <span class="who">${icon(c.icon, 15)}<span class="nm">${esc(c.name)}</span></span>
        <span class="edit">${step(-1, "minus", "减少")}<input id="plan-field-${esc(key)}" class="field plan-input" type="number" inputmode="numeric" min="0" max="1440" step="1" value="${esc(minutes)}" data-change="quota-minutes" ${data} aria-label="${esc(c.name)}今天的目标分钟数" />${step(1, "plus", "增加")}</span>
        <span class="hrs">${minutes === 0 ? "不排" : esc(meter(minutes * 60))}</span>
      </div>`;
    })
    .join("");
  const total = profileTotalMinutes(plan);
  return `<div class="masthead">
      <h1 class="t-masthead">一天从你坐下的那一刻算起</h1>
      <p class="sub">不看几点起床，也不在午夜清零。定好今天各项目的时间，然后坐下。</p>
    </div>
    <div class="plan">
      <div class="plan-head"><span class="engraved">今天的安排</span><span class="engraved">分钟</span></div>
      ${hair()}
      <div class="plan-rows">${rows}</div>
      ${hair()}
      <div class="plan-total"><span class="engraved">总目标</span><span class="mono num">${esc(meter(total * 60))}</span></div>
      <div class="plan-go">${btn("开始今天", { kind: "primary", cls: "lg", action: "start-day", data: { id: plan.id } })}</div>
    </div>
    ${last ? yesterdayBlock(last.day) : ""}`;
}

function yesterdayBlock(d: Day): string {
  const lanes = d.categories.map((c) => quotaLane(c, { day: d })).join("");
  return `<div class="yesterday">
    ${sectionLabel("上一天", { note: new Date(d.started_at * 1000).toLocaleDateString("zh-CN", { month: "long", day: "numeric" }), trailing: `<span class="mono high" style="font-size:15px;font-weight:500">${esc(meter(netSeconds(d)))}</span>` })}
    <div class="grid">${lanes}</div>
  </div>`;
}

// ---------- 运行台 ----------

/**
 * 人正停在「选下一格」这块板上。0.8.0 之后「没有格在走」⇔「一定在暂停」，
 * 所以这里只能看 timer——再去 && !isPaused 会恒为 false，
 * 把配额轨上的建议竖标和收工预估一起关掉。
 */
function inChooser(d: Day): boolean {
  return !d.timer;
}

function activeBoard(d: Day): string {
  const p = prefs();
  const suggestion = inChooser(d) ? suggest(d, p, ui.now) : null;
  // 方案 A：上排「当前这一格」｜「配额 + 身体」，运行图横跨两栏。
  // 三块都是 .columns 的**直接**子元素——中间再套一层 div 的话，
  // 左板就没法跟右栏在同一行里拉伸到齐平。
  const right = `<div class="rail" id="col-right">${quotaPanel(d, suggestion?.category ?? null)}${bodyPanel(d)}</div>`;
  const wide = `<div class="wide" id="col-wide">${diagramBlock()}</div>`;
  // 有格的时候（在走或被按停）上排两块拉到齐平；「选下一格」这块板内容少，
  // 硬拉就是在它下面挖一个三百来像素的坑，那时按自然高度。
  const taut = d.timer ? " taut" : "";
  return `${headerBar(d)}<div class="columns${taut}" id="columns">${focusPanel(d)}${right}${wide}</div>${d.ledger.length ? logBlock(d) : ""}`;
}

function headerBar(d: Day): string {
  const net = netSeconds(d);
  const paused = isPaused(d);
  const pausedTotal = d.paused_seconds + (paused ? pauseNowSeconds(d) : 0);
  const pausedReading = pausedTotal > 0
    ? `<i class="divider"></i><div class="net secondary"><span class="engraved">已暂停</span><div class="figure"><span class="val">${esc(meter(pausedTotal))}</span></div></div>`
    : "";
  const readings = `<div class="readings">
      <div class="net"><span class="engraved">从 ${esc(wallClock(d.started_at))} 坐下</span><div class="figure"><span class="val">${esc(meter(net))}</span><span class="of">/ ${esc(meter(quotaSeconds(d)))}</span></div></div>
      ${pausedReading}
    </div>`;
  const moreItems = `${menuItem("复制今天的 Markdown 总结", "copy-today-md")}<div class="menu-sep"></div>${menuItem("返回开始页…", "discard-day", { danger: true })}${menuItem("收工归档…", "end-day", { danger: true })}`;
  const controls = `<div class="controls">
      ${menuButton("more", ui.menu, icon("ellipsis-vertical", 16), moreItems, { iconOnly: true, ariaLabel: "更多操作" })}
    </div>`;
  return `<section class="plate header-bar" id="header-bar">${readings}${controls}</section>`;
}

function focusPanel(d: Day): string {
  let inner: string;
  if (d.timer && isPaused(d)) inner = pausedPanel(d, d.timer);
  else if (d.timer) inner = runningPanel(d, d.timer);
  else inner = nextBlockPanel(d);
  return `<section class="plate focus${d.timer && !isPaused(d) ? " is-running" : ""}" id="focus"><div class="inner">${inner}</div></section>`;
}

/** 格在走、但被按停了。 */
function pausedPanel(d: Day, t: BlockTimer): string {
  const auto = pausedAutomatically(d);
  const why = auto ? "检测到计时中断，已自动暂停。" : `继续后「${nameOf(t.category, d)}」接着走。`;
  const left = Math.max(0, t.total_seconds - t.elapsed_seconds);
  return `<div class="panel-head">${icon("pause", 15, "caution")}<span class="name">${esc(nameOf(t.category, d))} · 已暂停</span><span class="right t-caption">还剩 ${esc(duration(left))}</span></div>
    ${hair()}
    <div class="away-body">${bigReading(clock(pauseNowSeconds(d)), "已暂停", { tint: "ink-muted" })}<span class="t-caption" style="font-size:13px">${esc(why)}</span></div>
    <div class="spacer"></div>
    <div class="btn-row">${btn("继续", { kind: "primary", action: "toggle", icon: "play" })}${btn("结束这一格", { kind: "plate", action: "finish" })}</div>`;
}

/** 计时中的任务清单：可勾、可加，也可以一条都没有。 */
function taskList(t: BlockTimer): string {
  const rows = t.tasks
    .map(
      (task, i) =>
        `<button class="task-row${task.done ? " done" : ""}" data-action="toggle-task" data-index="${i}" aria-pressed="${task.done}">
          <span class="box">${task.done ? icon("check", 11) : ""}</span>
          <span class="text">${esc(task.text)}</span>
        </button>`
    )
    .join("");
  const adder = ui.addingTask
    ? `<div class="task-add"><input class="field xs" data-input="add-task" value="${esc(ui.addTaskDraft)}" placeholder="再加一条，回车确认" />${btn("加上", { kind: "quiet", cls: "body sm", action: "commit-task", disabled: !ui.addTaskDraft.trim() })}</div>`
    : `<button class="task-more" data-action="add-task">${icon("plus", 11)}<span>加一条</span></button>`;
  return `<div class="tasks">${rows}${adder}</div>`;
}

function runningPanel(d: Day, t: BlockTimer): string {
  const remaining = Math.max(0, t.total_seconds - t.elapsed_seconds);
  const startedAt = t.started_at > 0 ? t.started_at : ui.now - t.elapsed_seconds;
  const buttons = [
    btn("暂停", { kind: "plate", action: "toggle", icon: "pause" }),
    btn("+10 分钟", { kind: "plate", action: "extend" }),
    btn("结束这一格", { kind: "primary", action: "finish" }),
    btn("放弃", { kind: "quiet", action: "abandon-block" }),
  ].join("");
  return `<div class="panel-head">${icon(iconOf(t.category), 15, "signal")}<span class="name">${esc(nameOf(t.category, d))}</span></div>
    ${hair()}
    <div class="bigreading-pad">${bigReading(clock(remaining), "剩余")}</div>
    ${runStrip({ elapsed: t.elapsed_seconds, planned: t.total_seconds, running: true, focus: true, startedAt, projectedEnd: ui.now + remaining })}
    ${taskList(t)}
    <div class="spacer" style="min-height:16px"></div>
    <div class="btn-row">${buttons}</div>`;
}

function nextBlockPanel(d: Day): string {
  const p = prefs();
  const restingNow = resting(d);
  const suggestion = suggest(d, p, ui.now);
  if (!suggestion) return completeBlock(d);
  const selected = ui.selectedCategory && d.categories.some((c) => c.id === ui.selectedCategory) ? ui.selectedCategory : suggestion.category;
  const cat = d.categories.find((c) => c.id === selected)!;
  const minutes = ui.minutesDraft ?? (selected === suggestion.category ? suggestion.minutes : blockMinutes(cat, p));
  const brk = ui.breakDraft ?? p.break_minutes;
  const pausedStrip = restingNow
    ? `<div class="pause-strip resting">${icon("coffee", 13)}<span>休息中，还剩 <b class="mono">${esc(clock(breakRemaining(d)))}</b></span>${btn("不休息了", { kind: "quiet", cls: "sm", action: "end-break" })}</div>`
    : `<div class="pause-strip">${icon("pause", 13)}<span>已暂停 <b class="mono">${esc(clock(pauseNowSeconds(d)))}</b>，开一格就继续</span></div>`;
  const suggestionLine = `<div class="suggestion${suggestion.pressing ? " pressing" : ""}">
      ${icon(suggestion.pressing ? "triangle-alert" : "signpost", 14)}
      <div class="text"><b>建议 ${esc(nameOf(suggestion.category, d))} · ${suggestion.minutes} 分钟</b><span>${esc(suggestion.reason)}</span></div>
      ${selected !== suggestion.category ? btn(`选${nameOf(suggestion.category, d)}`, { kind: "quiet", cls: "high", action: "adopt" }) : ""}
    </div>`;
  const task = `<div class="task">
      <textarea class="field task-input" data-input="task" rows="2" aria-label="这一格要做的事" placeholder="要做的事，一行一条（可以不写）">${esc(ui.taskDraft)}</textarea>
    </div>`;
  const controls = `<div class="controls-row">
      ${labelled("时长", select({ change: "minutes", value: minutes, options: withValue(BLOCK_OPTIONS, minutes).map((n) => ({ value: n, label: `${n} 分` })), width: 84, label: "专注时长" }))}
      ${labelled("之后休息", select({ change: "break", value: brk, options: withValue(BREAK_OPTIONS, brk).map((n) => ({ value: n, label: n === 0 ? "不休息" : `${n} 分` })), width: 92, label: "休息时长" }))}
      <span class="start">${btn("开始", { kind: "primary", cls: "lg", action: "start-block", title: "⌘↩" })}</span>
    </div>`;
  return `<div class="block-body">${pausedStrip}${suggestionLine}${categoryPicker(d, selected)}${task}<div class="spacer"></div>${controls}</div>`;
}

function completeBlock(d: Day): string {
  return `<div class="panel-head">${icon("check", 15, "stroke")}<span class="name">${esc(d.profile_name)}档今天的配额，全部满了</span></div>
    ${hair()}
    <div class="complete-body">${bigReading(meter(netSeconds(d)), "已学")}<span class="t-caption">可以收工，也可以换个更高的档继续。</span></div>
    <div class="spacer"></div>
    <div class="btn-row">${btn("收工归档…", { kind: "primary", action: "end-day" })}</div>`;
}

function quotaPanel(d: Day, marked: string | null): string {
  const p = prefs();
  const selectable = inChooser(d) && !!marked;
  const lanes = d.categories.map((c) => quotaLane(c, { marked: marked === c.id, pulsing: ui.creditPulse === c.id, day: d, selectable, selected: selectable && ui.selectedCategory === c.id })).join("");
  let estimate = "";
  if (inChooser(d)) {
    const remaining = d.categories.reduce((sum, c) => sum + remainingSeconds(c), 0);
    if (remaining > 0) {
      const wall = estimatedRemainingWallSeconds(d, p);
      estimate = `${hair("mt8")}<p class="estimate">还要 ${esc(duration(wall))}，约 ${esc(wallClock(ui.now + wall))} 收工</p>`;
    }
  }
  return plate(`<div class="quotas">${sectionLabel("今日配额", { trailing: selectable ? `<span class="t-note">点击选下一格</span>` : "" })}${lanes}${estimate}</div>`);
}

function bodyPanel(d: Day): string {
  const p = prefs();
  const goal = Math.max(1, p.hydration_goal_cups);
  const cups = Array.from({ length: Math.max(goal, d.cups) }, (_, i) => `<i class="cup${i < d.cups ? " on" : ""}"></i>`).join("");
  const hydration = `<div class="hydration">${icon("droplet", 14)}<div class="cups">${cups}</div><span class="count">${d.cups}/${goal}</span>
      <button class="btn-icon" data-action="water" title="记一杯水" aria-label="记一杯水">${icon("plus", 12)}</button>
      ${d.cups > 0 ? `<button class="btn-icon" data-action="water-undo" title="撤销一杯" aria-label="撤销一杯水">${icon("minus", 12)}</button>` : ""}
    </div>`;
  return plate(`<div class="body-panel">${sectionLabel("喝水")}${hydration}</div>`);
}

function diagramBlock(): string {
  // 名字写在块上了，所以不再有行标列；图例也只剩三样，暂停就是空隙不用图例。
  const legend = `<span class="legend"><span><i></i>计入</span><span><i class="hollow"></i>未计入</span><span><i class="rest"></i>休息</span></span>`;
  const inner = `<div class="diagram-block">
    ${sectionLabel("今天的走向", { trailing: legend })}
    <div class="diagram">
      <canvas class="diagram-canvas" data-diagram="today" aria-label="今天的时间轴"></canvas>
    </div>
  </div>`;
  return plate(inner);
}

function logBlock(d: Day): string {
  const recent = d.ledger.slice(-6).reverse();
  const rows = recent.map((l) => sessionRow(l, d)).join(hair());
  return plate(`<div class="log">${sectionLabel("最近记录", { trailing: `<span class="t-note">${d.ledger.length} 格</span>` })}<div class="rows">${rows}</div></div>`);
}
