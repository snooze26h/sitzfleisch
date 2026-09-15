import { icons } from "lucide";
import { projectGlyph } from "./project-icons";

// 项目使用专属折页图形，通用操作与旧图标保留 Lucide；导航图形位于 brand.ts。

type IconNode = [tag: string, attrs: Record<string, string | number>][];

function pascal(name: string): string {
  return name
    .split("-")
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1) : ""))
    .join("");
}

const table = icons as unknown as Record<string, IconNode>;

export function icon(name: string, size = 15, cls = ""): string {
  const custom = projectGlyph(name);
  const candidate = table[pascal(name)];
  const node = Array.isArray(candidate) ? candidate : table.BookOpen;
  const inner = custom ?? node
    .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(" ")}></${tag}>`)
    .join("");
  return `<svg class="ic${custom ? " project-glyph" : ""}${cls ? ` ${cls}` : ""}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${custom ? 1.6 : 2}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
}
