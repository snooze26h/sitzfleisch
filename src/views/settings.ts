// 「设置」：分区导航、项目目录与配额矩阵，改动自动保存。
// 每个控件改完立即生效，没有「保存」按钮；文字框在失焦或回车时提交。

import type { CategoryDef, ProfileDef } from "../types";
import { inTauri } from "../api";
import { conflictingHost, MAX_BLOCK_RULES, normalizeHost, normalizeUrl } from "../blocking";
import { ICON_NAMED } from "../types";
import { esc, meter, shortNameFrom } from "../format";
import { icon } from "../icons";
import { btn, hair, labelled, plate, select, stepper, toggle } from "../components";
import { BLOCK_OPTIONS, BREAK_OPTIONS, IDLE_OPTIONS, day, prefs, profileTotalMinutes, ui, withValue } from "../state";

/** 设置的分区：id → 标题 + 图标。侧栏在设置页直接列它们，一区一页。 */
export const SETTINGS_SECTIONS: [string, string, string][] = [
  ["projects", "项目", "layers"],
  ["tiers", "时间安排", "sliders-horizontal"],
  ["rhythm", "节奏", "timer"],
  ["hosts", "网站屏蔽", "shield"],
  ["body", "身体", "heart-pulse"],
  ["notify", "提醒", "bell"],
  ["about", "关于", "info"],
];

function sectionBody(id: string): string {
  switch (id) {
    case "tiers": return tierPlan();
    case "rhythm": return rhythm();
    case "hosts": return websiteBlock();
    case "body": return bodyPanel();
    case "notify": return notificationPanel();
    case "about": return aboutPanel();
    default: return projectCatalog();
  }
}

export function settingsPage(): string {
  const current = SETTINGS_SECTIONS.find(([id]) => id === ui.settingsSection) ?? SETTINGS_SECTIONS[0];
  const [id, title] = current;
  // 一次只画一个分区：整页一根滚动条到底是上一版最难用的地方。
  return `<header class="page-heading"><h1>${esc(title)}</h1><p role="status">${ui.pendingPrefs > 0 ? "正在保存…" : "更改自动保存"}</p></header>`
    + sectionBody(id).replace('<section class="plate"', `<section class="plate settings-section" id="panel-${esc(id)}" tabindex="-1" aria-label="${esc(title)}"`);
}

function settingRow(title: string, detail: string, control: string): string {
  return `<div class="setting-row"><span class="titles"><b>${esc(title)}</b>${detail ? `<span>${esc(detail)}</span>` : ""}</span><span class="ctl">${control}</span></div>`;
}

// ---------- 项目 ----------

function projectCatalog(): string {
  const p = prefs();
  const rows = p.categories.map(projectRow).join(hair());
  const inner = `<div class="settings-panel">
    <div class="panel-meta">${btn("添加项目", { kind: "plate", action: "add-project" })}</div>
    ${rows}
  </div>`;
  return plate(inner);
}

function projectRow(c: CategoryDef): string {
  const open = ui.expandedProject === c.id;
  const header = `<button class="project-row" id="project-${esc(c.id)}" data-action="expand-project" data-id="${esc(c.id)}" aria-expanded="${open}" aria-label="${open ? "收起" : "编辑"}项目 ${esc(c.name)}">
      ${icon(open ? "chevron-down" : "chevron-right", 11, "chev")}
      <span class="glyph-badge">${icon(c.icon, 15)}</span>
      <span class="nm">${esc(c.name)}</span>
    </button>`;
  return open ? header + projectEditor(c) : header;
}

