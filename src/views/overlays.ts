// 覆盖层：提示条、确认对话框、两步确认的网站解除单。

import { esc } from "../format";
import { icon } from "../icons";
import { btn } from "../components";
import { day, dialogIsBusy, topOverlay, ui } from "../state";

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
  const guardrail = "这个动作是为了防止你在想刷的时候顺手删掉规则。";
  let warning: string;
  if (active && blocking.active && !blocking.error) warning = `确认并成功解除后，${r.host} 会在当前学习日立即恢复访问。${guardrail}`;
  else if (active) warning = `当前系统规则尚未确认。确认后会尝试从当前规则解除 ${r.host}；只有系统回读成功才会移出列表。${guardrail}`;
  else warning = `解除后，${r.host} 将不会在之后的学习日被屏蔽。${guardrail}`;
  let body: string;
  if (r.step === 1) {
    body = `<h2>先停一下</h2>
      <p>${esc(warning)}</p>
      <div class="btns">${btn("保留屏蔽", { kind: "primary", action: "removal-cancel" })}<span class="spacer"></span>${btn("我仍要解除", { kind: "danger", action: "removal-next" })}</div>`;
  } else {
    const matches = r.typed.trim() === r.host;
    body = `<h2>手动确认域名</h2>
      <div class="confirm-field"><span>输入完整域名 ${esc(r.host)}</span><input class="field mono md" data-input="removal" value="${esc(r.typed)}" placeholder="${esc(r.host)}" autocomplete="off" spellcheck="false" /></div>
      <div class="btns">${btn("返回", { kind: "plate", action: "removal-back" })}${btn("取消", { kind: "quiet", action: "removal-cancel" })}<span class="spacer"></span>${btn("确认解除", { kind: "danger", action: "removal-confirm", disabled: !matches })}</div>`;
  }
  return `<div class="backdrop" id="removal"><div class="dialog sheet" role="dialog" aria-modal="true" data-stop>${body}</div></div>`;
}
