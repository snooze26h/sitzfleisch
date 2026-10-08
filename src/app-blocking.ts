// 手机应用屏蔽在界面这边的小工具。规则以 core 的校验为准，这里只做即时判断和默认值。

import type { AppBlocking, Preferences } from "./types";

export const MAX_BLOCKED_APPS = 200;
/** 关掉屏蔽时要手动输入的那句话。 */
export const DISABLE_PHRASE = "关闭屏蔽";
const PACKAGE_NAME = /^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$/;

export function validPackageName(name: unknown): name is string {
  return typeof name === "string" && name.length <= 255 && PACKAGE_NAME.test(name);
}

/** 外壳在用户从没开过时不发这个字段，按「关着、名单为空」处理。 */
export function appBlocking(p: Preferences): AppBlocking {
  return p.app_blocking ?? { enabled: false, apps: [] };
}

/** 手动确认时比较输入：统一全半角、去掉看不见的格式字符、合并空白，英文不分大小写。 */
export function sameText(typed: string, expected: string): boolean {
  const plain = (text: string) => text.normalize("NFKC").replace(/\p{Cf}/gu, "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
  return plain(typed) === plain(expected);
}
