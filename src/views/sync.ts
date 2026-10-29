// 账号是现有设置页的一节：沿用细线、表单与按钮，数据选择和错误留在操作附近。
import type { SyncStatus } from "../types";
import { btn } from "../components";
import { esc, nowUnix } from "../format";
import { icon } from "../icons";
import { ui } from "../state";
import { normalizedSyncStatus, syncLabel, validSyncCode, validSyncEmail } from "../sync";

function caution(status: SyncStatus): boolean {
  return ["offline", "conflict", "reauth", "error"].includes(status.phase);
}

/** 今天页只放一行状态入口；未启用同步时不占据计时空间。 */
export function syncIndicator(): string {
  const s = normalizedSyncStatus(ui.snap?.sync);
  if (s.phase === "unconfigured" || s.phase === "signed_out") return "";
  return `<button class="sync-indicator${caution(s) ? " caution" : ""}" data-action="jump-settings" data-id="account" aria-label="${esc(syncLabel(s))}，打开账号与同步">${icon(caution(s) ? "triangle-alert" : "cloud", 15)}<span>${esc(syncLabel(s))}</span>${icon("chevron-right", 13)}</button>`;
}

function loginForm(s: SyncStatus): string {
  const busy = ui.syncBusy;
  const reauth = s.phase === "reauth";
  const email = reauth && s.email ? s.email : ui.syncEmail;
  const sent = !!ui.syncSentTo;
  // 重发节流是本机交互间隔，不能混用已经过服务端校准的学习日时钟。
  const wait = Math.max(0, ui.syncResendAt - nowUnix());
  return `<div class="sync-login" aria-busy="${busy}">
    <p class="sync-note">${reauth ? "登录已过期，本机记录仍保留。验证同一邮箱后继续同步。" : "在电脑和手机登录同一邮箱，就能接着同一段专注，查看相同的项目和记录。"}</p>
    <div class="sync-form-field"><label for="sync-email">邮箱</label><input id="sync-email" class="field md" type="email" inputmode="email" autocomplete="email" autocapitalize="none" spellcheck="false" maxlength="254" data-input="sync-email" value="${esc(email)}" placeholder="you@example.com" ${busy ? "disabled" : ""} ${sent || reauth ? "readonly" : ""} aria-describedby="sync-form-note${ui.syncError ? " sync-form-error" : ""}" /></div>
    ${sent ? `<p class="sync-note" id="sync-form-note">验证码已发送至 ${esc(ui.syncSentTo)}。如未收到，也请查看垃圾邮件。</p><div class="sync-form-field"><label for="sync-code">邮件验证码</label><input id="sync-code" class="field md sync-code" type="text" inputmode="numeric" autocomplete="one-time-code" maxlength="10" data-input="sync-code" value="${esc(ui.syncCode)}" ${busy ? "disabled" : ""} aria-describedby="sync-form-note${ui.syncError ? " sync-form-error" : ""}" /></div>` : `<p class="sync-note" id="sync-form-note">用邮件验证码登录，无需设置密码。登录后会先预览两端数据，再开始同步。</p>`}
    ${ui.syncError ? `<p class="sync-error" id="sync-form-error" role="alert">${esc(ui.syncError)}</p>` : ""}
    <div class="sync-actions">
      ${sent ? btn(busy ? "正在验证…" : "验证并登录", { kind: "primary", action: "sync-verify", disabled: busy || !validSyncCode(ui.syncCode.trim()) }) : btn(busy ? "正在发送…" : wait > 0 ? `${wait} 秒后可发送` : "发送验证码", { kind: "primary", action: "sync-send-code", disabled: busy || !validSyncEmail(email.trim()) || wait > 0 })}
      ${sent ? btn(wait > 0 ? `${wait} 秒后可重发` : "重新发送", { kind: "quiet", action: "sync-send-code", disabled: busy || wait > 0 }) : ""}
      ${sent && !reauth ? btn("更换邮箱", { kind: "quiet", action: "sync-change-email", disabled: busy }) : ""}
    </div>
    ${ui.syncPreview ? `<p class="sync-note">浏览器预览使用验证码 123456，不会发送邮件。</p>` : ""}
  </div>`;
}

function comparison(localDays: number, cloudDays: number, localActive: boolean, cloudActive: boolean): string {
  return `<dl class="sync-comparison"><div><dt>这台设备</dt><dd>${localDays} 个历史学习日<span>${localActive ? "有未结束的学习日" : "没有正在进行的学习日"}</span></dd></div><div><dt>云端</dt><dd>${cloudDays} 个历史学习日<span>${cloudActive ? "有未结束的学习日" : "没有正在进行的学习日"}</span></dd></div></dl>`;
}

