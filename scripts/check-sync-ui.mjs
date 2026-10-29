// 用实际状态校验与渲染函数验证账号界面；不发邮件、不调用原生凭据库。
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const output = mkdtempSync(join(tmpdir(), "sitzfleisch-sync-ui-"));
try {
  execFileSync(process.execPath, [
    resolve("node_modules/typescript/bin/tsc"), "src/views/sync.ts", "src/record-identity.ts", "--strict", "--skipLibCheck",
    "--target", "ES2020", "--module", "commonjs", "--moduleResolution", "node", "--outDir", output,
  ], { stdio: "inherit" });
  symlinkSync(resolve("node_modules"), join(output, "node_modules"), "dir");
  const require = createRequire(import.meta.url);
  const { captureActionTarget, normalizedSyncStatus, observedCommandArgs, syncLabel, validSyncCode, validSyncEmail } = require(join(output, "sync.js"));
  const { accountPanel, syncIndicator } = require(join(output, "views/sync.js"));
  const { ui } = require(join(output, "state.js"));
  const { studyDayKey, completionEntryIndex, sameBlockTimer } = require(join(output, "record-identity.js"));
  const { sessionRow } = require(join(output, "components.js"));
  const base = normalizedSyncStatus(undefined);
  assert.equal(base.phase, "unconfigured");
  assert.equal(normalizedSyncStatus(null).phase, "unconfigured");
  const { account_id: ignoredAccountId, ...legacySingle } = base;
  assert.equal(normalizedSyncStatus(legacySingle).account_id, null, "旧单机状态缺账号 ID 可兼容");
  assert.equal(normalizedSyncStatus({ ...legacySingle, email: "reader@example.com", phase: "synced" }).phase, "error", "共享状态不能缺账号身份");
  for (const bad of [true, [], "synced", {}, { ...base, phase: "synced", revision: -1 }, { ...base, last_synced_at: Infinity }, { ...base, error: "x".repeat(2001) }]) {
    assert.equal(normalizedSyncStatus(bad).phase, "error", "坏状态不能伪装成已同步");
  }
  assert.equal(normalizedSyncStatus({ ...base, phase: "conflict" }).phase, "error");
  assert.equal(normalizedSyncStatus({ ...base, phase: "awaiting_import", migration: { local_days: "3" } }).phase, "error");
  assert.equal(normalizedSyncStatus({ ...base, conflict: { fields: [42], local_days: 0, remote_days: 0, local_active: false, remote_active: false } }).phase, "error");

  for (const email of ["reader@example.com", "reader+mac@example.co.uk"]) assert.ok(validSyncEmail(email));
  for (const email of ["", "reader", "reader@example", "a\nb@example.com", "<img>@example.com", `${"a".repeat(65)}@example.com`]) assert.equal(validSyncEmail(email), false);
  for (const code of ["123456", "0123456789"]) assert.ok(validSyncCode(code));
  for (const code of ["", "12345", "12345678901", "123 456", "12e345", "１２３４５６"]) assert.equal(validSyncCode(code), false);

  const put = (s) => { ui.snap = { sync: s }; };
  put(base);
  assert.match(accountPanel(), /此版本尚未配置云同步/);
  assert.doesNotMatch(accountPanel(), /data-action="sync-send-code"/);
  assert.equal(syncIndicator(), "");

  const signedOut = { ...base, configured: true, phase: "signed_out" };
  put(signedOut);
  ui.syncEmail = "reader@example.com";
  assert.match(accountPanel(), /for="sync-email"/);
  assert.match(accountPanel(), /type="email"/);
  assert.match(accountPanel(), /data-action="sync-send-code"/);
  ui.syncSentTo = "reader@example.com";
  ui.syncCode = "123456";
  ui.syncBusy = true;
  ui.syncResendAt = ui.now + 60;
  const busy = accountPanel();
  assert.match(busy, /正在验证/);
  assert.match(busy, /data-action="sync-verify" disabled/);
  assert.match(busy, /data-action="sync-send-code" disabled/);
  assert.match(busy, /autocomplete="one-time-code"/);
  ui.syncBusy = false;
  ui.syncSentTo = "";
  ui.syncEmail = "new-reader@example.com";
  ui.syncResendAt = Math.floor(Date.now() / 1000) + 60;
  const changedEmailWaiting = accountPanel();
  assert.match(changedEmailWaiting, /data-action="sync-send-code" disabled/, "更换邮箱不会绕过重发节流");
  assert.match(changedEmailWaiting, /\d+ 秒后可发送/, "更换邮箱后，禁用的发送按钮仍须说明剩余等待");
  ui.syncResendAt = Math.floor(Date.now() / 1000) - 1;
  assert.doesNotMatch(accountPanel(), /data-action="sync-send-code" disabled/, "等待结束后有效的新邮箱可再次发送");

  put({ ...signedOut, phase: "awaiting_import", email: "reader@example.com", migration: { local_days: 8, cloud_days: 12, local_active: true, cloud_active: true, cloud_empty: false } });
  const migration = accountPanel();
  assert.match(migration, /8 个历史学习日/);
  assert.match(migration, /12 个历史学习日/);
  assert.match(migration, /两边都有未结束/);
  for (const strategy of ["merge_remote", "merge_local", "cloud_only"]) assert.match(migration, new RegExp(`data-strategy="${strategy}"`));
  assert.match(migration, /单机存档会保留/);

  const hostile = '<img src=x onerror="alert(1)">';
  const conflict = { ...signedOut, account_id: "00000000-0000-4000-8000-000000000001", phase: "conflict", email: hostile, pending: true, conflict: { fields: ["day", "preferences", hostile], local_days: 8, remote_days: 9, local_active: true, remote_active: false }, error: hostile, recovery_path: hostile };
  put(conflict);
  const html = accountPanel();
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /&lt;img/);
  assert.match(html, /两份恢复副本/);
  assert.match(html, /data-choice="remote"/);
  assert.match(html, /data-choice="local"/);
  assert.match(syncIndicator(), /打开账号与同步/);
  assert.equal(syncLabel({ ...conflict, phase: "offline" }), "离线 · 待同步");
  assert.equal(syncLabel({ ...conflict, phase: "offline", pending: false }), "离线");

  const firstRecord = { sync_id: "a-entry", category: "reading", seconds: 60, accepted: true, tasks: [], started_at: 100, ended_at: 160, completion_note: "first" };
  const secondRecord = { ...firstRecord, sync_id: "b-entry", completion_note: "second" };
  const firstDay = { sync_id: "a-day", started_at: 100, categories: [{ id: "reading", name: "阅读" }], ledger: [firstRecord, secondRecord] };
  const secondDay = { ...firstDay, sync_id: "b-day" };
  assert.notEqual(studyDayKey(firstDay), studyDayKey(secondDay), "同秒开始的两台设备不能共用历史 DOM 键");
  assert.equal(completionEntryIndex(firstDay, "b-entry", 0, 160), 1, "稳定 ID 优先于已变化的数组下标");
  assert.equal(completionEntryIndex(firstDay, "missing", 0, 160), -1, "找不到 ID 时不能回退修改同秒的其他记录");
  assert.equal(completionEntryIndex(firstDay, undefined, 0, 160), -1, "旧弹窗不能误改新加载的共享记录");
  assert.equal(completionEntryIndex({ ...firstDay, ledger: [{ ...firstRecord, sync_id: undefined }] }, undefined, 0, 160), 0, "无 ID 的旧单机记录仍可编辑");
  assert.equal(sameBlockTimer({ sync_id: "block-a", started_at: 100, category: "reading" }, { sync_id: "block-b", started_at: 100, category: "reading" }), false);
  assert.equal(sameBlockTimer({ started_at: 100, category: "reading" }, { started_at: 100, category: "reading" }), true);
  ui.snap = { sync: signedOut, state: { preferences: { categories: firstDay.categories } } };
  const firstRow = sessionRow(firstRecord, firstDay);
  const secondRow = sessionRow(firstRecord, secondDay);
  assert.notEqual(firstRow.match(/id="([^"]+)"/)[1], secondRow.match(/id="([^"]+)"/)[1]);
  assert.match(firstRow, /data-day-id="a-day"/);
  assert.match(firstRow, /data-entry-id="a-entry"/);
  const sharedSnapshot = { revision: 9000, sync: { ...base, configured: true, account_id: "00000000-0000-4000-8000-000000000001", phase: "synced", email: "reader@example.com", revision: 12 }, state: { day: { ...firstDay, timer: { sync_id: "block-a", elapsed_seconds: 10 }, pauses: [], break_until: null } } };
  const frozen = observedCommandArgs("end_day", {}, sharedSnapshot);
  assert.equal(frozen.expected.revision, 12, "操作版本来自云端版本，不能用每秒增长的本机快照序号");
  const advanced = structuredClone(sharedSnapshot);
  advanced.revision++;
  advanced.state.day.timer.elapsed_seconds += 1;
  assert.deepEqual(captureActionTarget(advanced), frozen.expected, "自然走时不使操作目标失效");
  advanced.sync.revision++;
  advanced.state.day.timer.sync_id = "block-b";
  assert.notDeepEqual(captureActionTarget(advanced), frozen.expected);
  assert.deepEqual(observedCommandArgs("end_day", frozen, advanced).expected, frozen.expected, "旧确认框必须保留打开时目标，不能悄悄改为新一格");
  assert.equal(observedCommandArgs("end_day", { expected: null }, advanced).expected, null, "单机期间打开的确认框不能升级为账号里的新操作");
  assert.equal("expected" in observedCommandArgs("sync_retry", {}, advanced), false, "账号操作不混入计时守卫");
  const otherAccount = structuredClone(sharedSnapshot);
  otherAccount.sync.account_id = "00000000-0000-4000-8000-000000000002";
  assert.notEqual(captureActionTarget(otherAccount).account_id, frozen.expected.account_id, "云端版本相同也不能跨账号复用旧确认");
  assert.equal(observedCommandArgs("end_day", frozen, otherAccount).expected.account_id, sharedSnapshot.sync.account_id, "旧确认必须把原账号交给后端拒绝，不能改绑新账号");
  console.log("同步界面检查通过：坏状态拒绝、输入边界、禁用与重发、迁移选择、冲突恢复、HTML 转义、同秒记录定位与陈旧操作目标保护。");
} finally {
  rmSync(output, { recursive: true, force: true });
}
