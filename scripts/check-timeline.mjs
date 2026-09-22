import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-timeline-"));
try {
  execFileSync(process.execPath, [resolve("node_modules/typescript/bin/tsc"), "src/timeline.ts", "--strict", "--target", "ES2020", "--module", "commonjs", "--outDir", output], { stdio: "inherit" });
  const { activeSpans } = createRequire(import.meta.url)(join(output, "timeline.js"));
  const pause = (start, end) => ({ started_at: start, ended_at: end, auto: false });
  assert.deepEqual(activeSpans(100, 44000, [pause(940, null)]), [{ start: 100, end: 940 }], "工作 14 分钟后暂停十二小时，不能把空档画成工作");
  const split = activeSpans(100, 3700, [pause(1000, 1600)]);
  assert.deepEqual(split, [{ start: 100, end: 1000 }, { start: 1600, end: 3700 }]);
  assert.equal(split.reduce((sum, s) => sum + s.end - s.start, 0), 3000, "图形总长度与扣除十分钟暂停后的专注时长一致");
  assert.deepEqual(activeSpans(0, 100, [pause(40, 70), pause(20, 50), pause(-10, 5), pause(110, 120)]), [{ start: 5, end: 20 }, { start: 70, end: 100 }], "重叠、乱序和区间外的暂停不会重复扣除");
  assert.deepEqual(activeSpans(0, 100, [pause(0, null)]), []);
  assert.deepEqual(activeSpans(100, 100, []), []);
  console.log("时间轴检查通过：长暂停、格内暂停、区间裁剪与重叠合并。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
