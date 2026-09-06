// 用真实快照合并函数回归乱序推送，历史与实时状态各自保留最新版本。
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-snapshot-check-"));
try {
  execFileSync(process.execPath, [
    resolve("node_modules/typescript/bin/tsc"), "src/snapshots.ts", "--strict",
    "--target", "ES2020", "--module", "commonjs", "--outDir", output,
  ], { stdio: "inherit" });
  const { mergeSnapshot } = createRequire(import.meta.url)(join(output, "snapshots.js"));
  const snapshot = (revision, preferences, history = null, saveError = null) => ({
    revision, state: { last_tick: 1000, preferences }, history, save_error: saveError,
  });
  let current = { snapshot: null, history: [], historyRevision: -1 };
  const receive = (incoming) => {
    current = mergeSnapshot(current.snapshot, current.history, current.historyRevision, incoming);
  };
  const before = snapshot(1, { break_minutes: 10, hydration_goal_cups: 8 }, []);
  const saved = snapshot(2, { break_minutes: 15, hydration_goal_cups: 8 }, []);
  receive(saved);
  receive(before);
  assert.equal(current.snapshot, saved, "迟到的心跳不能覆盖已保存的偏好");
  const nextEdit = { ...current.snapshot.state.preferences, hydration_goal_cups: 9 };
  assert.deepEqual(nextEdit, { break_minutes: 15, hydration_goal_cups: 9 });

  const archived = [{ day: { started_at: 100 }, ended_at: 200 }];
  const archiveReply = snapshot(3, nextEdit, archived);
  const heartbeat = snapshot(4, nextEdit);
  receive(heartbeat);
  receive(archiveReply);
  assert.equal(current.snapshot, heartbeat, "迟到的历史不能倒退实时状态");
  assert.equal(current.history, archived, "更新的无历史心跳不能吞掉归档返回值");
  assert.equal(current.historyRevision, 3);

  const afterDeletion = snapshot(5, nextEdit, []);
  const laterHeartbeat = snapshot(6, nextEdit);
  receive(laterHeartbeat);
  receive(afterDeletion);
  receive(archiveReply);
  assert.equal(current.snapshot, laterHeartbeat);
  assert.deepEqual(current.history, [], "旧历史不能恢复刚删除的归档");
  assert.equal(current.historyRevision, 5);
  receive(afterDeletion);
  assert.equal(current.historyRevision, 5, "重复返回不改变历史版本");

  // 存盘失败的横条跟着实时状态走同一条序号：迟到的推送不能把它贴回来，也不能提前抹掉。
  receive(snapshot(8, nextEdit, null, null));
  receive(snapshot(7, nextEdit, null, "写不进去"));
  assert.equal(current.snapshot.save_error, null, "迟到的失败快照不能把已经清掉的横条贴回来");
  receive(snapshot(9, nextEdit, null, "写不进去"));
  assert.equal(current.snapshot.save_error, "写不进去", "新的失败要立刻看得见");
  receive(snapshot(10, nextEdit, null, null));
  assert.equal(current.snapshot.save_error, null, "再存成功一次，横条就该消失");
  console.log("快照乱序检查通过：偏好保留、迟到归档、删除保留、重复返回、保存失败与恢复。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
