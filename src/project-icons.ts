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

// 同一套笔法：1.6 描边、形体浅填充，每枚带一粒小方块。方块平时跟着描边色走；
// 只有正在烧的那个项目（界面给它设了 --glyph-accent），这一粒才变成朱红——朱红只标此刻。
const ember = 'fill="var(--glyph-accent, currentColor)" stroke="none"';
const body = 'fill="currentColor" opacity=".16" stroke="none"';
const shapes = new Map<string, string>([
  ["flask-conical", `<path d="M6.3 15h11.4l2 3.6a1.6 1.6 0 0 1-1.4 2.4H5.7a1.6 1.6 0 0 1-1.4-2.4z" ${body}/><path d="M7.5 3h9M9 3v6.5L4.3 18.6A1.6 1.6 0 0 0 5.7 21h12.6a1.6 1.6 0 0 0 1.4-2.4L15 9.5V3M6.3 15h11.4"/><path d="M10.4 16.9h3.2v3.2h-3.2z" ${ember}/>`],
  ["book-open", `<path d="M12 7C9 5 6 4.5 2.5 5v14c3.5-.5 6.5 0 9.5 2z" ${body}/><path d="M12 7C9 5 6 4.5 2.5 5v14c3.5-.5 6.5 0 9.5 2 3-2 6-2.5 9.5-2V5c-3.5-.5-6.5 0-9.5 2zM12 7v14M5.5 9.5l3.5 1M5.5 13.5l3.5 1"/><path d="M15.6 5.4h3.2v4.8h-3.2z" ${ember}/>`],
  ["languages", `<path d="M8 3h13v13H8z" ${body}/><path d="M8 8V3h13v13h-5M3 8h13v13H3zM6.4 18.5l3.1-7 3.1 7M7.6 16h3.8"/><path d="M15.8 5.2h3v3h-3z" ${ember}/>`],
  ["newspaper", `<path d="M3 4h18v16H3z" ${body}/><path d="M3 4h18v16H3zM13.5 8.5H18M13.5 12H18M6 15.5h12"/><path d="M6 7.5h4.5V12H6z" ${ember}/>`],
  ["message-square", `<path d="M3.5 4h17v12.5H11L6 20.5v-4H3.5z" ${body}/><path d="M3.5 4h17v12.5H11L6 20.5v-4H3.5zM7.5 8.5h9M7.5 12H12"/><path d="M14 10.5h3v3h-3z" ${ember}/>`],
  ["binary", `<path d="M3 4h18v16H3z" ${body}/><path d="M3 4h18v16H3zM6.5 9.5l3 2.5-3 2.5"/><path d="M12 10.2h3.2v4.8H12z" ${ember}/>`],
  ["dumbbell", `<path d="M4 7h4v10H4zM16 7h4v10h-4z" ${body}/><path d="M4 7h4v10H4zM16 7h4v10h-4zM8 12h8M2 9.5v5M22 9.5v5"/><path d="M10.4 10.4h3.2v3.2h-3.2z" ${ember}/>`],
  ["brain", `<path d="M12 3a6.5 6.5 0 0 0-4 11.6V17h8v-2.4A6.5 6.5 0 0 0 12 3z" ${body}/><path d="M12 3a6.5 6.5 0 0 0-4 11.6V17h8v-2.4A6.5 6.5 0 0 0 12 3zM9.5 20.5h5"/><path d="M10.4 8.2h3.2v3.2h-3.2z" ${ember}/>`],
]);

export function projectGlyph(name: string): string | undefined {
  // 旧版还可能保存 code；两个编程标识共用形体，但不迁移已保存的值。
  return shapes.get(name === "code" ? "binary" : name);
}
