// 今天的时间轴：**一条**横轨，不再是每个项目一行。
//
// 上一版是 Bildfahrplan（每项目一轨 + 一条暂停轨）。它的毛病是实测出来的：
// 行多了就对不上横坐标，结束一格立刻进暂停、隔一两分钟再开下一格，于是每两格之间
// 都被暂停轨切一刀，越用越花；而且只在整点写字，中间大片空白。
//
// 现在的规矩：**一段就是一块，块上直接写名字**，暂停不画任何东西——就是空隙。
// 谁也不用再拿行标去对横轴。

import type { Day } from "../types";
import { activeSpans } from "../timeline";

// 与 styles.css 的令牌同值：canvas 读不到 CSS 变量，只能在这里再写一遍。
const INK_FAINT = "#a79d8b";
const INK_HIGH = "#f4edde";
const EMBER = "#cd6145";
const EMBER_INK = "#fff5e7";
const CAUTION = "#c6a367";
const BED = "#141411";

/** 轨高、轴高、总高：界面那边要拿总高定容器，别两处各写一套。 */
const TRACK_H = 46;
const AXIS_H = 22;
const TOP_PAD = 12;
export const DIAGRAM_H = TOP_PAD + TRACK_H + AXIS_H;

type Kind = "counted" | "dropped" | "live" | "held" | "rest";

