// 品牌与导航图形。导航三枚是细线单色图形：今天是半轮月，历史是一页月历上的一轮月，设置是调节杆。
// 朱红只标此刻，所以导航里的实心部分跟着描边色走。字标是应用图标上那枚立体 S 的抠图，品牌的朱红留在它的端面上。
type BrandGlyph = "today" | "history" | "settings";

const mark = 'fill="currentColor" stroke="none"';

const glyphs: Record<BrandGlyph, string> = {
  // 今天这一轮月，亮了一半。
  today: `<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 0 1 0 17z" ${mark}/>`,
  // 一页月历，当中一轮月。
  history: `<path d="M4 5h16v15H4z"/><path d="M4 9.5h16M8.5 3v4M15.5 3v4"/><circle cx="12" cy="14.75" r="2.4" ${mark}/>`,
  // 两根调节杆，一实一空两个滑块。
  settings: `<path d="M3 7h4M11 7h10M3 17h10M17 17h4"/><path d="M7 5h4v4H7z" ${mark}/><path d="M13 15h4v4h-4z"/>`,
};

export function brandIcon(name: BrandGlyph, size = 20): string {
  return `<svg class="ic brand-glyph" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${glyphs[name]}</svg>`;
}

const wordmarkImage = new URL("./assets/brand/sitzfleisch-mark.webp", import.meta.url).href;

/** 侧栏字标沿用应用图标的立体 S：透明抠图，骨白材质和朱红端面与 Dock 里的图标是同一个。 */
export function brandMark(): string {
  return `<img class="brand-mark" src="${wordmarkImage}" alt="" aria-hidden="true" width="38" height="48" decoding="async" draggable="false" />`;
}