function migrationPanel(s: SyncStatus): string {
  const m = s.migration!;
  return `<div class="sync-choice">
    <h2>${m.cloud_empty ? "把这台设备的数据带到账号里" : "选择如何接上云端数据"}</h2>
    ${comparison(m.local_days, m.cloud_days, m.local_active, m.cloud_active)}
    <p class="sync-note">${m.cloud_empty ? "云端还没有记录。开始后，其他设备登录同一邮箱就能接着使用。" : "合并会保留两边独立的历史。项目安排或当前计时不同时，按下面选择的一端处理。"} 这台设备原来的单机存档会保留。</p>
    ${m.local_active && m.cloud_active ? `<p class="sync-attention">两边都有未结束的学习日，请选定要接着使用的那一份。</p>` : ""}
    <div class="sync-actions">
      ${btn(m.cloud_empty ? "开始同步这台设备" : "合并，云端优先", { kind: "primary", action: "sync-connect", data: { strategy: m.cloud_empty ? "merge_local" : "merge_remote" }, disabled: ui.syncBusy })}
      ${m.cloud_empty ? "" : btn("合并，本机优先", { action: "sync-connect", data: { strategy: "merge_local" }, disabled: ui.syncBusy })}
      ${m.cloud_empty ? "" : btn("只使用云端", { kind: "quiet", action: "sync-connect", data: { strategy: "cloud_only" }, disabled: ui.syncBusy })}
      ${btn("暂不连接", { kind: "quiet", action: "sync-cancel", disabled: ui.syncBusy })}
    </div>
  </div>`;
}

function conflictPanel(s: SyncStatus): string {
  const c = s.conflict!;
  const changed = [...new Set(c.fields.map((field) => field.startsWith("preferences") ? "项目与计划"
    : field.startsWith("day") ? "当前学习日" : field.startsWith("history") ? "历史记录" : "共享记录"))].join("、");
  return `<div class="sync-choice">
    <h2>两端都改过，需要选一份</h2>
    ${changed ? `<p class="sync-attention">涉及${esc(changed)}。</p>` : ""}
    ${comparison(c.local_days, c.remote_days, c.local_active, c.remote_active)}
    <p class="sync-note">处理前会在本机保存两份恢复副本。两边独立的历史仍会合并；只有冲突部分采用你选的那一份。</p>
    <div class="sync-actions">${btn("采用云端的冲突内容", { kind: "plate", action: "sync-resolve", data: { choice: "remote" }, disabled: ui.syncBusy })}${btn("采用本机的冲突内容", { kind: "plate", action: "sync-resolve", data: { choice: "local" }, disabled: ui.syncBusy })}</div>
  </div>`;
}

export function accountPanel(): string {
  const s = normalizedSyncStatus(ui.snap?.sync);
  if (!s.configured) return `<div class="settings-panel sync-panel"><p class="sync-note">${s.error ? esc(s.error) : "此版本尚未配置云同步，部署后可用。"}</p><p class="sync-note">当前项目、计时和记录继续保存在这台设备上。</p></div>`;
  const signingIn = s.phase === "signed_out" || s.phase === "reauth" || (!s.email && s.phase === "error");
  const status = `<div class="sync-account"><div>${s.email ? `<b class="sync-email">${esc(s.email)}</b>` : ""}<span class="sync-phase${caution(s) ? " caution" : ""}" role="status">${esc(syncLabel(s))}</span></div>${s.email && s.phase !== "awaiting_import" ? btn("退出登录", { kind: "quiet", action: "sync-sign-out", disabled: ui.syncBusy }) : ""}</div>`;
  const details = signingIn ? loginForm(s) : s.phase === "awaiting_import" ? migrationPanel(s) : s.phase === "conflict" ? conflictPanel(s)
    : `<p class="sync-note">${s.pending ? "有改动等待上传，联网后会继续同步。跨设备接力前，请确认两端都显示“已同步”。" : "项目、当天进度和记录共用一份。共享计时在合盖和锁屏后继续，手动暂停才会停下。"}</p>
      ${s.last_synced_at !== null ? `<p class="sync-note">最近同步：${esc(new Date(s.last_synced_at * 1000).toLocaleString("zh-CN", { hour12: false }))}</p>` : ""}
      <div class="sync-actions">${btn(ui.syncBusy ? "正在同步…" : "立即同步", { action: "sync-retry", icon: "refresh-cw", disabled: ui.syncBusy })}</div>`;
  return `<div class="settings-panel sync-panel" aria-busy="${ui.syncBusy}">${status}${details}
    ${!signingIn && ui.syncError ? `<p class="sync-error" role="alert">${esc(ui.syncError)}</p>` : ""}
    ${s.error ? `<p class="sync-error" role="alert">${esc(s.error)}</p>` : ""}
    ${s.recovery_path ? `<details class="sync-recovery" data-preserve-open id="sync-recovery"><summary>查看恢复副本位置</summary><p>${esc(s.recovery_path)}</p></details>` : ""}
    <p class="sync-note sync-boundary">提醒、权限和屏蔽设置留在各自设备上。手机后台同步可能延后，回到应用时会补同步。</p>
  </div>`;
}
