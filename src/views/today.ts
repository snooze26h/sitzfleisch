// 「今天」：没开始时是开始页（左栏大标题、当天的安排、总目标和开始键，右栏一弯新月），开始后是运行台。
// 运行台前两行是两列：这一格（倒计时、横卧的这炷香、操作）| 今天这一轮月，
// 今日配额（每个项目一行横香）| 今天的总读数；下面是今天的走向和最近记录。不套板子，靠细线和留白分区。

import type { BlockTimer, Day } from "../types";
import { MAX_BLOCK_MINUTES, MIN_BLOCK_MINUTES } from "../types";
import { clock, duration, esc, meter, wallClock } from "../format";
import { icon } from "../icons";
import { crescentArt, dayMoon } from "../moon";
import {
  bigReading,
  blockStick,
  btn,
  categoryPicker,
  labelled,
  menuButton,
  menuItem,
  minuteInput,
  quotaRows,
  sectionLabel,
  select,
  sessionRow,
} from "../components";
import { blockMinutes, estimatedRemainingWallSeconds, remainingSeconds, suggest } from "../scheduler";
import {
  BREAK_OPTIONS,
  breakRemaining,
  day,
  idleNowSeconds,
  iconOf,
  isPaused,
  nameOf,
  netSeconds,
  pauseNowSeconds,
  pausedAutomatically,
  prefs,
  profileTotalMinutes,
  quotaSeconds,
  restSeconds,
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
  if (!plan) {
    return `<header class="masthead"><h1 class="t-masthead">落座，便是今天</h1>
      <p class="sub">先在设置中添加项目</p></header>`;
  }
  const rows = p.categories
    .map((c) => {
      const minutes = plan.quotas.find((q) => q.category === c.id)?.minutes ?? 0;
      const key = `${encodeURIComponent(plan.id)}:${encodeURIComponent(c.id)}`;
      const data = `data-profile="${esc(plan.id)}" data-category="${esc(c.id)}"`;
      const step = (delta: number, glyph: string, verb: string) =>
        `<button id="plan-${glyph}-${esc(key)}" data-action="step" data-bind="quota" data-delta="${delta}" data-step="15" data-min="0" data-max="1440" ${data} ${delta < 0 ? minutes <= 0 ? "disabled" : "" : minutes >= 1440 ? "disabled" : ""} aria-label="${esc(c.name)}${verb} 15 分钟">${icon(glyph, 13)}</button>`;
      return `<div class="plan-row${minutes === 0 ? " off" : ""}">
        <span class="who">${icon(c.icon, 20)}<span class="nm">${esc(c.name)}</span></span>
        <span class="edit">${step(-1, "minus", "减少")}<input id="plan-field-${esc(key)}" class="field plan-input" type="number" inputmode="numeric" min="0" max="1440" step="1" value="${esc(minutes)}" data-change="quota-minutes" ${data} aria-label="${esc(c.name)}今天的目标分钟数" />${step(1, "plus", "增加")}</span>
        <span class="hrs">${minutes === 0 ? "不排" : esc(meter(minutes * 60))}</span>
      </div>`;
    })
    .join("");
  const total = profileTotalMinutes(plan);
  return `<section class="start-board" id="start">
      <header class="masthead">
        <h1 class="t-masthead">落座，便是今天</h1>
      </header>
      <div class="plan">
        <div class="plan-head"><span class="engraved">今天的安排</span><span class="engraved">分钟</span></div>
        <div class="plan-rows">${rows}</div>
      </div>
      <div class="plan-foot">
        <div class="plan-total"><span class="engraved">总目标</span><span class="num">${esc(meter(total * 60))}</span></div>
        <div class="plan-go">${btn("开始今天", { kind: "primary", cls: "lg", action: "start-day", data: { id: plan.id }, disabled: ui.startingDay })}</div>
      </div>
      ${crescentArt("start-art")}
    </section>`;
}

// ---------- 运行台 ----------

/**
 * 人正停在「选下一格」这块板上。0.8.0 之后「没有格在走」⇔「一定在暂停」，
 * 所以这里只能看 timer——再去 && !isPaused 会恒为 false，
 * 把配额上的建议记号和收工预估一起关掉。
 */
function inChooser(d: Day): boolean {
  return !d.timer;
}

function activeBoard(d: Day): string {
  const suggestion = inChooser(d) ? suggest(d, prefs(), ui.now) : null;
  // 四块排成两行两列：这一格 | 今天这一轮月，今日配额 | 今天的总读数。
  // 左右两列上下各自对齐，窗口第一屏里放得下「此刻」和「今天」两个尺度。
  const moon = `<div class="moon-cell moon-stage" id="day-moon">${dayMoon(d)}</div>`;
  const notificationWarning = ui.platform?.mobile && ui.systemStatus?.notificationsEnabled === false
    ? `<div class="notification-warning"><b>通知未开启，锁屏后到点不会提醒</b>${btn("去开启", { kind: "plate", action: "system-settings", data: { target: "app_notifications" } })}</div>` : "";
  return `<section class="today-top${ui.compact && suggestion ? " choosing" : ""}${ui.compact && notificationWarning ? " with-warning" : ""}" id="today-top">${focusPanel(d)}${moon}${quotaPanel(d, suggestion?.category ?? null)}${dayReadings(d)}${notificationWarning}</section>
    ${diagramBlock()}
    ${d.ledger.length ? logBlock(d) : ""}`;
}

