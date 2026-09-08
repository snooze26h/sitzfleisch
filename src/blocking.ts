// 输入时的预览与 mock 共用；正式保存仍交给 core 做权威校验。

export const MAX_BLOCK_RULES = 64;
const MAX_URL_BYTES = 4096;

function domainError(host: string): string | null {
  if (host === "localhost" || host.includes(":") || host.startsWith("[") || host.split(".").every((part) => /^\d+$/.test(part))) {
    return "不能添加 localhost 或 IP 地址。";
  }
  if (/[^\x00-\x7f]/.test(host)) return "只接受英文域名；中文域名请先转换成 punycode。";
  if (host.length > 253 || !host.includes(".") || host.split(".").some((part) => !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(part))) {
    return "请输入完整域名，例如 douyin.com；每段不能以连字符开头或结尾。";
  }
  try {
    // 浏览器也把十六进制、缩写 IPv4 当作 IP，不能只检查用户输入的字符。
    const parsedHost = new URL(`http://${host}`).hostname;
    if (parsedHost.split(".").every((part) => /^\d+$/.test(part))) return "不能添加 localhost 或 IP 地址。";
  } catch {
    return "没有识别出安全、完整的域名。";
  }
  return null;
}

export function normalizeHost(raw: string): { host: string } | { error: string } {
  const host = raw.trim().toLowerCase().replace(/^www\./, "");
  if (!host) return { error: "域名不能为空。" };
  if (/[\s\x00-\x1f\x7f-\x9f\\/:?#@]/.test(host)) return { error: "整个网站模式只接受域名，例如 douyin.com；要保留其他页面，请改用精确网址。" };
  const error = domainError(host);
  return error ? { error } : { host };
}

export function normalizeUrl(raw: string): { url: string } | { error: string } {
  const value = raw.trim();
  if (!value) return { error: "网址不能为空。" };
  if (new TextEncoder().encode(value).length > MAX_URL_BYTES) return { error: "完整网址不能超过 4096 字节，请缩短后重试。" };
  if (/[\s\x00-\x1f\x7f-\x9f\\]/.test(value)) return { error: "网址不能包含空白、控制字符或反斜杠。" };
  const authority = value.match(/^https?:\/\/([^/?#]+)/i)?.[1];
  if (!authority) return { error: "请粘贴以 https:// 或 http:// 开头的完整网址。" };
  if (authority.includes("@")) return { error: "网址不能包含用户名或密码。" };
  if (/[^\x00-\x7f]/.test(authority)) return { error: "只接受英文域名；中文域名请先转换成 punycode。" };
  try {
    const parsed = new URL(value);
    const error = domainError(parsed.hostname);
    if (error) return { error };
    if (new TextEncoder().encode(parsed.href).length > MAX_URL_BYTES) return { error: "规范化后的网址超过 4096 字节，请缩短后重试。" };
    return { url: parsed.href };
  } catch {
    return { error: "没有识别出完整网址，请检查域名和端口。" };
  }
}

/** 系统 hosts 只写裸域与 www 两项；不能把其他子域也误报成冲突。 */
export function conflictingHost(url: string, hosts: string[]): string | undefined {
  try {
    const hostname = new URL(url).hostname;
    return hosts.find((host) => hostname === host || hostname === `www.${host}`);
  } catch {
    return undefined;
  }
}