function projectEditor(c: CategoryDef): string {
  const names = ICON_NAMED.some(([name]) => name === c.icon) ? ICON_NAMED : [[c.icon, c.icon] as [string, string], ...ICON_NAMED];
  const glyphs = names
    .map(
      ([name, label]) =>
        `<button class="glyph${c.icon === name ? " on" : ""}" data-action="set-icon" data-id="${esc(c.id)}" data-icon="${esc(name)}" title="${esc(label)}" aria-label="${esc(label)}" aria-pressed="${c.icon === name}">${icon(name, 16)}</button>`
    )
    .join("");
  const uniform = prefs().uniform_block_minutes;
  return `<div class="project-editor" id="editor-${esc(c.id)}">
    <div class="identity">
      ${labelled("名称", `<input class="field sm" style="width:150px" data-change="project-name" data-id="${esc(c.id)}" value="${esc(c.name)}" placeholder="项目名称" aria-label="编辑项目名称 ${esc(c.name)}" />`)}
      ${labelled("短名", `<input class="field sm" style="width:84px" data-change="project-short-name" data-id="${esc(c.id)}" value="${esc(c.short_name)}" placeholder="${esc(shortNameFrom(c.name))}" aria-label="${esc(c.name)}在侧栏与菜单栏上的短名，留空自动" />`)}
      ${labelled("一格", select({ change: "block-length", data: { id: c.id }, value: uniform > 0 ? uniform : c.default_block_minutes, options: withValue(BLOCK_OPTIONS, uniform > 0 ? uniform : c.default_block_minutes).map((n) => ({ value: n, label: `${n} 分` })), width: 84, label: `${c.name}默认一格多长`, disabled: uniform > 0 }))}
      ${uniform > 0 ? `<span class="t-note">节奏里开着「统一块长」，所有项目都用 ${uniform} 分</span>` : ""}
    </div>
    <div class="s-field"><span class="engraved">图标</span><div class="glyphs">${glyphs}</div></div>
    <div class="foot">${btn("删除项目", { kind: "quiet-danger", action: "del-project", data: { id: c.id } })}</div>
  </div>`;
}

// ---------- 时间安排 ----------

function quotaRow(c: CategoryDef, profile: ProfileDef): string {
  const minutes = profile.quotas.find((q) => q.category === c.id)?.minutes ?? 0;
  const key = `${encodeURIComponent(profile.id)}:${encodeURIComponent(c.id)}`;
  const data = `data-profile="${esc(profile.id)}" data-category="${esc(c.id)}"`;
  const adjust = (delta: number, glyph: string, verb: string) =>
    `<button id="quota-${glyph}-${esc(key)}" data-action="step" data-bind="quota" data-delta="${delta}" data-step="15" data-min="0" data-max="1440" ${data} ${delta < 0 ? minutes <= 0 ? "disabled" : "" : minutes >= 1440 ? "disabled" : ""} aria-label="${esc(c.name)}${verb} 15 分钟">${icon(glyph, 12)}</button>`;
  return `<div class="plan-row${minutes === 0 ? " off" : ""}" id="quota-row-${esc(c.id)}">
    <span class="who">${icon(c.icon, 15)}<span class="nm">${esc(c.name)}</span></span>
    <span class="edit">${adjust(-1, "minus", "减少")}<input id="quota-field-${esc(key)}" class="field plan-input" type="number" inputmode="numeric" min="0" max="1440" step="1" value="${esc(minutes)}" data-change="quota-minutes" ${data} aria-label="${esc(c.name)}目标分钟数" />${adjust(1, "plus", "增加")}</span>
    <span class="hrs">${minutes === 0 ? "不排" : esc(meter(minutes * 60))}</span>
  </div>`;
}

function tierPlan(): string {
  const p = prefs();
  const plan = p.profiles[0];
  if (!plan) return plate(`<div class="settings-panel"><p class="t-note">还没有项目。</p></div>`);
  const rows = p.categories.map((c) => quotaRow(c, plan)).join("");
  const total = profileTotalMinutes(plan);
  const inner = `<div class="settings-panel">
    <div class="panel-meta"><span class="t-note">单位：分钟</span></div>
    <div class="plan-rows">${rows}</div>
    ${hair()}
    <div class="plan-total"><span class="engraved">总目标</span><span class="mono num">${esc(meter(total * 60))}</span></div>
  </div>`;
  return plate(inner);
}

// ---------- 节奏 ----------

function rhythm(): string {
  const p = prefs();
  const idle = p.idle_reminder_enabled ? p.idle_reminder_minutes : 0;
  const uniform = p.uniform_block_minutes;
  const inner = `<div class="settings-panel">
    ${settingRow("每格之后休息", "", select({ change: "break-default", value: p.break_minutes, options: withValue(BREAK_OPTIONS, p.break_minutes).map((n) => ({ value: n, label: n === 0 ? "不休息" : `${n} 分` })), width: 104, label: "每格之后休息" }))}
    ${hair()}
    ${settingRow(
      "统一块长",
      uniform > 0 ? "所有项目都用这个长度，项目里的「一格」暂时不生效" : "按每个项目各自的「一格」",
      `${uniform > 0 ? select({ change: "uniform-length", value: uniform, options: withValue(BLOCK_OPTIONS, uniform).map((n) => ({ value: n, label: `${n} 分` })), width: 84, label: "统一块长" }) : ""}${toggle({ change: "uniform-toggle", checked: uniform > 0, label: "统一块长" })}`
    )}
    ${hair()}
    ${settingRow("暂停提醒", "未开格时按间隔提醒，显示本次暂停总时长", select({ change: "idle", value: idle, options: withValue(IDLE_OPTIONS, idle).map((n) => ({ value: n, label: n === 0 ? "关闭" : `${n} 分` })), width: 104, label: "暂停提醒间隔" }))}
    ${hair()}
    ${settingRow("登录时自动启动", ui.autostart === null ? "正在读取系统设置…" : "", toggle({ change: "autostart", checked: ui.autostart === true, disabled: ui.autostart === null, label: "登录时自动启动" }))}
  </div>`;
  return plate(inner);
}

