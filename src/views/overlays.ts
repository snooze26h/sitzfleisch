// 覆盖层：提示条、确认对话框、两步确认的解除单（网站规则与应用屏蔽）、选择屏蔽应用。

import { duration, esc } from "../format";
import { MAX_COMPLETION_NOTE_CHARS } from "../types";
import { icon } from "../icons";
import { btn } from "../components";
import { day, dialogIsBusy, prefs, topOverlay, ui } from "../state";
import { conflictingHost } from "../blocking";
import { appBlocking, MAX_BLOCKED_APPS, sameText } from "../app-blocking";
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
    case "picker":
      return appPickerSheet();
    case "completion":
      return completionSheet();
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
  const extension = d.extension ? `<div class="extension-editor">
    <label for="extension-minutes">增加分钟数</label>
    <div class="extension-value"><input id="extension-minutes" class="field" type="number" inputmode="numeric" min="1" max="${d.extension.max}" step="1" value="${esc(d.extension.minutes)}" data-input="extend" ${busy ? "disabled" : ""} /><span>分钟</span></div>
    <div class="extension-presets" aria-label="常用延长时间">${[5, 10, 15, 30].filter((n) => n <= d.extension!.max).map((n) => btn(`${n} 分钟`, { action: "extend-preset", kind: "plate", data: { minutes: n }, disabled: busy })).join("")}</div>
  </div>` : "";
  return `<div class="backdrop" id="dialog" data-action="dialog-cancel"><div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title" aria-describedby="dialog-detail" data-stop>
      <h2 id="dialog-title">${esc(d.title)}</h2>
      <p id="dialog-detail">${esc(d.message)}</p>
      ${extension}
      <div class="btns"><span class="spacer"></span>${cancel}${alt}${confirm}</div>
    </div></div>`;
}

function completionSheet(): string {
  const entry = ui.completion!;
  return `<div class="backdrop" id="completion"><div class="dialog completion-dialog" role="dialog" aria-modal="true" aria-labelledby="completion-title" aria-describedby="completion-detail" data-stop>
    <h2 id="completion-title">这段时间完成了什么？</h2>
    <p class="completion-meta">${esc(entry.title)} · 已专注 ${esc(duration(entry.seconds))}</p>
    <p id="completion-detail">记下实际完成的内容，之后也可以在记录中补充。</p>
    <label class="sr-only" for="completion-note">实际完成的内容</label>
    <textarea id="completion-note" class="field" data-input="completion" rows="5" maxlength="${MAX_COMPLETION_NOTE_CHARS}" placeholder="例如：读完论文的方法部分，跑完两组对照实验。" ${entry.busy ? "disabled" : ""}>${esc(entry.draft)}</textarea>
    <div class="completion-foot"><span class="completion-error" role="status">${esc(entry.error)}</span><span>${entry.draft.length} / ${MAX_COMPLETION_NOTE_CHARS}</span></div>
    <div class="btns"><span class="spacer"></span>${btn(entry.automatic ? "先跳过" : "取消", { kind: "plate", action: "completion-cancel", disabled: entry.busy })}${btn(entry.busy ? "正在保存…" : "保存记录", { kind: "primary", action: "completion-save", disabled: entry.busy })}</div>
  </div></div>`;
}

