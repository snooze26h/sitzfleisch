import { icons } from "lucide";

// Lucide 是全 app 唯一的图标系统：24×24 画布、2 宽描边、圆头圆角，按尺寸等比缩线宽。

type IconNode = [tag: string, attrs: Record<string, string | number>][];

function pascal(name: string): string {
  return name
    .split("-")
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1) : ""))
    .join("");
}

const table = icons as unknown as Record<string, IconNode>;

export function icon(name: string, size = 15, cls = ""): string {
  const node = table[pascal(name)] ?? table.BookOpen;
  const inner = node
    .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(" ")}></${tag}>`)
    .join("");
  return `<svg class="ic${cls ? ` ${cls}` : ""}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
}