// ---------- 网站屏蔽 ----------

interface BlockState {
  title: string;
  detail?: string;
  problem: boolean;
}

export function blockState(): BlockState {
  const b = ui.snap!.blocking;
  const hosts = prefs().blocked_hosts.length;
  if (!inTauri) return { title: "预览模式 · 不拦截网站", problem: false };
  if (b.busy) return { title: "正在应用整站规则 · 请完成系统授权", problem: false };
  if (b.error) return { title: "整站规则需要处理", detail: b.error, problem: true };
  if (!hosts) return { title: "未配置整站规则", problem: false };
  if (!day()) return { title: "下次学习日生效", problem: false };
  if (b.active) return { title: `系统规则已写入 · ${hosts} 个域名`, problem: false };
  return { title: "系统规则与设置不一致", detail: `应该屏蔽 ${hosts} 个网站，尚未确认生效。`, problem: true };
}

function browserBlockState(): BlockState {
  const b = ui.snap!.blocking.browser;
  if (!inTauri) return { title: "预览模式 · 扩展未连接", detail: "浏览器预览只演示设置，不会拦截网页。请在桌面应用中连接扩展。", problem: false };
  if (b.error) return { title: "扩展连接需要处理", detail: b.error, problem: true };
  if (!b.available) return { title: "本地连接服务未启动", detail: "请重启坐功，再检查扩展连接状态。", problem: true };
  if (!b.connected) return { title: "扩展未连接", detail: "需要在 Chrome 或 Edge 中安装并启用扩展，保持坐功运行。", problem: false };
  if (!b.synced) return { title: "扩展已连接 · 等待同步", detail: "约每 30 秒尝试同步规则，当前修改尚未确认生效。", problem: false };
  if (!prefs().blocked_urls.length) return { title: "扩展已连接 · 未配置精确网址", problem: false };
  return day()
    ? { title: "扩展已同步 · 精确网址生效中", detail: "仅在已连接的浏览器中拦截。", problem: false }
    : { title: "扩展已同步 · 下次学习日生效", problem: false };
}

function wholeSiteBrowserState(): BlockState {
  const b = ui.snap!.blocking.browser;
  if (!inTauri) return { title: "浏览器预览不执行整站拦截。", problem: false };
  if (!b.connected || b.error || !b.available) return { title: "浏览器整站拦截未连接，请安装或启用扩展。仅写入系统规则无法确认浏览器已拦截。", problem: true };
  if (!b.supports_hosts) return { title: "当前扩展只支持精确网址，请更新并重新加载扩展以启用整站拦截。", problem: true };
  if (!b.synced) return { title: "浏览器整站规则等待同步，请在扩展中点击「立即同步」。", problem: false };
  return { title: !prefs().blocked_hosts.length ? "浏览器已连接，添加整站规则后会同步。" : day() ? "浏览器已同步 · 整站拦截生效中" : "浏览器已同步 · 下次学习日生效", problem: false };
}

function statusLabel(state: BlockState): string {
  return `<span class="status-dot${state.problem ? " problem" : ""}"><i aria-hidden="true"></i>${esc(state.title)}</span>`;
}

function ruleRows(kind: "host" | "url", values: string[]): string {
  if (!values.length) return `<p class="blocking-note">${kind === "url" ? "还没有精确网址规则。" : "还没有整站规则。"}</p>`;
  return `<div class="blocking-rule-list">${values.map((value) => `<div class="blocking-rule">
    <span class="rule-content"><span class="rule-kind">${kind === "url" ? "精确网址" : "整个网站"}</span><span class="rule-value mono sel">${esc(value)}</span></span>
    ${btn("解除", { kind: "quiet-danger", action: "remove-rule", data: { kind, value }, title: `解除${kind === "url" ? "精确网址" : "整站"}规则 ${value}（需要两步确认）` })}
  </div>`).join("")}</div>`;
}

