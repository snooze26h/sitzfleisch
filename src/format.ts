// 时间与数字的写法：度量用仪表记法（3h55），句子用中文（3 小时 55 分）。

export function clock(seconds: number): string {
  const safe = Math.max(0, Math.floor(seconds));
  const h = Math.floor(safe / 3600);
  const m = Math.floor((safe % 3600) / 60);
  const s = safe % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** 句子里的时长：45 秒 / 25 分钟 / 2 小时 / 5 小时 35 分 */
export function duration(seconds: number): string {
  const safe = Math.max(0, seconds);
  if (safe > 0 && safe < 60) return `${Math.floor(safe)} 秒`;
  const minutes = Math.round(safe / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m} 分钟`;
  if (m === 0) return `${h} 小时`;
  return `${h} 小时 ${m} 分`;
}

/** 仪表记法：45m / 8h / 4h30。数字等宽对齐，读的是刻度不是句子。 */
export function meter(seconds: number): string {
  const minutes = Math.round(Math.max(0, seconds) / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h${String(m).padStart(2, "0")}`;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

export function wallClock(unix: number): string {
  const d = new Date(unix * 1000);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/** 8月28日 周五 */
export function dayLabel(unix: number): string {
  const d = new Date(unix * 1000);
  return `${d.getMonth() + 1}月${d.getDate()}日 ${WEEKDAYS[d.getDay()]}`;
}

export function dayOfMonth(unix: number): string {
  return `${new Date(unix * 1000).getDate()}日`;
}

/**
 * 一个字顶两个拉丁字符宽的那类字：汉字、假名、谚文、全角标点。
 * 短名的宽度按显示算，「≤ 4 个汉字」与「≤ 8 个拉丁字符」于是是同一条线。
 */
const WIDE = /[ᄀ-ᅟ⺀-〾ぁ-㏿㐀-䶿一-鿿ꀀ-꓏가-힣豈-﫿︰-﹯＀-｠￠-￦]|[\u{20000}-\u{2FA1F}]/u;

export function displayWidth(text: string): number {
  let width = 0;
  for (const ch of text) width += WIDE.test(ch) ? 2 : 1;
  return width;
}

/** 短名放得下多宽：4 个汉字，或 8 个拉丁字符。 */
export const SHORT_NAME_WIDTH = 8;

/**
 * `short_name` 为空时的自动回退：放得下就用全名，放不下先试空格前的首个词，
 * 再放不下就取前两个字（不是后两个——「深度工作」截成「工作」会丢掉是哪一种）。
 * **这条规则与 `src-tauri::short_name_from` 必须一模一样**，两边各有测试。
 */
export function shortNameFrom(name: string): string {
  const trimmed = name.trim();
  if (displayWidth(trimmed) <= SHORT_NAME_WIDTH) return trimmed;
  const first = trimmed.split(/\s+/)[0] ?? "";
  if (first && displayWidth(first) <= SHORT_NAME_WIDTH) return first;
  return [...trimmed].slice(0, 2).join("");
}

const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export function esc(text: string | number | null | undefined): string {
  return String(text ?? "").replace(/[&<>"']/g, (c) => ESCAPES[c] ?? c);
}

export function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}
