import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-timeline-"));
try {
  execFileSync(process.execPath, [resolve("node_modules/typescript/bin/tsc"), "src/timeline.ts", "src/markdown.ts", "--strict", "--target", "ES2020", "--module", "commonjs", "--outDir", output], { stdio: "inherit" });
  const load = createRequire(import.meta.url);
  const { activeSpans } = load(join(output, "timeline.js"));
  const { markdownForDay } = load(join(output, "markdown.js"));
  const { restSeconds } = load(join(output, "state.js"));
  const pause = (start, end) => ({ started_at: start, ended_at: end, auto: false });
  assert.deepEqual(activeSpans(100, 44000, [pause(940, null)]), [{ start: 100, end: 940 }], "工作 14 分钟后暂停十二小时，不能把空档画成工作");
  const split = activeSpans(100, 3700, [pause(1000, 1600)]);
  assert.deepEqual(split, [{ start: 100, end: 1000 }, { start: 1600, end: 3700 }]);
  assert.equal(split.reduce((sum, s) => sum + s.end - s.start, 0), 3000, "图形总长度与扣除十分钟暂停后的专注时长一致");
  assert.deepEqual(activeSpans(0, 100, [pause(40, 70), pause(20, 50), pause(-10, 5), pause(110, 120)]), [{ start: 5, end: 20 }, { start: 70, end: 100 }], "重叠、乱序和区间外的暂停不会重复扣除");
  assert.deepEqual(activeSpans(0, 100, [pause(0, null)]), []);
  assert.deepEqual(activeSpans(100, 100, []), []);

  // 休息也落在暂停里：结束一格后休息 10 分钟、又停了 5 分钟。读数和 Markdown 都要把两者分开，同一段时间不记两遍。
  const t0 = 1_700_000_000;
  const blockEnd = t0 + 1800;
  const now = blockEnd + 900;
  const day = {
    profile_name: "标准", profile_id: "standard", started_at: t0, timer: null, cups: 0,
    categories: [{ id: "deep", name: "深度工作", quota_minutes: 60, accepted_seconds: 1500 }],
    ledger: [{ category: "deep", seconds: 1500, accepted: true, tasks: [], started_at: t0 + 300, ended_at: blockEnd, completion_note: "" }],
    seated_seconds: 1500, paused_seconds: 300 + 900, suspend_seconds: 0, seated_since_water: 0, seated_since_relief: 0, paused_without_block: 900,
    break_until: null,
    pauses: [pause(t0, t0 + 300), pause(blockEnd, null)],
    rests: [{ started_at: blockEnd, ended_at: blockEnd + 600 }],
  };
  assert.equal(restSeconds(day, now), 600);
  assert.equal(restSeconds(day, blockEnd + 120), 120, "还在休息的那段只算到此刻");
  const md = markdownForDay(day, null, now);
  assert.match(md, /· 暂停 10m · 休息 10m/, "暂停扣掉休息后是 5 + 5 分钟");
  assert.match(md, /## 休息\n- \S+–\S+ 10 分钟/);
  const pauseSection = md.slice(md.indexOf("## 暂停"));
  assert.equal((pauseSection.match(/^- /gm) ?? []).length, 2, "暂停只剩开格前 5 分钟和休息后的 5 分钟");
  assert.match(pauseSection, /–… 5 分钟/, "还在停着的那段写到「…」");
  console.log("时间轴检查通过：长暂停、格内暂停、区间裁剪与重叠合并，休息与暂停分开记。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