function ruleEditor(kind: "host" | "url"): string {
  const exact = kind === "url";
  const draft = exact ? ui.urlDraft : ui.hostDraft;
  const preview = draft.trim() ? (exact ? normalizeUrl(draft) : normalizeHost(draft)) : null;
  const error = preview && "error" in preview ? preview.error : null;
  const full = prefs().blocked_hosts.length + prefs().blocked_urls.length >= MAX_BLOCK_RULES;
  const detail = error ?? (preview && "url" in preview
    ? `仅屏蔽这个完整网址：${preview.url}`
    : preview && "host" in preview ? `将屏蔽 ${preview.host} 和 www.${preview.host} 下的所有页面，包括收藏和视频。` : "");
  return `<div class="blocking-editor">
    <label class="blocking-input-label" for="blocked-${kind}-input">${exact ? "完整网址" : "网站域名"}</label>
    <div class="host-add"><input id="blocked-${kind}-input" class="field md" data-input="${kind}" value="${esc(draft)}" aria-describedby="blocking-${kind}-help blocking-${kind}-preview" aria-invalid="${!!error}" placeholder="${exact ? "https://www.douyin.com/?recommend=1" : "douyin.com"}" autocomplete="off" spellcheck="false" />${btn(exact ? "添加精确网址" : "添加整站规则", { kind: "plate", action: `add-${kind}`, disabled: !preview || !!error || full || ui.pendingPrefs > 0 })}</div>
    <p class="blocking-note" id="blocking-${kind}-help" ${full ? "" : "hidden"}>已达到 ${MAX_BLOCK_RULES} 条上限，请先解除不需要的规则。</p>
    <p class="blocking-note${error ? " caution" : ""}" id="blocking-${kind}-preview" role="status" ${detail ? "" : "hidden"}>${esc(detail)}</p>
  </div>`;
}

function websiteBlock(): string {
  const p = prefs();
  const hostState = blockState();
  const wholeSiteState = wholeSiteBrowserState();
  const browserState = browserBlockState();
  const preview = ui.urlDraft.trim() ? normalizeUrl(ui.urlDraft) : null;
  const previewUrl = preview && "url" in preview ? preview.url : undefined;
  const conflict = previewUrl ? conflictingHost(previewUrl, p.blocked_hosts) : undefined;
  const existingConflicts = [...new Set(p.blocked_urls.map((url) => conflictingHost(url, p.blocked_hosts)).filter((host): host is string => !!host))];
  const conflicts = [...new Set([...existingConflicts, ...(conflict ? [conflict] : [])])];
  const warning = conflicts.length
    ? `<p class="blocking-conflict" role="status">整站规则 <b>${conflicts.map(esc).join("、")}</b> 仍会屏蔽对应网站的收藏和视频。若只想拦推荐页，请解除下方对应的整站规则。</p>`
    : "";
  const problem = hostState.problem
    ? `<div class="problem-block"><span class="titles"><b>${esc(hostState.title)}</b>${hostState.detail ? `<span>${esc(hostState.detail)}</span>` : ""}</span>${btn("重新应用整站规则", { kind: "plate", cls: "caution", action: "reapply-blocking" })}</div>`
    : "";
  const inner = `<div class="settings-panel website-block">
    <div class="panel-meta"><span class="t-note">${p.blocked_hosts.length + p.blocked_urls.length} / ${MAX_BLOCK_RULES} 条</span></div>
    ${warning}
    ${hair("mt13")}
    <div class="blocking-group-heading"><b>精确网址</b><span class="t-note">只拦这一个网址</span>${statusLabel(browserState)}</div>
    ${browserState.detail ? `<p class="blocking-note${browserState.problem ? " caution" : ""}">${esc(browserState.detail)}</p>` : ""}
    ${ruleEditor("url")}
    ${ruleRows("url", p.blocked_urls)}
    <details id="blocking-help" class="blocking-help" data-preserve-open><summary>安装浏览器扩展与填写示例</summary>
      <ol><li>打开 Chrome 的 <span class="mono sel">chrome://extensions</span> 或 Edge 的 <span class="mono sel">edge://extensions</span>，开启「开发者模式」。</li><li>点击下方按钮找到扩展目录，再在浏览器中选择「加载已解压的扩展程序」，选中该目录。</li><li>保持坐功运行；扩展约每 30 秒尝试同步规则。上方显示「已同步」后，在学习日期间生效。</li></ol>
      ${btn("打开扩展文件夹", { kind: "plate", action: "reveal-browser-extension" })}
      <p class="blocking-note">抖音推荐页：<span class="mono sel">https://www.douyin.com/?recommend=1</span>。收藏页路径不同，可以正常打开。</p>
      <p class="blocking-note">B 站首页：<span class="mono sel">https://www.bilibili.com/</span>。视频页 <span class="mono sel">/video/…</span> 可以正常打开。</p>
      <p class="blocking-note">精确匹配逐字比较：路径、参数、顺序或 # 后内容不同都会放行。拦截发生在页面导航之后，可能一闪。</p>
    </details>
    ${hair("mt13")}
    <div class="blocking-group-heading"><b>整个网站</b><span class="t-note">这个域名下全都不开</span>${statusLabel(hostState)}</div>
    <p class="blocking-note${wholeSiteState.problem ? " caution" : ""}" role="status">${esc(wholeSiteState.title)}</p>
    ${ruleEditor("host")}
    ${ruleRows("host", p.blocked_hosts)}
    ${problem}
    <div class="foot-note"><p>暂停和休息时规则保持生效，收工后解除。写入和解除系统 hosts 各需一次管理员授权。</p>${btn("核对整站规则", { kind: "quiet", action: "recheck-blocking", disabled: ui.snap!.blocking.busy })}</div>
  </div>`;
  return plate(inner);
}

