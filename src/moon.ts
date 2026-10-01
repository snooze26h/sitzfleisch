// 今天这一轮月。全天目标是一轮满月：已计入的时间把它一点点照亮——蛾眉月、上弦、凸月，
// 学满了就是满月，也就是「圆满」。没照亮的那一面只剩一点地照，整轮月的轮廓就是今天的目标。
// 暂停时月亮停在原处，不再长。月亮上不加任何记号：在不在计时由计时区和时间轴说，月亮只管亮了几成。
//
// 月面不是裁一张平光的满月，而是按进度现算光照：太阳从右边照过来，越靠近明暗界线光越斜、越暗，
// 界线附近的环形山自己显出起伏。资料来自 NASA 的月球影像（design/yue/render_moon.py 处理成 moon-maps.webp：
// R 反照率，G/B 起伏造成的法线偏移），光照参数与那边共用 moon-shading.json。
// 页面里只放一块 <canvas data-moon="亮了几成">；每次重绘后 paintMoons 按它在屏幕上实际占的设备像素定画布大小，
// 一像素对一像素地上色（两块屏幕像素密度不同也跟得上），只有亮度或尺寸变了才重画。

import type { Day } from "./types";
import { quotaSeconds } from "./state";
import shading from "./assets/yue/moon-shading.json";

const mapsUrl = new URL("./assets/yue/moon-maps.webp", import.meta.url).href;

/**
 * 朝今天目标走了多少秒：每个项目最多算到它自己的配额——一个项目超额，补不了另一个项目的缺口。
 * 项目已计入的秒数取台账与分类合计里较多的那个（旧存档可能只有分类合计）。
 * live 是这一格正在走的秒数，算进它所属的项目。
 */
function creditedSeconds(d: Day, live: { category: string; seconds: number } | null = null): number {
  return d.categories.reduce((sum, c) => {
    const quota = c.quota_minutes * 60;
    if (quota <= 0) return sum;
    const ledger = d.ledger.reduce((s, e) => s + (e.category === c.id && e.accepted && e.seconds > 0 ? e.seconds : 0), 0);
    const done = Math.max(ledger, c.accepted_seconds) + (live?.category === c.id ? live.seconds : 0);
    return sum + Math.min(done, quota);
  }, 0);
}

/**
 * 这一天圆满了没有：有目标，且每个有配额的项目都满了——和「下一格」建议走完的判据是同一件事。
 * 满月、历史墙的「圆满」计数共用这一条。
 */
export function dayFull(d: Day): boolean {
  const quota = quotaSeconds(d);
  return quota > 0 && creditedSeconds(d) >= quota;
}

// ---------- 页面上的一轮月 ----------

type MoonSize = "hero" | "art" | "mini" | "tile" | "row" | "key";

/**
 * 画布的像素宽高不写在这里：由 paintMoons 按显示尺寸 × 设备像素比现定，morphdom 也不去动它（见 main.ts）。
 * earth 是地照的倍数，只有新月用得上（见 newMoon）。
 */
function moonCanvas(lit: number, size: MoonSize, cls = "", earth = 1): string {
  return `<canvas class="moon moon-${size}${cls ? ` ${cls}` : ""}" data-moon="${Math.max(0, Math.min(1, lit)).toFixed(4)}"${earth !== 1 ? ` data-earth="${earth}"` : ""} aria-hidden="true"></canvas>`;
}

/**
 * 月历上那些小月亮的地照倍数。真实的地照太淡，48px 一格时暗面几乎看不见；调亮几倍后整轮轮廓都在，
 * 亮了几成一眼比得出来，没有记录的那天（新月）就只剩这层灰影，和学了一点点的日子也分得开。
 */
const CALENDAR_EARTH = 3;

/**
 * 今天这一轮月。这一格在走时，它走过的时间也算进亮的那一瓣；按停时停在原处。
 * hero 是运行台右上的主图形，mini 是侧栏的缩影，tile 是历史墙上「今天」那一格。
 */
export function dayMoon(d: Day, variant: "hero" | "mini" | "tile" = "hero"): string {
  const t = d.timer;
  const full = !t && dayFull(d);
  const lit = full ? 1 : Math.min(1, creditedSeconds(d, t ? { category: t.category, seconds: t.elapsed_seconds } : null) / Math.max(1, quotaSeconds(d)));
  return moonCanvas(lit, variant, full ? "full" : "", variant === "tile" ? CALENDAR_EARTH : 1);
}