/** 月亮下面：「从几点坐下」这一天已学多少、停了多久、休息了多久，外加今天这一天的菜单。 */
function dayReadings(d: Day): string {
  // 核心心跳已经累计当前暂停段，界面直接使用总账，不能再叠加一次。
  // 休息也落在暂停里：这里拆开写，计划好的休息不算成「停下来」。不满一分钟的不写，免得冒出一个 0m。
  const rest = restSeconds(d);
  const secondary = (label: string, seconds: number) => seconds >= 60
    ? `<div class="net secondary"><span class="engraved">${label}</span><div class="figure"><span class="val">${esc(meter(seconds))}</span></div></div>`
    : "";
  const paused = secondary("已暂停", Math.max(0, d.paused_seconds - rest)) + secondary("已休息", rest);
  const moreItems = `${menuItem("复制今天的 Markdown 总结", "copy-today-md")}<div class="menu-sep"></div>${menuItem("返回开始页…", "discard-day", { danger: true })}${menuItem("收工归档…", "end-day", { danger: true })}`;
  if (ui.compact) {
    // 钟点与菜单通栏，给右侧的 40px / 28px 读数留足宽度。
    return `<div class="day-heading"><header class="section-head"><h2 class="section-title">从 ${esc(wallClock(d.started_at))} 坐下</h2><span class="trail">${menuButton("more", ui.menu, icon("ellipsis-vertical", 17), moreItems, { iconOnly: true, ariaLabel: "更多操作" })}</span></header></div>
      <aside class="day-readings" id="day-readings"><div class="net"><span class="engraved">已学</span><div class="figure"><span class="val">${esc(meter(netSeconds(d)))}</span><span class="of">/ ${esc(meter(quotaSeconds(d)))}</span></div></div>${paused}</aside>`;
  }
  return `<aside class="day-readings" id="day-readings">
    <header class="section-head"><h2 class="section-title">从 ${esc(wallClock(d.started_at))} 坐下</h2><span class="trail">${menuButton("more", ui.menu, icon("ellipsis-vertical", 17), moreItems, { iconOnly: true, ariaLabel: "更多操作" })}</span></header>
    <div class="net"><span class="engraved">已学</span><div class="figure"><span class="val">${esc(meter(netSeconds(d)))}</span><span class="of">/ ${esc(meter(quotaSeconds(d)))}</span></div></div>
    ${paused}
  </aside>`;
}

function focusPanel(d: Day): string {
  let inner: string;
  let phase: string;
  if (d.timer && isPaused(d)) inner = pausedPanel(d, d.timer);
  else if (d.timer) inner = runningPanel(d, d.timer);
  else inner = nextBlockPanel(d);
  if (d.timer) phase = isPaused(d) ? "paused" : "flow";
  else if (!suggest(d, prefs(), ui.now)) phase = "done";
  else phase = resting(d) ? "rest" : "ready";
  return `<section class="focus" id="focus" data-phase="${phase}">${inner}</section>`;
}

/** 格在走、但被按停了。 */
function pausedPanel(d: Day, t: BlockTimer): string {
  // 手动按停时标题、读数和「继续」键已经说全了；只有自动按停要多交代一句原因。
  const why = pausedAutomatically(d) ? `<p class="why">计时中断，已自动暂停。</p>` : "";
  const left = Math.max(0, t.total_seconds - t.elapsed_seconds);
  const startedAt = t.started_at > 0 ? t.started_at : ui.now - t.elapsed_seconds;
  return `<div class="focus-head">${icon("pause", 18, "caution")}<span class="name">${esc(nameOf(t.category, d))} · 已暂停</span><span class="right">还剩 ${esc(duration(left))}</span></div>
    <div class="focus-reading">${bigReading(clock(pauseNowSeconds(d)), "已暂停", { tint: "caution" })}${why}</div>
    ${blockStick({ elapsed: t.elapsed_seconds, planned: t.total_seconds, running: false, startedAt, projectedEnd: ui.now + left })}
    <div class="btn-row">${btn("继续", { kind: "primary", action: "toggle", icon: "play" })}${btn("延长…", { kind: "plate", action: "extend", disabled: t.total_seconds >= MAX_BLOCK_MINUTES * 120 })}${btn("结束这一格", { kind: "plate", action: "finish" })}</div>`;
}

