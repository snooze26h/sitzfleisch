// 覆盖层：提示条、确认对话框、两步确认的网站解除单。

import { esc } from "../format";
import { icon } from "../icons";
import { btn } from "../components";
import { day, dialogIsBusy, prefs, topOverlay, ui } from "../state";
import { conflictingHost } from "../blocking";
import { inTauri } from "../api";

export function toastView(): string {
  if (!ui.toast) return "";
  return `<div class="toast" id="toast" role="status">${icon("signpost", 13)}<span>${esc(ui.toast.message)}</span><button class="btn-icon" data-action="toast-close" aria-label="关闭提示">${icon("x", 11)}</button></div>`;
}

export function overlays(): string {
  // 顺序由 topOverlay() 一处说了算，键盘处理读的是同一个判据。
  switch (topOverlay()) {
    case "dialog":
      return dialogView();
    case "removal":
      return removalSheet();
    default:
      return "";
  }
}

function dialogView(): string {
  const d = ui.dialog!;
  // 这一下正在跑（比如「重试并退出」在等外壳）：三个键全禁用，免得重复触发。
  const busy = dialogIsBusy();
  const confirm = btn(d.confirmLabel, { kind: d.destructive ? "danger" : "primary", action: "dialog-confirm", disabled: busy });
  const cancel = btn(d.cancelLabel, { kind: d.destructive ? "primary" : "plate", action: "dialog-cancel", disabled: busy });
  const alt = d.alt ? btn(d.alt.label, { kind: "danger", action: "dialog-alt", disabled: busy }) : "";
  return `<div class="backdrop" id="dialog" data-action="dialog-cancel"><div class="dialog" role="dialog" aria-modal="true" data-stop>
      <h2>${esc(d.title)}</h2>
      <p>${esc(d.message)}</p>
      <div class="btns"><span class="spacer"></span>${cancel}${alt}${confirm}</div>
    </div></div>`;
}

function removalSheet(): string {
  const r = ui.removal!;
  const active = !!day();
  const blocking = ui.snap!.blocking;
  const busy = ui.pendingPrefs > 0;
  const guardrail = "这个动作是为了防止你在想刷的时候顺手删掉规则。";
  let warning: string;
  if (r.kind === "url") {
    const conflict = conflictingHost(r.value, prefs().blocked_hosts);
    warning = `确认后会移除这条精确网址规则，浏览器扩展将在下次同步时解除。${conflict ? `整站规则 ${conflict} 仍然会屏蔽这个页面。` : ""}${guardrail}`;
  } else if (!inTauri) warning = `预览模式：确认后会从模拟设置中移除这条整站规则。${guardrail}`;
  else if (active) warning = `确认后会从设置移除 ${r.value}，再尝试解除系统屏蔽。请完成管理员授权；授权取消或失败时，网站仍可能被屏蔽，需要在整站状态中重新应用规则。其他精确网址规则仍然保留。${guardrail}`;
  else if (blocking.active || blocking.busy || blocking.error) warning = `确认后会从设置移除 ${r.value}。系统可能仍有残留屏蔽，请在整站状态中点击「重新应用整站规则」并完成管理员授权，解除系统残留。其他精确网址规则仍然保留。${guardrail}`;
  else warning = `确认后，${r.value} 将从整站设置移除，下次学习日不再应用这条规则；其他精确网址规则仍然保留。${guardrail}`;
  let body: string;
  if (r.step === 1) {
    body = `<h2>先停一下</h2>
      <p>${esc(warning)}</p>
      <div class="btns">${btn("保留屏蔽", { kind: "primary", action: "removal-cancel" })}<span class="spacer"></span>${btn("我仍要解除", { kind: "danger", action: "removal-next" })}</div>`;
  } else {
    const matches = r.typed.trim() === r.value;
    const label = r.kind === "url" ? "完整网址" : "完整域名";
    body = `<h2>手动确认${r.kind === "url" ? "网址" : "域名"}</h2>
      <div class="confirm-field"><label for="removal-input">输入${label} <span class="rule-value mono sel">${esc(r.value)}</span></label><input id="removal-input" class="field mono md" data-input="removal" value="${esc(r.typed)}" aria-label="输入${label}以确认解除" placeholder="${esc(r.value)}" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""} /></div>
      <div class="btns">${btn("返回", { kind: "plate", action: "removal-back", disabled: busy })}${btn("取消", { kind: "quiet", action: "removal-cancel", disabled: busy })}<span class="spacer"></span>${btn(busy ? "正在解除…" : "确认解除", { kind: "danger", action: "removal-confirm", disabled: !matches || busy })}</div>`;
  }
  return `<div class="backdrop" id="removal"><div class="dialog sheet" role="dialog" aria-modal="true" data-stop>${body}</div></div>`;
}