/** 归档的一天：那天的月亮亮到了几成。row 是归档行开头那枚，tile 是历史墙上的一格。 */
export function archiveMoon(d: Day, variant: "row" | "tile"): string {
  const full = dayFull(d);
  return moonCanvas(full ? 1 : creditedSeconds(d) / Math.max(1, quotaSeconds(d)), variant, full ? "full" : "", CALENDAR_EARTH);
}

/** 图例里的一枚。 */
export function keyMoon(lit: number): string {
  return moonCanvas(lit, "key", lit >= 1 ? "full" : "", CALENDAR_EARTH);
}

/**
 * 最早一条记录以来、没有记录的那一天：一轮没亮的新月，只剩地照映出的一层灰影。
 */
export function newMoon(variant: "tile" | "key"): string {
  return moonCanvas(0, variant, "new", CALENDAR_EARTH);
}

/** 开始页与空历史上的一弯新月：还没开始，只亮着一线。 */
export function crescentArt(cls: string): string {
  return `<figure class="moon-art ${cls}" aria-hidden="true">${moonCanvas(0.14, "art")}</figure>`;
}

// ---------- 算光 ----------

interface Level {
  size: number;
  A: Float32Array;
  dx: Float32Array;
  dy: Float32Array;
  cover: Float32Array;
  nx: Float32Array;
  ny: Float32Array;
  nz: Float32Array;
}

let levels: Level[] | null = null;
let loading = false;

/** 解析式的球面法线与月缘覆盖率（抗锯齿一像素）；压成 8 位的话，界线附近会起台阶。 */
function sphere(size: number, into: Pick<Level, "nx" | "ny" | "nz" | "cover">): void {
  const r = (shading.radius * size) / shading.size;
  const c = size / 2;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = y * size + x;
      const px = (x + 0.5 - c) / r;
      const py = -(y + 0.5 - c) / r;
      const rr = px * px + py * py;
      into.nx[i] = px;
      into.ny[i] = py;
      into.nz[i] = Math.sqrt(Math.max(0, 1 - rr));
      into.cover[i] = Math.max(0, Math.min(1, 0.5 - (Math.sqrt(rr) - 1) * r));
    }
  }
}

function blank(size: number): Level {
  const n = size * size;
  return { size, A: new Float32Array(n), dx: new Float32Array(n), dy: new Float32Array(n), cover: new Float32Array(n), nx: new Float32Array(n), ny: new Float32Array(n), nz: new Float32Array(n) };
}

/** 往下缩一级：2×2 取平均（反照率与起伏按覆盖率加权），球面法线重算。 */
function halve(src: Level): Level {
  const size = src.size / 2;
  const dst = blank(size);
  sphere(size, dst);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let a = 0, dx = 0, dy = 0, w = 0;
      for (let k = 0; k < 4; k++) {
        const j = (2 * y + (k >> 1)) * src.size + 2 * x + (k & 1);
        const c = src.cover[j];
        a += src.A[j] * c;
        dx += src.dx[j] * c;
        dy += src.dy[j] * c;
        w += c;
      }
      const i = y * size + x;
      if (w > 0) {
        dst.A[i] = a / w;
        dst.dx[i] = dx / w;
        dst.dy[i] = dy / w;
      }
    }
  }
  return dst;
}

function buildLevels(img: HTMLImageElement): Level[] {
  const size = shading.size;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.drawImage(img, 0, 0);
  const data = ctx.getImageData(0, 0, size, size).data;
  const top = blank(size);
  sphere(size, top);
  const k = shading.relief_range / 127;
  for (let i = 0; i < size * size; i++) {
    top.A[i] = data[i * 4] / 255;
    top.dx[i] = (data[i * 4 + 1] - 128) * k;
    top.dy[i] = (data[i * 4 + 2] - 128) * k;
  }
  const out = [top];
  while (out[out.length - 1].size > 64) out.push(halve(out[out.length - 1]));
  return out;
}

