export const MAX_RULES = 64;
export const MAX_URL_BYTES = 4096;
const encoder = new TextEncoder();

export function normalizeUrl(value) {
  if (typeof value !== "string" || !value || encoder.encode(value).length > MAX_URL_BYTES) {
    throw new Error("invalid-url");
  }
  // 控制字符会被 URL 解析器悄悄剥离，不能把不同输入误当成同一条规则。
  if (value !== value.trim() || /[\u0000-\u0020\u007f]/u.test(value) || !/^https?:\/\//i.test(value)) {
    throw new Error("invalid-url");
  }
  const parsed = new URL(value);
  if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password || !parsed.hostname) {
    throw new Error("invalid-url");
  }
  if (encoder.encode(parsed.href).length > MAX_URL_BYTES) throw new Error("invalid-url");
  return parsed.href;
}

export function validateRules(value) {
  const version = value?.protocol;
  const expectedKeys = version === 2 ? "active,hosts,protocol,revision,urls" : "active,protocol,revision,urls";
  if (!value || typeof value !== "object" || Array.isArray(value)
    || Object.keys(value).sort().join(",") !== expectedKeys
    || ![1, 2].includes(version) || typeof value.active !== "boolean"
    || typeof value.revision !== "string" || !/^[0-9a-f]{16}$/.test(value.revision)
    || !Array.isArray(value.urls) || value.urls.length > MAX_RULES) {
    throw new Error("invalid-rules");
  }
  const urls = value.urls.map(normalizeUrl);
  if (version === 1) return { protocol: 1, active: value.active, urls, revision: value.revision };
  if (!Array.isArray(value.hosts) || value.hosts.length + urls.length > MAX_RULES) throw new Error("invalid-rules");
  const hosts = value.hosts.map((host) => {
    if (typeof host !== "string" || host.length > 253 || !host.includes(".") || host.startsWith("www.")
      || host.split(".").some((part) => !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(part))) throw new Error("invalid-host");
    const parsed = new URL(`http://${host}`).hostname;
    if (parsed !== host || parsed.split(".").every((part) => /^\d+$/.test(part))) throw new Error("invalid-host");
    return host;
  });
  return { protocol: 2, active: value.active, urls, hosts, revision: value.revision };
}

export function matchingRuleKind(rules, value) {
  if (!rules?.active) return null;
  try {
    // 整站匹配只看域名，与代理解析、旧连接、页面路径和参数无关。
    const parsed = new URL(value);
    if (!["http:", "https:"].includes(parsed.protocol)) return null;
    const host = parsed.hostname.replace(/\.$/, "");
    if (rules.hosts?.some((rule) => host === rule || host === `www.${rule}`)) return "host";
    return rules.urls.includes(normalizeUrl(value)) ? "url" : null;
  } catch {
    return null;
  }
}

export function matchesUrl(rules, value) {
  return matchingRuleKind(rules, value) !== null;
}

export function blockedPageContext(value, blockedPage) {
  try {
    // 两个网址经过片段转义后最多各膨胀三倍；先限长，再交给 URL 解析器。
    if (typeof value !== "string" || value.length > blockedPage.length + MAX_URL_BYTES * 6 + 32) return null;
    const parsed = new URL(value);
    if (`${parsed.protocol}//${parsed.host}${parsed.pathname}` !== blockedPage || parsed.search) return null;
    const params = new URLSearchParams(parsed.hash.slice(1));
    const keys = [...params.keys()].sort().join(",");
    if (!["url", "return,url"].includes(keys)) return null;
    return {
      original: normalizeUrl(params.get("url")),
      returnUrl: params.has("return") ? normalizeUrl(params.get("return")) : null,
    };
  } catch {
    return null;
  }
}

export function originalBlockedUrl(value, blockedPage) {
  return blockedPageContext(value, blockedPage)?.original ?? null;
}

export function blockedPageUrl(blockedPage, original, returnUrl) {
  let normalized;
  try { normalized = normalizeUrl(original); } catch {
    // 整站规则也要拦超长或带凭据的导航；不把敏感/超限网址写进阻止页片段。
    return blockedPage;
  }
  const params = new URLSearchParams({ url: normalized });
  if (returnUrl) params.set("return", normalizeUrl(returnUrl));
  return `${blockedPage}#${params}`;
}