function runningPanel(d: Day, t: BlockTimer): string {
  const remaining = Math.max(0, t.total_seconds - t.elapsed_seconds);
  const startedAt = t.started_at > 0 ? t.started_at : ui.now - t.elapsed_seconds;
  return `<div class="focus-head">${icon(iconOf(t.category), 20)}<span class="name">${esc(nameOf(t.category, d))}</span></div>
    <div class="focus-reading">${bigReading(clock(remaining), "剩余")}</div>
    ${blockStick({ elapsed: t.elapsed_seconds, planned: t.total_seconds, running: true, startedAt, projectedEnd: ui.now + remaining })}
    <div class="btn-row">
      ${btn("暂停", { kind: "plate", action: "toggle", icon: "pause" })}
      ${btn("延长…", { kind: "plate", action: "extend", disabled: t.total_seconds >= MAX_BLOCK_MINUTES * 120 })}
      <span class="spacer"></span>
      ${btn("结束这一格", { kind: "primary", action: "finish" })}
      ${btn("放弃", { kind: "quiet", action: "abandon-block" })}
    </div>`;
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
  // 头上这一行说清现在是停着还是在休息：图标就是暂停键和茶杯，不用要人去猜的小方块。
  const pausedStrip = restingNow
    ? `<div class="pause-strip resting">${icon("coffee", 18)}<span>休息中，还剩 <b class="mono">${esc(clock(breakRemaining(d)))}</b></span>${btn("不休息了", { kind: "quiet", cls: "sm", action: "end-break" })}</div>`
    : `<div class="pause-strip">${icon("pause", 18)}<span>已暂停 <b class="mono">${esc(clock(idleNowSeconds(d)))}</b></span></div>`;
  const suggestionLine = `<div class="suggestion${suggestion.pressing ? " pressing" : ""}">
      ${suggestion.pressing ? icon("triangle-alert", 18) : ""}
      <div class="text"><b>建议 ${esc(nameOf(suggestion.category, d))} · ${suggestion.minutes} 分钟</b><span>${esc(suggestion.reason)}</span></div>
    </div>`;
  const controls = `<div class="controls-row">
      ${labelled("时长", minuteInput({ change: "minutes", value: minutes, min: MIN_BLOCK_MINUTES, max: MAX_BLOCK_MINUTES, label: "专注时长" }))}
      ${labelled("之后休息", select({ change: "break", value: brk, options: withValue(BREAK_OPTIONS, brk).map((n) => ({ value: n, label: n === 0 ? "不休息" : `${n} 分` })), width: 104, label: "休息时长" }))}
      <span class="start">${btn("开始", { kind: "primary", cls: "lg", action: "start-block", title: "⌘↩" })}</span>
    </div>`;
  return `<div class="block-body">${pausedStrip}${suggestionLine}${categoryPicker(d, selected)}${controls}</div>`;
}

function completeBlock(d: Day): string {
  return `<div class="focus-head">${icon("check", 18, "stroke")}<span class="name">今天的安排全部走完了</span></div>
    <div class="focus-reading">${bigReading(meter(netSeconds(d)), "已学")}</div>
    <div class="btn-row">${btn("收工归档…", { kind: "primary", action: "end-day" })}</div>`;
}

function quotaPanel(d: Day, suggested: string | null): string {
  const p = prefs();
  const selectable = inChooser(d) && !!suggested;
  // 和上面的项目选择器同一个选中项：没手动挑过就是建议的那个。
  const chosen = ui.selectedCategory && d.categories.some((c) => c.id === ui.selectedCategory) ? ui.selectedCategory : suggested;
  const t = d.timer;
  let estimate = "";
  if (inChooser(d)) {
    const remaining = d.categories.reduce((sum, c) => sum + remainingSeconds(c), 0);
    if (remaining > 0) {
      const wall = estimatedRemainingWallSeconds(d, p);
      estimate = `<p class="estimate">还要 ${esc(duration(wall))}，约 ${esc(wallClock(ui.now + wall))} 收工</p>`;
    }
  }
  const rows = quotaRows(d, {
    pulse: ui.creditPulse,
    selectable,
    selected: chosen,
    active: t ? { category: t.category, elapsed: t.elapsed_seconds, held: isPaused(d) } : null,
  });
  return `<section class="quota-block" id="quota">${sectionLabel("今日配额", { trailing: selectable ? `<span class="t-note">点击选下一格</span>` : "" })}${rows}${estimate}</section>`;
}

function diagramBlock(): string {
  // 名字写在块上了，所以不再有行标列；图例也只剩三样，暂停就是空隙不用图例。
  const legend = `<span class="legend"><span><i></i>计入</span><span><i class="hollow"></i>未计入</span><span><i class="rest"></i>休息</span></span>`;
  return `<section class="diagram-block" id="course">
    ${sectionLabel("今天的走向", { trailing: legend })}
    <div class="diagram"><canvas class="diagram-canvas" data-diagram="today" aria-label="今天的时间轴"></canvas></div>
  </section>`;
}

function logBlock(d: Day): string {
  const recent = d.ledger.slice(-6).reverse();
  return `<section class="log" id="log">${sectionLabel("最近记录", { trailing: `<span class="t-note">${d.ledger.length} 格</span>` })}<div class="rows">${recent.map((l) => sessionRow(l, d)).join("")}</div></section>`;
}
