import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { blockedPageContext, blockedPageUrl, matchingRuleKind, matchesUrl, normalizeUrl, originalBlockedUrl, validateRules } from "./rules.js";

const recommend = "https://www.douyin.com/?recommend=1";
const favorite = "https://www.douyin.com/user/self?from_tab_name=main&showSubTab=video&showTab=favorite_collection";
const envelope = (urls) => ({ protocol: 1, active: true, urls, revision: "0123456789abcdef" });

test("整站规则覆盖直播首页、房间与参数，保留同级站点和精确网址边界", () => {
  const rules = validateRules({ ...envelope([recommend]), protocol: 2, hosts: ["live.bilibili.com"] });
  for (const url of ["https://live.bilibili.com/", "https://live.bilibili.com/123456?from=search#player",
    "http://www.live.bilibili.com:8080/room/123", "https://LIVE.BILIBILI.COM./123",
    `https://live.bilibili.com/?q=${"a".repeat(5000)}`]) {
    assert.equal(matchingRuleKind(rules, url), "host", url);
  }
  for (const url of ["https://www.bilibili.com/", "https://space.bilibili.com/1", "https://otherlive.bilibili.com/",
    "https://live.bilibili.com.evil.example/", "https://room.live.bilibili.com/", "https://example.com/?host=live.bilibili.com"]) {
    assert.equal(matchesUrl(rules, url), false, url);
  }
  assert.equal(matchingRuleKind(rules, recommend), "url");
  assert.equal(matchesUrl(rules, favorite), false);
  assert.equal(matchesUrl({ ...rules, active: false }, "https://live.bilibili.com/123"), false);
});

test("新协议严检整站列表和合计条数，旧协议仍可读取", () => {
  const valid = { ...envelope([]), protocol: 2, hosts: ["live.bilibili.com"] };
  for (const host of [null, 1, "", "localhost", "127.0.0.1", "0x7f000001", "0x7f.1", "foo.123", "www.example.com",
    "https://example.com", "example.com/path", "example.com:443", "*.example.com", "EXAMPLE.COM", "example.com.",
    "bad..example", "-bad.example", "example.com\n127.0.0.1 other.example", "中文.example", `${"a".repeat(64)}.com`]) {
    assert.throws(() => validateRules({ ...valid, hosts: [host] }), undefined, String(host));
  }
  for (const patch of [{ hosts: null }, { hosts: Array(65).fill("example.com") },
    { urls: Array(64).fill(recommend) }, { protocol: 3 }, { extra: true }]) {
    assert.throws(() => validateRules({ ...valid, ...patch }));
  }
  assert.equal(validateRules(envelope([recommend])).protocol, 1);
  assert.equal(validateRules(valid).hosts[0], "live.bilibili.com");
});

test("推荐入口和 B 站首页与收藏、视频逐字区分", () => {
  const rules = validateRules(envelope([recommend, "https://www.bilibili.com/"]));
  assert.equal(matchesUrl(rules, recommend), true);
  assert.equal(matchesUrl(rules, favorite), false);
  assert.equal(matchesUrl(rules, "https://www.bilibili.com/video/BV1234567890/"), false);
  assert.equal(matchesUrl(rules, "https://www.douyin.com/"), false);
  assert.equal(matchesUrl(rules, "https://www.douyin.com/?recommend=1&extra=1"), false);
  assert.equal(matchesUrl(rules, "https://douyin.com/?recommend=1"), false);
});

test("只采用 URL 标准规范化，保留参数顺序、大小写、编码、片段和空分隔符", () => {
  assert.equal(normalizeUrl("HTTPS://EXAMPLE.COM:443"), "https://example.com/");
  const target = "https://example.com/Path?a=A&b=2#Hash";
  const rules = validateRules(envelope([target]));
  assert.equal(matchesUrl(rules, target), true);
  for (const changed of [
    target.replace("/Path", "/path"), target.replace("a=A", "a=a"),
    target.replace("a=A&b=2", "b=2&a=A"), target.replace("#Hash", "#hash"),
    target.replace("#Hash", ""), target.replace("https:", "http:"),
    target.replace("/Path?", "/Path/?"),
  ]) assert.equal(matchesUrl(rules, changed), false, changed);
  for (const url of ["https://example.com/?", "https://example.com/#", "https://example.com/?q=%2f"]) {
    assert.equal(normalizeUrl(url), url);
    assert.equal(matchesUrl(validateRules(envelope([url])), "https://example.com/"), false);
  }
});

