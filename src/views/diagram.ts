// 运行图（Bildfahrplan）：横轴是时间，每个项目一条轨，最下面一条是暂停。
// 一整天是一根**只往前走**的线：在哪条轨上就说明那段时间在做什么，换事情时折返。
//
// 关键规则：暂停会打断它所在的那一格。一格被暂停切成几截，就画成几条，
// 中间露出暂停轨——否则线会为了去画暂停而倒退回过去的时间，那是假的。

import type { Day } from "../types";

const INK_FAINT = "#89847b";
const STROKE = "#dad5ca";
const SIGNAL = "#c7452f";
const CAUTION = "#b08843";
const BED = "#0e0e10";
const SPINE = "rgba(255,255,255,0.22)";

/** 轨高与顶部留白：界面里的行标签直接用这两个值，免得两边各写一套又对不上。 */
export const LANE_H = 28;
export const DIAGRAM_TOP = 22;
const HEADROOM = 28;
const BAR_H = 10;

type Kind = "counted" | "dropped" | "live" | "pause" | "rest";

interface Span {
  start: number;
  end: number;
  lane: number;
  kind: Kind;
  /** 这一截所属的那一格的总时长。 */
  totalSeconds?: number;
  /** 一格被暂停切成几截时，只有最长的那截写字，免得同一个数字出现两遍。 */
  labelled?: boolean;
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

/** 把 [start,end) 里落在任何一段暂停中的部分挖掉，返回剩下的几截。 */
function subtractPauses(start: number, end: number, pauses: { start: number; end: number }[]): [number, number][] {
  let pieces: [number, number][] = [[start, end]];
  for (const p of pauses) {
    const next: [number, number][] = [];
    for (const [a, b] of pieces) {
      if (p.end <= a || p.start >= b) {
        next.push([a, b]);
        continue;
      }
      if (p.start > a) next.push([a, p.start]);
      if (p.end < b) next.push([p.end, b]);
    }
    pieces = next;
  }
  return pieces.filter(([a, b]) => b > a);
}

export function drawDiagram(canvas: HTMLCanvasElement, day: Day, now: number, endedAt: number | null): void {
  const laneIds = day.categories.map((c) => c.id);
  const pauseLane = laneIds.length;
  const laneCount = laneIds.length + 1;
  const cssW = Math.max(160, Math.floor(canvas.getBoundingClientRect().width || canvas.parentElement?.clientWidth || 600));
  const cssH = laneCount * LANE_H + DIAGRAM_TOP;
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

  const usable = Math.max(60, cssW - HEADROOM);
  const end = endedAt ?? now;
  const span = Math.max(1800, end - day.started_at);
  const x = (t: number) => Math.max(0, Math.min(usable, (usable * (t - day.started_at)) / span));
  const laneTop = (i: number) => DIAGRAM_TOP + LANE_H * i;
  const railY = (i: number) => laneTop(i) + LANE_H / 2;
  const crisp = (v: number) => Math.round(v) + 0.5;

  const line = (x1: number, y1: number, x2: number, y2: number, color: string, width = 1) => {
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
  };

  // 1. 轨道底纹：隔行淡一层，行与行才分得开
  for (let i = 0; i < laneCount; i++) {
    if (i % 2 === 0) continue;
    ctx.fillStyle = "rgba(255,255,255,0.018)";
    ctx.fillRect(0, laneTop(i), cssW, LANE_H);
  }
  ctx.fillStyle = "rgba(255,255,255,0.012)";
  ctx.fillRect(0, laneTop(pauseLane), cssW, LANE_H);

  // 2. 钟点竖线与刻度
  const mark = new Date(day.started_at * 1000);
  mark.setMinutes(0, 0, 0);
  if (mark.getTime() / 1000 < day.started_at) mark.setHours(mark.getHours() + 1);
  const hours = Math.ceil(span / 3600);
  const stride = hours > 14 ? 3 : hours > 8 ? 2 : 1;
  let count = 0;
  ctx.font = `500 10px ui-monospace, "SF Mono", Menlo, monospace`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "center";
  while (mark.getTime() / 1000 <= end) {
    if (count % stride === 0) {
      const px = crisp(x(mark.getTime() / 1000));
      line(px, DIAGRAM_TOP, px, cssH, "rgba(255,255,255,0.045)");
      // 贴着左右边缘的刻度只画线不写字，写了也是被裁掉的半个数。
      if (px > 16 && px < cssW - 16) {
        ctx.fillStyle = INK_FAINT;
        ctx.fillText(`${pad2(mark.getHours())}时`, px, 9);
      }
    }
    mark.setHours(mark.getHours() + 1);
    count += 1;
  }

  // 3. 收集这一天的所有时段。暂停优先：它会把所在的格切开。
  // break_until 只描述「现在这一段」，历史上的休息没有留痕——所以只有正在进行的
  // 那一段暂停才画成休息，往回不臆造。
  const restingNow = day.break_until !== null && now < day.break_until;
  // 格被按停时，它已经不在跑了——朱红只留给**真的在走**的那一格。
  const lastPause = day.pauses[day.pauses.length - 1];
  const pausedNow = !!lastPause && lastPause.ended_at === null;
  const pauses = day.pauses
    .map((p, i) => ({
      start: p.started_at,
      end: Math.min(p.ended_at ?? end, end),
      rest: restingNow && p.ended_at === null && i === day.pauses.length - 1,
    }))
    .filter((p) => p.end > p.start)
    .sort((a, b) => a.start - b.start);

  const spans: Span[] = [];
  const laneOf = (id: string) => laneIds.indexOf(id);
  const addBlock = (start: number, finish: number, lane: number, kind: Kind, seconds: number) => {
    const pieces = subtractPauses(start, Math.min(finish, end), pauses);
    if (!pieces.length) return;
    let longest = 0;
    pieces.forEach(([a, b], i) => {
      if (b - a > pieces[longest][1] - pieces[longest][0]) longest = i;
    });
    pieces.forEach(([a, b], i) => {
      spans.push({ start: a, end: b, lane, kind, totalSeconds: seconds, labelled: i === longest });
    });
  };
  for (const entry of day.ledger) {
    const lane = laneOf(entry.category);
    if (lane < 0) continue;
    const start = entry.started_at > 0 ? entry.started_at : entry.ended_at - entry.seconds;
    addBlock(start, entry.ended_at, lane, entry.accepted ? "counted" : "dropped", entry.seconds);
  }
  if (day.timer) {
    const t = day.timer;
    const lane = laneOf(t.category);
    if (lane >= 0) {
      const start = t.started_at > 0 ? t.started_at : now - t.elapsed_seconds;
      addBlock(start, now, lane, pausedNow ? "counted" : "live", t.elapsed_seconds);
    }
  }
  for (const p of pauses) spans.push({ start: p.start, end: p.end, lane: pauseLane, kind: p.rest ? "rest" : "pause" });
  // 排完序就一定不重叠，线只会往前走。
  spans.sort((a, b) => a.start - b.start || a.end - b.end);

  // 4. 主脉：把这些时段串成一根单调往前的线
  if (spans.length) {
    ctx.strokeStyle = SPINE;
    ctx.lineWidth = 1;
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(x(spans[0].start), railY(spans[0].lane));
    for (const s of spans) {
      ctx.lineTo(x(s.start), railY(s.lane));
      ctx.lineTo(x(s.end), railY(s.lane));
    }
    ctx.stroke();
  }

  // 5. 每一段的实体
  for (const s of spans) {
    const left = x(s.start);
    const width = Math.max(2, x(s.end) - left);
    if (s.kind === "pause" || s.kind === "rest") {
      const y = railY(s.lane) - 3;
      ctx.save();
      ctx.beginPath();
      ctx.rect(left, y, width, 6);
      ctx.clip();
      ctx.strokeStyle = "rgba(137,132,123,0.55)";
      ctx.lineWidth = 1;
      ctx.beginPath();
      if (s.kind === "rest") {
        // 休息用横向点线，和发呆的斜纹一眼分得开，仍然是同一支灰。
        ctx.setLineDash([2, 3]);
        ctx.moveTo(left, y + 1.5);
        ctx.lineTo(left + width, y + 1.5);
        ctx.moveTo(left, y + 4.5);
        ctx.lineTo(left + width, y + 4.5);
      } else {
        for (let offset = left - 6; offset < left + width; offset += 5) {
          ctx.moveTo(offset, y + 6);
          ctx.lineTo(offset + 6, y);
        }
      }
      ctx.stroke();
      ctx.restore();
      continue;
    }
    const y = railY(s.lane) - BAR_H / 2;
    ctx.beginPath();
    ctx.roundRect(left, y, width, BAR_H, 2);
    if (s.kind === "counted" || s.kind === "live") {
      ctx.fillStyle = s.kind === "live" ? SIGNAL : STROKE;
      ctx.fill();
      // 够宽就把这一格有多长写在里面，省得去数格子。
      if (s.labelled && width >= 40 && s.totalSeconds) {
        ctx.font = `600 9px ui-monospace, "SF Mono", Menlo, monospace`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillStyle = s.kind === "live" ? "rgba(255,255,255,0.92)" : BED;
        ctx.fillText(meterText(s.totalSeconds), left + width / 2, railY(s.lane) + 0.5);
      }
    } else {
      ctx.strokeStyle = INK_FAINT;
      ctx.lineWidth = 1;
      ctx.stroke();
      line(left + 2, railY(s.lane), left + width - 2, railY(s.lane), CAUTION, 1.5);
    }
  }

  // 6. 此刻：轴上一道短刻；正在做事就在那条轨上点一个点
  if (endedAt === null) {
    // 此刻这一道：在走是朱红，按停了转赭石——和侧栏那只暂停秒表一个颜色。
    const nowTint = pausedNow ? CAUTION : SIGNAL;
    const position = crisp(x(now));
    line(position, DIAGRAM_TOP - 6, position, DIAGRAM_TOP + 4, nowTint, 1.5);
    const active = spans.find((s) => s.start <= now && s.end >= now - 2);
    if (active) {
      ctx.fillStyle = nowTint;
      ctx.beginPath();
      ctx.arc(x(now), railY(active.lane), 2.5, 0, Math.PI * 2);
      ctx.fill();
    }
  }
}
