// 用实际前端预览回归「推荐页屏蔽、收藏与视频放行」及输入边界。
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-blocking-check-"));
try {
  execFileSync(process.execPath, [
    resolve("node_modules/typescript/bin/tsc"), "src/blocking.ts", "--strict",
    "--target", "ES2020", "--module", "commonjs", "--outDir", output,
  ], { stdio: "inherit" });
  const { normalizeHost, normalizeUrl, conflictingHost, MAX_BLOCK_RULES } = createRequire(import.meta.url)(join(output, "blocking.js"));
  const url = (raw) => {
    const result = normalizeUrl(raw);
    assert.ok("url" in result, `${raw} 应被接受：${result.error}`);
    return result.url;
  };
  const recommend = "https://www.douyin.com/?recommend=1";
  const favorites = "https://www.douyin.com/user/self?from_tab_name=main&showSubTab=video&showTab=favorite_collection";
  assert.equal(url(recommend), recommend);
  assert.equal(url(favorites), favorites, "收藏路径和大小写不能被截掉");
  assert.notEqual(url(recommend), url(favorites));
  assert.equal(url("HTTPS://WWW.BILIBILI.COM:443"), "https://www.bilibili.com/");
  assert.notEqual(url("https://www.bilibili.com/"), url("https://www.bilibili.com/video/BV1Learning/"));
  for (const [left, right] of [
    ["/Video", "/video"], ["/?recommend=1", "/?recommend=2"],
    ["/?a=1&b=2", "/?b=2&a=1"], ["/?x=ABC", "/?x=abc"],
    ["/#Home", "/#home"], ["/", "/?"], ["/", "/#"], ["/watch", "/watch/"],
  ]) assert.notEqual(url(`https://example.com${left}`), url(`https://example.com${right}`), "精确匹配不能忽略路径、参数、顺序或锚点");
  assert.equal(url("https://example.com/学习"), "https://example.com/%E5%AD%A6%E4%B9%A0");
  assert.equal(url("  https://example.com/  "), "https://example.com/", "输入框可去掉粘贴带入的首尾空白");
  for (const raw of [
    "douyin.com", "https:///example.com", "ftp://example.com/", "javascript:alert(1)",
    "https://user:password@example.com/", "https://@example.com/", "https://example.com/a b",
    "https://example.com/\nnext", "https://example.com/\u0080next", "https://example.com/\u009fnext", "https://example.com/\\next", "http://localhost/",
    "http://127.0.0.1/", "http://2130706433/", "http://0x7f000001/", "http://[::1]/",
    "https://internal/", "https://-bad.example/", "https://bad-.example/", "https://foo..example/",
    "https://example.com./", "https://中文.example/", "https://example.com:99999/",
    `https://${"a".repeat(64)}.example/`,
  ]) assert.ok("error" in normalizeUrl(raw), `${JSON.stringify(raw)} 应被拒绝`);
  const prefix = "https://example.com/";
  assert.equal(url(prefix + "a".repeat(4096 - prefix.length)).length, 4096);
  assert.ok("error" in normalizeUrl(prefix + "a".repeat(4097 - prefix.length)));
  assert.ok("error" in normalizeUrl(prefix + "学".repeat(500)), "Unicode 路径转义后的字节数也要受限");

  assert.deepEqual(normalizeHost(" WWW.Douyin.COM "), { host: "douyin.com" });
  for (const raw of ["https://douyin.com/", "douyin.com/video", "douyin.com?recommend=1", "douyin.com:443", "douyin.com#home", "localhost", "127.0.0.1", "0x7f.1", "0177.0.0.1", "example.123", "foo..example", "-bad.example", `${"a".repeat(64)}.example`]) {
    assert.ok("error" in normalizeHost(raw), `整站输入不能悄悄截短 ${raw}`);
  }
  assert.equal(conflictingHost(recommend, ["douyin.com"]), "douyin.com");
  assert.equal(conflictingHost(favorites, ["douyin.com"]), "douyin.com", "旧整站规则仍然会阻挡收藏，必须明确提示");
  assert.equal(conflictingHost("https://space.bilibili.com/1", ["bilibili.com"]), undefined, "hosts 不自动覆盖任意子域");
  assert.equal(conflictingHost("https://otherdouyin.com/", ["douyin.com"]), undefined);
  assert.equal(MAX_BLOCK_RULES, 64);
  console.log("屏蔽预览检查通过：完整网址、参数与锚点严格保留，收藏/视频区分，非法输入与长度限制，整站冲突提示。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