test("验证 URL 输入边界与凭据，坏 URL 不误匹配", () => {
  for (const value of [null, 1, {}, "", "example.com", "file:///tmp/a", "javascript:alert(1)",
    "https://user:secret@example.com/", "https://example.com/\n", " https://example.com/",
    "https://example.com/?a=one two", "https://example.com/" + "x".repeat(4096), "https://example.com/" + "中".repeat(1400),
  ]) assert.throws(() => normalizeUrl(value), undefined, String(value));
  assert.equal(matchesUrl(envelope([recommend]), null), false);
  assert.equal(matchesUrl({ ...envelope([recommend]), active: false }, recommend), false);
});

test("整个响应严格验证，不能部分接受损坏规则", () => {
  for (const patch of [{ protocol: 2 }, { active: "true" }, { urls: null }, { urls: Array(65).fill(recommend) },
    { urls: [recommend, "file:///tmp/a"] }, { revision: "ABCDEF0123456789" }, { revision: "a" }, { extra: true },
  ]) assert.throws(() => validateRules({ ...envelope([recommend]), ...patch }));
  // 主程序允许重复规则；重复输入不能使整个快照失效。
  assert.equal(validateRules(envelope([recommend, recommend])).urls.length, 2);
});

test("阻止页 URL 解析只接受本扩展页面及安全原网址", () => {
  const page = "chrome-extension://test/blocked.html";
  assert.equal(originalBlockedUrl(`${page}#url=${encodeURIComponent(recommend)}`, page), recommend);
  for (const value of [`${page}?a=1#url=${encodeURIComponent(recommend)}`, `${page}#url=javascript:alert(1)`,
    `${page}#url=${encodeURIComponent(recommend)}&url=${encodeURIComponent(favorite)}`,
    `https://example.com/blocked.html#url=${encodeURIComponent(recommend)}`,
  ]) assert.equal(originalBlockedUrl(value, page), null);
});

test("扩展只声明必要权限，不注入脚本或开放扩展页面", async () => {
  const manifest = JSON.parse(await readFile(new URL("./manifest.json", import.meta.url), "utf8"));
  assert.deepEqual(manifest.permissions.sort(), ["alarms", "storage", "tabs", "webNavigation"]);
  assert.deepEqual(manifest.host_permissions, ["http://127.0.0.1/*"]);
  assert.equal(manifest.manifest_version, 3);
  assert.equal(manifest.background.type, "module");
  assert.equal(manifest.content_scripts, undefined);
  assert.equal(manifest.web_accessible_resources, undefined);
});

test("阻止页返回目标仅允许一个有限长的 HTTP(S) 网址，保留参数和片段", () => {
  const page = "chrome-extension://test/blocked.html";
  const target = `${favorite}#saved`;
  const url = blockedPageUrl(page, recommend, target);
  assert.deepEqual(blockedPageContext(url, page), { original: recommend, returnUrl: target });
  assert.equal(originalBlockedUrl(url, page), recommend);
  assert.deepEqual(blockedPageContext(blockedPageUrl(page, recommend), page), { original: recommend, returnUrl: null });
  for (const suffix of [
    "return=javascript:alert(1)", "return=file:///tmp/private", "return=https://user:secret@example.com/",
    `return=${encodeURIComponent(target)}&return=${encodeURIComponent(target)}`,
    "return=https://example.com/" + "a".repeat(4096), "unexpected=value",
  ]) assert.equal(blockedPageContext(`${page}#url=${encodeURIComponent(recommend)}&${suffix}`, page), null);
  assert.equal(blockedPageContext(`${page}#${"x".repeat(30000)}`, page), null);
});