/** 先把资料图读进来；读好之前页面上的月亮是空的，读好后立刻补画一次。 */
export function loadMoon(ready: () => void): void {
  if (levels || loading) return;
  loading = true;
  const img = new Image();
  img.decoding = "async";
  img.onload = () => {
    levels = buildLevels(img);
    ready();
  };
  img.onerror = () => {
    loading = false;
  };
  img.src = mapsUrl;
}

/**
 * 与 design/yue/render_moon.py 的 shade 同一套：月面光度按 Lommel–Seeliger 与 Lambert 混合，
 * 所以满月平而亮、界线处自然暗下去；暗面只留一点地照。输出是「骨白色 + 不透明度」，叠在什么底上都对。
 */
function shade(level: Level, lit: number, earthBoost = 1): ImageData {
  const { size, A, dx, dy, cover, nx, ny, nz } = level;
  const out = new ImageData(size, size);
  const px = out.data;
  const g = Math.acos(2 * lit - 1);
  const lx = Math.sin(g);
  const lz = Math.cos(g);
  const w = shading.lommel;
  const floor = shading.albedo_floor;
  const [cr, cg, cb] = shading.light;
  for (let i = 0; i < size * size; i++) {
    const cov = cover[i];
    const j = i * 4;
    if (cov <= 0) continue;
    const x = nx[i] + dx[i];
    const y = ny[i] + dy[i];
    const z = Math.sqrt(Math.max(1e-6, 1 - x * x - y * y));
    const mu0 = x * lx + z * lz;
    const alb = floor + (1 - floor) * A[i];
    let val = 0;
    if (mu0 > 0) {
      const ph = (1 - w) * mu0 + (w * 2 * mu0) / (mu0 + Math.max(z, 1e-4));
      val = Math.pow(Math.min(1, ph * alb * shading.exposure), shading.gamma);
    }
    const earth = shading.earthshine * earthBoost * alb * (0.55 + 0.45 * nz[i]);
    if (earth > val) val = earth;
    px[j] = cr;
    px[j + 1] = cg;
    px[j + 2] = cb;
    px[j + 3] = Math.round(val * cov * 255);
  }
  return out;
}

const shaded = new Map<string, ImageData>();
const painted = new WeakMap<HTMLCanvasElement, string>();
let scratch: HTMLCanvasElement | null = null;

/**
 * 给页面里的月亮上色：画布按显示尺寸 × 设备像素比定大小，亮度或尺寸变了才重画。
 * 亮度按千分之一取整：同样亮的几轮只算一次，主图形约半分钟才重算一回。
 * 取不小于画布的最小一级资料来算，再一次性高质量缩到画布大小，不经过 CSS 二次缩放。
 */
export function paintMoons(root: ParentNode): void {
  if (!levels) return;
  const dpr = window.devicePixelRatio || 1;
  for (const canvas of root.querySelectorAll<HTMLCanvasElement>("canvas[data-moon]")) {
    const css = canvas.getBoundingClientRect().width;
    if (css <= 0) continue;
    const px = Math.max(16, Math.round(css * dpr));
    if (canvas.width !== px || canvas.height !== px) {
      canvas.width = px;
      canvas.height = px;
      painted.delete(canvas);
    }
    const lit = Math.round(Number(canvas.dataset.moon) * 1000) / 1000;
    const earth = Number(canvas.dataset.earth) || 1;
    const stamp = `${px}:${lit}:${earth}`;
    if (painted.get(canvas) === stamp) continue;
    const level = [...levels].reverse().find((l) => l.size >= px) ?? levels[0];
    const key = `${level.size}:${lit}:${earth}`;
    let img = shaded.get(key);
    if (!img) {
      img = shade(level, lit, earth);
      shaded.set(key, img);
      if (shaded.size > 96) shaded.delete(shaded.keys().next().value!);
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) continue;
    ctx.clearRect(0, 0, px, px);
    if (level.size === px) {
      ctx.putImageData(img, 0, 0);
    } else {
      scratch ??= document.createElement("canvas");
      scratch.width = level.size;
      scratch.height = level.size;
      scratch.getContext("2d")!.putImageData(img, 0, 0);
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(scratch, 0, 0, px, px);
    }
    painted.set(canvas, stamp);
  }
}
