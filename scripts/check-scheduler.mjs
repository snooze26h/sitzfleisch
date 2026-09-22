// 调度的边界回归。与 core 中的同名场景一起守住界面和菜单栏的一致性。
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-scheduler-"));
try {
  execFileSync(process.execPath, [resolve("node_modules/typescript/bin/tsc"), "src/scheduler.ts", "--strict", "--target", "ES2020", "--module", "commonjs", "--outDir", output], { stdio: "inherit" });
  const { suggest, blockMinutes } = createRequire(import.meta.url)(join(output, "scheduler.js"));
  const categories = ["main", "reading"].map((id) => ({ id, quota_minutes: 60, accepted_seconds: 0 }));
  const prefs = { uniform_block_minutes: 0, categories: categories.map((c) => ({ id: c.id, role: "dailyFloor", default_block_minutes: 50 })) };
  const day = { categories, ledger: [], seated_seconds: 0 };
  assert.equal(suggest(day, prefs, 1000).category, "main", "两个最低保障项目并列时，界面和 Rust 均保留计划中的第一个");
  assert.equal(suggest({ ...day, categories: [...categories].reverse() }, prefs, 1000).category, "reading");
  assert.equal(blockMinutes({ ...categories[0], quota_minutes: 1, accepted_seconds: 59 }, prefs), 1, "只剩一秒也按一分钟建议，不恢复旧版五分钟下限");
  assert.equal(blockMinutes(categories[0], { ...prefs, uniform_block_minutes: 37 }), 37, "自定义默认值保持精确分钟");
  assert.equal(suggest({ ...day, categories: categories.map((c) => ({ ...c, accepted_seconds: 3600 })) }, prefs, 1000), null);
  console.log("调度检查通过：并列规则、分钟边界、自定义块长、目标完成。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