interface Block {
  start: number;
  end: number;
  kind: Kind;
  name: string;
  seconds: number;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** 和 format.ts 的 meter 同一写法、同样向下取整（canvas 这边不引界面模块）。 */
function meterText(seconds: number): string {
  const minutes = Math.floor(Math.max(0, seconds) / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h${pad2(m)}`;
}

export function drawDiagram(
  canvas: HTMLCanvasElement,
  day: Day,
  now: number,
  endedAt: number | null,
  nameOfCategory: (id: string) => string
): void {
  const cssW = Math.max(160, Math.floor(canvas.getBoundingClientRect().width || canvas.parentElement?.clientWidth || 600));
  const cssH = DIAGRAM_H;
  const dpr = window.devicePixelRatio || 1;
  const pxW = Math.round(cssW * dpr);
  const pxH = Math.round(cssH * dpr);
  if (canvas.width !== pxW || canvas.height !== pxH) {
    canvas.width = pxW;
    canvas.height = pxH;
  }
  if (canvas.style.height !== `${cssH}px`) canvas.style.height = `${cssH}px`;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssW, cssH);

  const end = endedAt ?? now;
  const span = Math.max(1800, end - day.started_at);
  const usable = cssW;
  const x = (t: number) => Math.max(0, Math.min(usable, (usable * (t - day.started_at)) / span));
  const crisp = (v: number) => Math.round(v) + 0.5;
  const trackTop = TOP_PAD;
  const trackBottom = TOP_PAD + TRACK_H;

  // 1. 轨槽：整条轨先铺一层底，正中一道虚线是「香没点着」的那些时候；块画上去就把它盖住了。
  ctx.fillStyle = "rgba(231,223,208,0.03)";
  ctx.fillRect(0, trackTop, usable, TRACK_H);
  ctx.strokeStyle = "rgba(231,223,208,0.22)";
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 4]);
  ctx.beginPath();
  ctx.moveTo(0, crisp(trackTop + TRACK_H / 2));
  ctx.lineTo(usable, crisp(trackTop + TRACK_H / 2));
  ctx.stroke();
  ctx.setLineDash([]);

  // 2. 刻度：每 30 分钟一根，整点写字。上一版只标整点，中间一大片没有参照。
  const mark = new Date(day.started_at * 1000);
  mark.setMinutes(mark.getMinutes() >= 30 ? 60 : 30, 0, 0);
  const hours = span / 3600;
  // 一天拉长之后半点线会糊成一片，那时只留整点。
  const halfHours = hours <= 6;
  const hourStride = hours > 14 ? 2 : 1;
  ctx.font = `500 13px ui-monospace, "SF Mono", Menlo, monospace`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "center";
  const endLabelGap = ctx.measureText("00:00").width * 1.5 + 12;
  while (mark.getTime() / 1000 <= end) {
    const t = mark.getTime() / 1000;
    const onHour = mark.getMinutes() === 0;
    const px = crisp(x(t));
    if (onHour ? mark.getHours() % hourStride === 0 : halfHours) {
      ctx.strokeStyle = onHour ? "rgba(231,223,208,0.12)" : "rgba(231,223,208,0.06)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px, trackTop);
      ctx.lineTo(px, trackBottom + (onHour ? 5 : 2));
      ctx.stroke();
      // 两端已经写了起止钟点，靠得太近的整点就不写了，免得两个数字叠在一起。
      if (onHour && px > endLabelGap && px < cssW - endLabelGap) {
        ctx.fillStyle = INK_FAINT;
        ctx.fillText(`${pad2(mark.getHours())}:00`, px, trackBottom + 13);
      }
    }
    mark.setMinutes(mark.getMinutes() + 30);
  }

  // 3. 收集块。暂停不再是一段图形，它只是两块之间的空隙。
  const restingNow = day.break_until !== null && now < day.break_until;
  const lastPause = day.pauses[day.pauses.length - 1];
  const pausedNow = !!lastPause && lastPause.ended_at === null;
  const blocks: Block[] = [];
  for (const entry of day.ledger) {
    const start = entry.started_at > 0 ? entry.started_at : entry.ended_at - entry.seconds;
    const stop = Math.min(entry.ended_at, end);
    // 老记录缺起点时只保留原有的时长估计，不凭空推断缺失的暂停位置。
    const spans = entry.started_at > 0 ? activeSpans(start, stop, day.pauses) : [{ start, end: stop }];
    for (const span of spans) blocks.push({ ...span, kind: entry.accepted ? "counted" : "dropped", name: nameOfCategory(entry.category), seconds: span.end - span.start });
  }
  if (day.timer) {
    const t = day.timer;
    const stop = pausedNow ? Math.min(now, lastPause.started_at) : now;
    const start = t.started_at > 0 ? t.started_at : stop - t.elapsed_seconds;
    const spans = t.started_at > 0 ? activeSpans(start, stop, day.pauses) : [{ start, end: stop }];
    // 按停着的这一格还没计入（点「放弃」还会变成空心），和配额行、横香一样画成赭石，不冒充骨白的「计入」。
    for (const span of spans) if (span.end > span.start) blocks.push({ ...span, kind: pausedNow ? "held" : "live", name: nameOfCategory(t.category), seconds: span.end - span.start });
  }
  // 休息按记下的起止画，还在休息的那段画到此刻；休息结束后它照样是一块「休息」，不会退成空隙。
  let restOnRecord = false;
  for (const r of day.rests) {
    const stop = Math.min(r.ended_at, end);
    if (stop > r.started_at) blocks.push({ start: r.started_at, end: stop, kind: "rest", name: "休息", seconds: stop - r.started_at });
    if (r.started_at <= now && now < r.ended_at) restOnRecord = true;
  }
  // 升级前就开始的那段休息没有记录，只能照旧从这段暂停的起点画到此刻。
  if (restingNow && lastPause && !restOnRecord) {
    blocks.push({ start: lastPause.started_at, end: now, kind: "rest", name: "休息", seconds: now - lastPause.started_at });
  }
  blocks.sort((a, b) => a.start - b.start);

  // 4. 画块。名字写在块上——这就是「对不上是哪个项目」的解法。
  const barTop = trackTop + 5;
  const barH = TRACK_H - 10;
  for (const b of blocks) {
    const x1 = x(b.start);
    const x2 = x(b.end);
    const w = Math.max(2, x2 - x1);
    // 已计入是骨白的灰，正在烧的是朱红，按停着的是赭石，放弃的只留轮廓，休息是赭石的淡底。
    const fill =
      b.kind === "live" ? EMBER
      : b.kind === "held" ? CAUTION
      : b.kind === "rest" ? "rgba(198,163,103,0.28)"
      : b.kind === "dropped" ? BED
      : "rgba(231,223,208,0.8)";
    ctx.fillStyle = fill;
    ctx.beginPath();
    ctx.roundRect(x1, barTop, w, barH, 2);
    ctx.fill();
    if (b.kind === "dropped") {
      ctx.strokeStyle = "rgba(231,223,208,0.42)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.roundRect(crisp(x1), crisp(barTop), Math.round(w) - 1, Math.round(barH) - 1, 2);
      ctx.stroke();
    }
    // 块窄到写不下就不写，绝不画半个字。
    const label = `${b.name} ${meterText(b.seconds)}`;
    ctx.font = `500 14px -apple-system, "PingFang SC", sans-serif`;
    if (ctx.measureText(label).width + 14 <= w) {
      ctx.fillStyle = b.kind === "live" ? EMBER_INK : b.kind === "counted" || b.kind === "held" ? BED : INK_HIGH;
      ctx.textAlign = "left";
      ctx.fillText(label, x1 + 7, barTop + barH / 2);
    } else if (ctx.measureText(b.name).width + 14 <= w) {
      ctx.fillStyle = b.kind === "live" ? EMBER_INK : b.kind === "counted" || b.kind === "held" ? BED : INK_HIGH;
      ctx.textAlign = "left";
      ctx.fillText(b.name, x1 + 7, barTop + barH / 2);
    }
  }

  // 5. 此刻：轨的右端一根竖线。在烧是朱红；格被按停或在休息是赭石；两格之间是淡墨。
  //    上一版线头还顶着一个小方块，和别处那些要人去猜的方块一起去掉了。
  //    此刻就在轨的最右边，线收进画布里一点，不然有一半画在外面。
  if (endedAt === null) {
    const px = Math.min(cssW - 1, crisp(x(now)));
    const held = pausedNow || restingNow;
    ctx.strokeStyle = !held ? EMBER : restingNow || day.timer ? CAUTION : INK_FAINT;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(px, trackTop - 6);
    ctx.lineTo(px, trackBottom + 4);
    ctx.stroke();
  }

  // 6. 两端的钟点，写在轨下面，省得靠中间的刻度倒推一天从几点开始。
  ctx.font = `500 13px ui-monospace, "SF Mono", Menlo, monospace`;
  ctx.fillStyle = INK_FAINT;
  ctx.textAlign = "left";
  const from = new Date(day.started_at * 1000);
  ctx.fillText(`${pad2(from.getHours())}:${pad2(from.getMinutes())}`, 0, trackBottom + 13);
  ctx.textAlign = "right";
  const to = new Date(end * 1000);
  ctx.fillText(`${pad2(to.getHours())}:${pad2(to.getMinutes())}`, cssW, trackBottom + 13);
}
