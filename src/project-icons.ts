// 项目图标沿用已有标识，升级外观无需改写用户配置或历史记录。
export const PROJECT_ICON_CHOICES: [string, string][] = [
  ["flask-conical", "科研"],
  ["book-open", "阅读"],
  ["languages", "语言"],
  ["newspaper", "浏览"],
  ["message-square", "交流"],
  ["binary", "编程"],
  ["dumbbell", "锻炼"],
  ["brain", "思考"],
];

const cut = 'fill="var(--brand-accent, #cd6145)" stroke="none"';
const shapes = new Map<string, string>([
  ["flask-conical", `<path d="M6 14h12l3 5H3z" fill="currentColor" opacity=".2" stroke="none"/><path d="M8 3h8M9 3v6l-5.7 9.5A1.7 1.7 0 0 0 4.8 21h14.4a1.7 1.7 0 0 0 1.5-2.5L15 9V3M7 14h10"/><path d="m15 14 4 5h-4z" ${cut}/>`],
  ["book-open", `<path d="M3 4c4-1 6 0 9 2v15c-3-2-5-3-9-2z" fill="currentColor" opacity=".18" stroke="none"/><path d="M3 4c4-1 6 0 9 2 3-2 5-3 9-2v15c-4-1-6 0-9 2-3-2-5-3-9-2zM12 6v15M6 8l3 1M6 12l3 1"/><path d="m17 4 4 1v5l-4-2z" ${cut}/>`],
  ["languages", `<path d="M8 3h11v15H8z" fill="currentColor" opacity=".16" stroke="none"/><path d="M8 3h11v15M4 6h10l3 3v12H4zM14 6v3h3M7 17l3-7 3 7M8 14h4"/><path d="M14 6h3v3z" ${cut}/>`],
  ["newspaper", `<path d="M3 6h18v14H3z" fill="currentColor" opacity=".16" stroke="none"/><path d="M3 6h18v14H3zM6 3h12M7 10h4v4H7zM14 10h3M14 14h3M7 17h10"/><path d="M17 6h4v4z" ${cut}/>`],
  ["message-square", `<path d="M3 4h18v13H10l-5 4v-4H3z" fill="currentColor" opacity=".18" stroke="none"/><path d="M3 4h18v13H10l-5 4v-4H3zM7 9h10M7 13h6"/><path d="M17 4h4v4z" ${cut}/>`],
  ["binary", `<path d="M3 4h14l4 4v13H3z" fill="currentColor" opacity=".13" stroke="none"/><path d="M3 4h14l4 4v13H3zM17 4v4h4M9 10l-3 3 3 3M15 10l3 3-3 3"/><path d="M17 4h4v4z" ${cut}/>`],
  ["dumbbell", `<path d="M3 7h5v10H3zM16 7h5v10h-5z" fill="currentColor" opacity=".2" stroke="none"/><path d="M3 7h5v10H3zM16 7h5v10h-5zM8 10h8M8 14h8M1 10v4M23 10v4"/><path d="M18 7h3v10h-3z" ${cut}/>`],
  ["brain", `<path d="M12 3a7 7 0 0 0-4 12.7V18h8v-2.3A7 7 0 0 0 12 3" fill="currentColor" opacity=".16" stroke="none"/><path d="M12 3a7 7 0 0 0-4 12.7V18h8v-2.3A7 7 0 0 0 12 3ZM9 21h6M9 8c0-2 5-2 5 .5 0 2-4 1.5-4 3.5 0 1.2 2 1.5 3 1"/><path d="M14 3a7 7 0 0 1 5 5h-4z" ${cut}/>`],
]);

export function projectGlyph(name: string): string | undefined {
  // 旧版还可能保存 code；两个编程标识共用形体，但不迁移已保存的值。
  return shapes.get(name === "code" ? "binary" : name);
}
