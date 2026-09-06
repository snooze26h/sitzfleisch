// 短名的自动回退规则：TS 与 Rust 必须给出同样的答案。
// Rust 那边是 src-tauri 的 short_name_fallback_prefers_the_whole_name_then_the_first_word，
// 两处的用例逐条对齐；改了一边没改另一边，托盘和界面就会喊出不同的名字。
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-short-name-check-"));
try {
  execFileSync(process.execPath, [
    resolve("node_modules/typescript/bin/tsc"), "src/format.ts", "--strict",
    "--target", "ES2020", "--module", "commonjs", "--outDir", output,
  ], { stdio: "inherit" });
  const { shortNameFrom } = createRequire(import.meta.url)(join(output, "format.js"));

  assert.equal(shortNameFrom("学js"), "学js", "宽度 4，放得下");
  assert.equal(shortNameFrom("深度工作"), "深度工作", "4 个汉字正好是上限");
  assert.equal(shortNameFrom("English Reading"), "English", "太长就取空格前的首个词");
  assert.equal(shortNameFrom("深度工作计划"), "深度", "没空格又放不下，取前两个字");
  assert.equal(shortNameFrom("Extraordinarily Long"), "Ex", "首个词也放不下，还是前两个字");
  assert.equal(shortNameFrom("  写作  "), "写作", "首尾空白不算数");

  console.log("短名规则检查通过：全名、首个词、前两字三条回退与 Rust 一致。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
