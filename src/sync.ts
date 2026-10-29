import type { ActionTarget, Snapshot, SyncStatus } from "./types";

const PHASES: SyncStatus["phase"][] = ["unconfigured", "signed_out", "awaiting_import", "synced", "pending", "offline", "conflict", "reauth", "error"];

export const SYNC_MUTATION_COMMANDS = new Set([
  "start_day", "start_block", "switch_profile", "toggle_pause", "extend_block", "finish_block",
  "set_completion_note", "end_break", "abandon_block", "end_day", "abandon_day", "update_preferences", "delete_history_day",
]);

export function captureActionTarget(snapshot: Snapshot | null): ActionTarget | null {
  if (!snapshot) return null;
  const sync = normalizedSyncStatus(snapshot.sync);
  if (!sync.account_id || sync.phase === "awaiting_import") return null;
  const day = snapshot.state.day;
  return {
    account_id: sync.account_id,
    revision: sync.revision,
    day_id: day?.sync_id || null,
    timer_id: day?.timer?.sync_id || null,
    paused: !!day && day.pauses[day.pauses.length - 1]?.ended_at === null,
    break_until: day?.break_until ?? null,
  };
}

/** 已捕获的确认框目标（包括明确的 null）不能被最新快照替换。 */
export function observedCommandArgs(command: string, args: Record<string, unknown>, snapshot: Snapshot | null): Record<string, unknown> {
  if (!SYNC_MUTATION_COMMANDS.has(command) || Object.prototype.hasOwnProperty.call(args, "expected")) return args;
  const expected = captureActionTarget(snapshot);
  return expected ? { ...args, expected } : args;
}

function emptyStatus(): SyncStatus {
  return { configured: false, account_id: null, phase: "unconfigured", email: null, revision: 0, pending: false, last_synced_at: null, error: null, migration: null, conflict: null, recovery_path: null };
}

/** 不猜测半份状态；坏 IPC 数据给出可见错误，不能显示成“已同步”。 */
export function normalizedSyncStatus(value: unknown): SyncStatus {
  if (value === undefined || value === null) return emptyStatus();
  const invalid = (): SyncStatus => ({ ...emptyStatus(), phase: "error", error: "同步状态格式无效，请重新打开应用。" });
  if (typeof value !== "object") return invalid();
  const s = value as Partial<SyncStatus>;
  // 旧单机快照缺字段可兼容；登录状态缺账号身份时不能继续发出共享写操作。
  const accountId = s.account_id === undefined && s.email === null ? null : s.account_id;
  if (!(accountId === null || typeof accountId === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(accountId))) return invalid();
  if (accountId === null && s.email !== null && s.phase !== "awaiting_import") return invalid();
  const text = (v: unknown, max: number): v is string | null => v === null || (typeof v === "string" && v.length <= max);
  const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0 && (v as number) <= 10000;
  if (typeof s.configured !== "boolean" || !PHASES.includes(s.phase as SyncStatus["phase"])
    || !text(s.email, 254) || !Number.isSafeInteger(s.revision) || s.revision! < 0
    || typeof s.pending !== "boolean" || !text(s.error, 2000) || !text(s.recovery_path, 4096)
    || !(s.last_synced_at === null || (Number.isSafeInteger(s.last_synced_at) && s.last_synced_at! >= 0 && s.last_synced_at! <= 253402300799))) return invalid();
  const m = s.migration;
  if (m !== null && (!m || typeof m !== "object" || !count(m.local_days) || !count(m.cloud_days)
    || typeof m.local_active !== "boolean" || typeof m.cloud_active !== "boolean" || typeof m.cloud_empty !== "boolean")) return invalid();
  const c = s.conflict;
  if (c !== null && (!c || typeof c !== "object" || !count(c.local_days) || !count(c.remote_days)
    || typeof c.local_active !== "boolean" || typeof c.remote_active !== "boolean"
    || !Array.isArray(c.fields) || c.fields.length > 128 || !c.fields.every((f) => typeof f === "string" && f.length <= 256))) return invalid();
  if (s.phase === "awaiting_import" && !m || s.phase === "conflict" && !c) return invalid();
  return { ...s, account_id: accountId } as SyncStatus;
}

export function validSyncEmail(value: string): boolean {
  return value.length <= 254 && /^[^\s@<>"\\\u0000-\u001f]{1,64}@[a-zA-Z0-9](?:[a-zA-Z0-9.-]*[a-zA-Z0-9])?\.[a-zA-Z]{2,63}$/.test(value);
}

export function validSyncCode(value: string): boolean { return /^\d{6,10}$/.test(value); }

export function syncLabel(status: SyncStatus): string {
  switch (status.phase) {
    case "unconfigured": return "本机保存";
    case "signed_out": return "尚未登录";
    case "awaiting_import": return "选择要同步的数据";
    case "synced": return "已同步";
    case "pending": return "待同步";
    case "offline": return status.pending ? "离线 · 待同步" : "离线";
    case "conflict": return "有冲突待处理";
    case "reauth": return "需要重新登录";
    case "error": return "同步遇到问题";
  }
}