// ---------- 身体 ----------

function bodyPanel(): string {
  const p = prefs();
  const water = `${p.water_reminder_enabled ? stepper({ bind: "water-min", value: p.water_reminder_minutes, label: `${p.water_reminder_minutes} 分`, min: 10, max: 180, step: 5, ariaLabel: "喝水提醒间隔" }) : ""}${toggle({ change: "water-on", checked: p.water_reminder_enabled, label: "喝水提醒" })}`;
  const stretch = `${p.stretch_reminder_enabled ? stepper({ bind: "stretch-min", value: p.stretch_reminder_minutes, label: `${p.stretch_reminder_minutes} 分`, min: 15, max: 180, step: 5, ariaLabel: "起身提醒间隔" }) : ""}${toggle({ change: "stretch-on", checked: p.stretch_reminder_enabled, label: "起身护眼提醒" })}`;
  const inner = `<div class="settings-panel">
    ${settingRow("喝水提醒", "只在专注计时中提醒，记一杯水后重新计时", water)}
    ${hair()}
    ${settingRow("每天喝水目标", "", stepper({ bind: "goal", value: p.hydration_goal_cups, label: `${p.hydration_goal_cups} 杯`, min: 1, max: 20, step: 1, ariaLabel: "每天喝水目标" }))}
    ${hair()}
    ${settingRow("起身 / 护眼提醒", "", stretch)}
  </div>`;
  return plate(inner);
}

// ---------- 提醒 ----------

function notificationStatusText(): string {
  switch (ui.notificationStatus) {
    case "granted":
      return "已允许";
    case "denied":
      return "系统里被拒了，改用界面里的提示条";
    case "unknown":
      return "还没允许，点右边申请";
    default:
      return "正在检查…";
  }
}

function notificationPanel(): string {
  const p = prefs();
  // 还没授权时才给「申请权限」——授权过之后这个按钮点了也没有反应，留着只会误导。
  const grant = ui.notificationStatus === "granted" || ui.notificationStatus === "checking"
    ? ""
    : btn("申请权限", { kind: "plate", action: "notif-recheck" });
  const inner = `<div class="settings-panel">
    ${settingRow("提示音", "提醒时响一声，窗口关着也听得见", toggle({ change: "sound", checked: p.sound_enabled, label: "提示音" }))}
    ${hair()}
    ${settingRow("系统通知", notificationStatusText(), `${grant}${btn("试一条", { kind: "plate", action: "notif-test" })}${btn("打开系统设置", { kind: "quiet", action: "notif-open" })}`)}
  </div>`;
  return plate(inner);
}

// ---------- 关于 ----------

function aboutPanel(): string {
  const inner = `<div class="settings-panel">
    ${settingRow("版本", "坐功 · Sitzfleisch", `<span class="mono t-note">${esc(ui.appVersion ?? "读取中…")}</span>`)}
    ${hair()}
    ${settingRow("数据只存在本机", ui.snap!.state_path, btn("在 Finder 中显示", { kind: "quiet", action: "reveal-state" }))}
  </div>`;
  return plate(inner);
}
