// 今天的时间轴：**一条**横轨，不再是每个项目一行。
//
// 上一版是 Bildfahrplan（每项目一轨 + 一条暂停轨）。它的毛病是实测出来的：
// 行多了就对不上横坐标，结束一格立刻进暂停、隔一两分钟再开下一格，于是每两格之间
// 都被暂停轨切一刀，越用越花；而且只在整点写字，中间大片空白。
//
// 现在的规矩：**一段就是一块，块上直接写名字**，暂停不画任何东西——就是空隙。
// 谁也不用再拿行标去对横轴。

import type { Day } from "../types";

const INK_FAINT = "#9d978d";
const INK_HIGH = "#f4f2ee";
const STROKE = "#dad5ca";
const SIGNAL = "#c7452f";
const CAUTION = "#b08843";
const BED = "#0e0e10";

/** 轨高、轴高、总高：界面那边要拿总高定容器，别两处各写一套。 */
const TRACK_H = 46;
const AXIS_H = 22;
const TOP_PAD = 8;
export const DIAGRAM_H = TOP_PAD + TRACK_H + AXIS_H;

type Kind = "counted" | "dropped" | "live" | "rest";

interface Block {
  start: number;
  end: number;
  kind: Kind;
  name: string;
  seconds: number;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

function meterText(seconds: number): string {
  const minutes = Math.round(Math.max(0, seconds) / 60);
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

  // 1. 轨槽：整条轨先铺一层底，空隙自然就是「没在跑」。
  ctx.fillStyle = "rgba(255,255,255,0.035)";
  ctx.fillRect(0, trackTop, usable, TRACK_H);

  // 2. 刻度：每 30 分钟一根，整点写字。上一版只标整点，中间一大片没有参照。
  const mark = new Date(day.started_at * 1000);
  mark.setMinutes(mark.getMinutes() >= 30 ? 60 : 30, 0, 0);
  const hours = span / 3600;
  // 一天拉长之后半点线会糊成一片，那时只留整点。
  const halfHours = hours <= 6;
  const hourStride = hours > 14 ? 2 : 1;
  ctx.font = `500 11px ui-monospace, "SF Mono", Menlo, monospace`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "center";
  while (mark.getTime() / 1000 <= end) {
    const t = mark.getTime() / 1000;
    const onHour = mark.getMinutes() === 0;
    const px = crisp(x(t));
    if (onHour ? mark.getHours() % hourStride === 0 : halfHours) {
      ctx.strokeStyle = onHour ? "rgba(255,255,255,0.10)" : "rgba(255,255,255,0.05)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px, trackTop);
      ctx.lineTo(px, trackBottom + (onHour ? 5 : 2));
      ctx.stroke();
      // 两端已经写了起止钟点，靠得太近的整点就不写了，免得两个数字叠在一起。
      if (onHour && px > 52 && px < cssW - 52) {
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
    blocks.push({
      start,
      end: Math.min(entry.ended_at, end),
      kind: entry.accepted ? "counted" : "dropped",
      name: nameOfCategory(entry.category),
      seconds: entry.seconds,
    });
  }
  if (day.timer) {
    const t = day.timer;
    const start = t.started_at > 0 ? t.started_at : now - t.elapsed_seconds;
    blocks.push({
      start,
      end: now,
      kind: pausedNow ? "counted" : "live",
      name: nameOfCategory(t.category),
      seconds: t.elapsed_seconds,
    });
  }
  if (restingNow && lastPause) {
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
    const fill =
      b.kind === "live" ? SIGNAL
      : b.kind === "rest" ? "rgba(176,136,67,0.30)"
      : b.kind === "dropped" ? "rgba(218,213,202,0.16)"
      : "rgba(218,213,202,0.72)";
    ctx.fillStyle = fill;
    ctx.beginPath();
    ctx.roundRect(x1, barTop, w, barH, 3);
    ctx.fill();
    if (b.kind === "dropped") {
      ctx.strokeStyle = "rgba(218,213,202,0.35)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.roundRect(crisp(x1), crisp(barTop), Math.round(w) - 1, Math.round(barH) - 1, 3);
      ctx.stroke();
    }
    // 块窄到写不下就不写，绝不画半个字。
    const label = `${b.name} ${meterText(b.seconds)}`;
    ctx.font = `500 12px -apple-system, "PingFang SC", sans-serif`;
    if (ctx.measureText(label).width + 14 <= w) {
      ctx.fillStyle = b.kind === "live" ? "#fff" : b.kind === "counted" ? BED : INK_HIGH;
      ctx.textAlign = "left";
      ctx.fillText(label, x1 + 7, barTop + barH / 2);
    } else if (ctx.measureText(b.name).width + 14 <= w) {
      ctx.fillStyle = b.kind === "live" ? "#fff" : b.kind === "counted" ? BED : INK_HIGH;
      ctx.textAlign = "left";
      ctx.fillText(b.name, x1 + 7, barTop + barH / 2);
    }
  }

  // 5. 此刻：一根竖线加一个点，落在轨的右端。
  if (endedAt === null) {
    const px = crisp(x(now));
    ctx.strokeStyle = pausedNow || restingNow ? CAUTION : SIGNAL;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(px, trackTop - 4);
    ctx.lineTo(px, trackBottom + 4);
    ctx.stroke();
    ctx.fillStyle = pausedNow || restingNow ? CAUTION : SIGNAL;
    ctx.beginPath();
    ctx.arc(px, trackTop - 4, 2.5, 0, Math.PI * 2);
    ctx.fill();
  }

  // 6. 两端的钟点，写在轨下面，省得靠中间的刻度倒推一天从几点开始。
  ctx.font = `500 11px ui-monospace, "SF Mono", Menlo, monospace`;
  ctx.fillStyle = INK_FAINT;
  ctx.textAlign = "left";
  const from = new Date(day.started_at * 1000);
  ctx.fillText(`${pad2(from.getHours())}:${pad2(from.getMinutes())}`, 0, trackBottom + 13);
  ctx.textAlign = "right";
  const to = new Date(end * 1000);
  ctx.fillText(`${pad2(to.getHours())}:${pad2(to.getMinutes())}`, cssW, trackBottom + 13);
  void STROKE;
}