function removalSheet(): string {
  const r = ui.removal!;
  const active = !!day();
  const blocking = ui.snap!.blocking;
  const busy = ui.pendingPrefs > 0;
  const guardrail = "这个动作是为了防止你在想刷的时候顺手删掉规则。";
  const forApps = r.kind === "app" || r.kind === "app-blocking";
  let warning: string;
  if (r.kind === "app") warning = `确认后，「${r.confirm}」会移出屏蔽名单，马上就能打开。这一步是为了防止你在想刷的时候顺手放开它。`;
  else if (r.kind === "app-blocking") warning = `确认后，屏蔽名单里的 ${appBlocking(prefs()).apps.length} 个应用马上都能打开。名单会留着，下次打开开关就恢复屏蔽。这一步是为了防止你在想刷的时候顺手关掉屏蔽。`;
  else if (r.kind === "url") {
    const conflict = conflictingHost(r.value, prefs().blocked_hosts);
    warning = `确认后会移除这条精确网址规则，浏览器扩展将在下次同步时解除。${conflict ? `整站规则 ${conflict} 仍然会屏蔽这个页面。` : ""}${guardrail}`;
  } else if (!inTauri) warning = `预览模式：确认后会从模拟设置中移除这条整站规则。${guardrail}`;
  else if (active) warning = `确认后会从设置移除 ${r.value}，再尝试解除系统屏蔽。请完成管理员授权；授权取消或失败时，网站仍可能被屏蔽，需要在整站状态中重新应用规则。其他精确网址规则仍然保留。${guardrail}`;
  else if (blocking.active || blocking.busy || blocking.error) warning = `确认后会从设置移除 ${r.value}。系统可能仍有残留屏蔽，请在整站状态中点击「重新应用整站规则」并完成管理员授权，解除系统残留。其他精确网址规则仍然保留。${guardrail}`;
  else warning = `确认后，${r.value} 将从整站设置移除，下次学习日不再应用这条规则；其他精确网址规则仍然保留。${guardrail}`;
  let body: string;
  if (r.step === 1) {
    body = `<h2 id="removal-title">先停一下</h2>
      <p>${esc(warning)}</p>
      <div class="btns">${btn("保留屏蔽", { kind: "primary", action: "removal-cancel" })}<span class="spacer"></span>${btn("我仍要解除", { kind: "danger", action: "removal-next" })}</div>`;
  } else if (forApps) {
    // 应用名多是中文，比较时不分英文大小写；要输入的字放在框外面，照着打就行。
    const matches = sameText(r.typed, r.confirm);
    const what = r.kind === "app" ? "应用名" : "这句话";
    body = `<h2 id="removal-title">${r.kind === "app" ? "手动确认应用名" : "手动确认关闭"}</h2>
      <div class="confirm-field"><label for="removal-input">输入${what} <span class="rule-value sel">${esc(r.confirm)}</span></label><input id="removal-input" class="field md" data-input="removal" value="${esc(r.typed)}" placeholder="${esc(r.confirm)}" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""} /></div>
      <div class="btns">${btn("返回", { kind: "plate", action: "removal-back", disabled: busy })}${btn("取消", { kind: "quiet", action: "removal-cancel", disabled: busy })}<span class="spacer"></span>${btn(busy ? "正在保存…" : r.kind === "app" ? "确认移出" : "确认关闭", { kind: "danger", action: "removal-confirm", disabled: !matches || busy })}</div>`;
  } else {
    const matches = r.typed.trim() === r.value;
    const label = r.kind === "url" ? "完整网址" : "完整域名";
    body = `<h2 id="removal-title">手动确认${r.kind === "url" ? "网址" : "域名"}</h2>
      <div class="confirm-field"><label for="removal-input">输入${label} <span class="rule-value mono sel">${esc(r.value)}</span></label><input id="removal-input" class="field mono md" data-input="removal" value="${esc(r.typed)}" placeholder="${esc(r.value)}" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""} /></div>
      <div class="btns">${btn("返回", { kind: "plate", action: "removal-back", disabled: busy })}${btn("取消", { kind: "quiet", action: "removal-cancel", disabled: busy })}<span class="spacer"></span>${btn(busy ? "正在解除…" : "确认解除", { kind: "danger", action: "removal-confirm", disabled: !matches || busy })}</div>`;
  }
  return `<div class="backdrop" id="removal"><div class="dialog sheet" role="dialog" aria-modal="true" aria-labelledby="removal-title" data-stop>${body}</div></div>`;
}

function appPickerSheet(): string {
  const picker = ui.appPicker!;
  const listed = new Set(appBlocking(prefs()).apps.map((app) => app.package_name));
  const room = MAX_BLOCKED_APPS - listed.size;
  const query = picker.query.trim().toLocaleLowerCase();
  const visible = picker.apps
    .filter((app) => !query || app.label.toLocaleLowerCase().includes(query) || app.packageName.toLowerCase().includes(query));
  const full = picker.chosen.length >= room;
  const rows = visible.map((app) => {
    const inList = listed.has(app.packageName);
    const chosen = picker.chosen.includes(app.packageName);
    const on = inList || chosen;
    return `<button class="picker-app${on ? " on" : ""}" data-action="picker-toggle" data-package="${esc(app.packageName)}" aria-pressed="${on}" ${inList || (full && !chosen) ? "disabled" : ""}>
      <span class="picker-check" aria-hidden="true">${on ? icon("check", 14) : ""}</span>
      <span class="picker-name"><b>${esc(app.label)}</b><span class="mono">${esc(app.packageName)}</span></span>
      ${inList ? `<span class="picker-tag">已在名单</span>` : ""}
    </button>`;
  }).join("");
  const status = picker.loading ? "正在读取手机上的应用…"
    : picker.error ? picker.error
    : !picker.apps.length ? "没有读到可以屏蔽的应用。"
    : !visible.length ? "没有找到这个应用。" : "";
  const limited = picker.limited && !picker.loading
    ? `<div class="picker-limited"><p>系统只交出了系统自带的应用。允许坐功读取应用列表，才能看到你装的应用。</p>${picker.canRequestFullList ? btn(picker.requesting ? "等待系统授权…" : "允许读取", { kind: "plate", action: "picker-full-list", disabled: picker.requesting }) : ""}</div>`
    : "";
  const count = picker.chosen.length;
  return `<div class="backdrop" id="app-picker"><div class="dialog sheet app-picker" role="dialog" aria-modal="true" aria-labelledby="picker-title" data-stop>
    <h2 id="picker-title">添加要屏蔽的应用</h2>
    <label class="sr-only" for="picker-query">搜索应用</label>
    <input id="picker-query" class="field" data-input="picker-query" value="${esc(picker.query)}" placeholder="搜索应用名" autocomplete="off" spellcheck="false" ${picker.loading ? "disabled" : ""} />
    ${limited}
    <div class="picker-list" role="group" aria-label="手机上的应用">${status ? `<p class="picker-status" role="status">${esc(status)}</p>` : rows}</div>
    ${full && room > 0 ? `<p class="t-note">名单最多 ${MAX_BLOCKED_APPS} 个应用。</p>` : ""}
    <div class="btns">${btn("取消", { kind: "plate", action: "picker-cancel" })}<span class="spacer"></span>${btn(count ? `添加 ${count} 个` : "添加", { kind: "primary", action: "picker-confirm", disabled: !count || ui.pendingPrefs > 0 })}</div>
  </div></div>`;
}
