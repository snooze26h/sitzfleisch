// 核心导航延续应用图标的折页形体；项目图标由 project-icons.ts 扩展同一套语言。
type BrandGlyph = "today" | "history" | "settings";

const glyphs: Record<BrandGlyph, string> = {
  today: '<path d="M5 3h10l5 5v13H5z" fill="currentColor" opacity=".12"/><path d="M5 3h10l5 5v13H5zM15 3v5h5"/><path d="M9 12h7M9 16h4"/><path d="M15 3v5h5z" fill="var(--brand-accent, #cd6145)" stroke="none"/>',
  history: '<path d="M4 8v13h13M7 5v13h13" opacity=".55"/><path d="M10 2h6l5 5v8H10z" fill="currentColor" opacity=".12"/><path d="M10 2h6l5 5v8H10zM16 2v5h5"/><path d="M16 2v5h5z" fill="var(--brand-accent, #cd6145)" stroke="none"/>',
  settings: '<path d="M5 4h14v16H5z" fill="currentColor" opacity=".12"/><path d="M5 4h14v16H5zM9 7v10M15 7v10"/><path d="M7 10h4M13 14h4" stroke-width="3"/><path d="M16 4h3v3z" fill="var(--brand-accent, #cd6145)" stroke="none"/>',
};

const wordmarkImage = new URL("./assets/brand/sitzfleisch-mark.webp", import.meta.url).href;

/** 使用原应用图标的透明形体，让侧栏与应用图标共享相同材质和细节。 */
export function brandMark(): string {
  return `<img class="brand-mark" src="${wordmarkImage}" alt="" aria-hidden="true" width="38" height="48" decoding="async" draggable="false" />`;
}

const artwork = {
  archive: new URL("./assets/time-fold/fold-archive.webp", import.meta.url).href,
};

const ribbons = {
  ready: new URL("./assets/time-fold/ribbon-ready.webp", import.meta.url).href,
  flow: new URL("./assets/time-fold/ribbon-flow.webp", import.meta.url).href,
  paused: new URL("./assets/time-fold/fold-pause.webp", import.meta.url).href,
  rest: new URL("./assets/time-fold/ribbon-rest.webp", import.meta.url).href,
};

export type RibbonPhase = "ready" | "flow" | "paused" | "rest" | "done";

/** 状态图保持在同一层里：切换可以交叠过渡，心跳重绘不会重播动画。 */
export function ribbonScene(phase: RibbonPhase, cls = ""): string {
  return `<figure class="ribbon-scene${cls ? ` ${cls}` : ""}" data-phase="${phase}" aria-hidden="true">${Object.entries(ribbons).map(([name, src]) => `<img class="ribbon-frame ribbon-${name}" src="${src}" alt="" width="960" height="640" decoding="async" draggable="false" />`).join("")}</figure>`;
}

/** 状态仍由文字和控件完整表达；插画不进入读屏顺序，也不参与每秒动画。 */
export function foldArt(state: keyof typeof artwork, cls = ""): string {
  return `<figure class="fold-art fold-${state}${cls ? ` ${cls}` : ""}" aria-hidden="true"><img src="${artwork[state]}" alt="" width="768" height="768" decoding="async" draggable="false" /></figure>`;
}

export function brandIcon(name: BrandGlyph, size = 20): string {
  return `<svg class="ic brand-glyph" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${glyphs[name]}</svg>`;
}
