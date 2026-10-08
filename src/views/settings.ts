// 「设置」：分区导航、项目目录与配额矩阵，改动自动保存。
// 每个控件改完立即生效，没有「保存」按钮；文字框在失焦或回车时提交。

import type { CategoryDef, ProfileDef } from "../types";
import { inTauri } from "../api";
import { conflictingHost, MAX_BLOCK_RULES, normalizeHost, normalizeUrl } from "../blocking";
import { appBlocking, MAX_BLOCKED_APPS } from "../app-blocking";
import { ICON_NAMED, NOTIFICATION_CHANNELS } from "../types";
import { esc, meter, shortNameFrom } from "../format";
import { icon } from "../icons";
import { PROJECT_ICON_CHOICES } from "../project-icons";
import { btn, labelled, select, stepper, toggle } from "../components";
import { BLOCK_OPTIONS, BREAK_OPTIONS, IDLE_OPTIONS, day, hasFeature, honorPhone, prefs, profileTotalMinutes, ui, withValue } from "../state";

/** 设置的分区：id → 标题 + 图标。侧栏在设置页直接列它们，一区一页。 */
const SETTINGS_SECTIONS: [string, string, string][] = [
  ["projects", "项目", "layers"],
  ["tiers", "时间安排", "sliders-horizontal"],
  ["rhythm", "节奏", "timer"],
  ["hosts", "网站屏蔽", "shield"],
  ["apps", "应用屏蔽", "shield"],
  ["body", "身体", "heart-pulse"],
  ["notify", "提醒", "bell"],
  ["about", "关于", "info"],
];

export function settingsSections(): [string, string, string][] {
  return SETTINGS_SECTIONS.filter(([id]) => (id !== "hosts" || hasFeature("website_blocking"))
    && (id !== "apps" || hasFeature("app_blocking")));
}

function sectionBody(id: string): string {
  switch (id) {
    case "tiers": return tierPlan();
    case "rhythm": return rhythm();
    case "hosts": return websiteBlock();
    case "apps": return appBlockPanel();
    case "body": return bodyPanel();
    case "notify": return notificationPanel();
    case "about": return aboutPanel();
    default: return projectCatalog();
  }
}

export function settingsPage(): string {
  const sections = settingsSections();
  if (ui.compact && ui.settingsIndex) {
    return `<header class="page-heading"><div class="heading-copy"><h1>设置</h1></div></header><nav class="settings-index" aria-label="设置分区">${sections.map(([id, label, ic]) =>
      `<button class="settings-link" data-action="jump-settings" data-id="${id}">${icon(ic, 20)}<span>${label}</span>${icon("chevron-right", 16, "chev")}</button>`
    ).join("")}</nav>`;
  }
  const current = sections.find(([id]) => id === ui.settingsSection) ?? sections[0];
  const [id, title] = current;
  // 一次只画一个分区：整页一根滚动条到底是上一版最难用的地方。
  return `${ui.compact ? `<button class="settings-back" data-action="settings-index" aria-label="返回设置索引">${icon("chevron-left", 18)}<span>设置</span></button>` : ""}<header class="page-heading settings-heading"><div class="heading-copy"><h1>${esc(title)}</h1><p role="status">${ui.pendingPrefs > 0 ? "正在保存…" : ""}</p></div></header>`
    + `<section class="settings-section" id="panel-${esc(id)}" tabindex="-1" aria-label="${esc(title)}">${sectionBody(id)}</section>`;
}

function settingRow(title: string, detail: string, control: string, cls = ""): string {
  return `<div class="setting-row${cls ? ` ${cls}` : ""}"><span class="titles"><b>${esc(title)}</b>${detail ? `<span>${esc(detail)}</span>` : ""}</span><span class="ctl">${control}</span></div>`;
}

// ---------- 项目 ----------

function projectCatalog(): string {
  const p = prefs();
  const rows = p.categories.map(projectRow).join("");
  return `<div class="settings-panel">
    <div class="panel-meta">${btn("添加项目", { kind: "plate", action: "add-project", icon: "plus" })}</div>
    <div class="project-list">${rows}</div>
  </div>`;
}

function projectRow(c: CategoryDef): string {
  const open = ui.expandedProject === c.id;
  const header = `<button class="project-row" id="project-${esc(c.id)}" data-action="expand-project" data-id="${esc(c.id)}" aria-expanded="${open}" aria-label="${open ? "收起" : "编辑"}项目 ${esc(c.name)}">
      ${icon(open ? "chevron-down" : "chevron-right", 14, "chev")}
      <span class="glyph-badge">${icon(c.icon, 22)}</span>
      <span class="nm">${esc(c.name)}</span>
    </button>`;
  return `<div class="project-item${open ? " open" : ""}">${open ? header + projectEditor(c) : header}</div>`;
}

function projectEditor(c: CategoryDef): string {
  const names = PROJECT_ICON_CHOICES.some(([name]) => name === c.icon) ? PROJECT_ICON_CHOICES
    : [[c.icon, ICON_NAMED.find(([name]) => name === c.icon)?.[1] ?? "当前图标"] as [string, string], ...PROJECT_ICON_CHOICES];
  const glyphs = names
    .map(
      ([name, label]) =>
        `<button class="glyph project-choice${c.icon === name ? " on" : ""}" data-action="set-icon" data-id="${esc(c.id)}" data-icon="${esc(name)}" title="${esc(label)}" aria-label="${esc(label)}" aria-pressed="${c.icon === name}">${icon(name, 26)}<span>${esc(label)}</span></button>`
    )
    .join("");
  const otherGlyphs = ICON_NAMED.filter(([name]) => !names.some(([primary]) => primary === name))
    .map(([name, label]) => `<button class="glyph" data-action="set-icon" data-id="${esc(c.id)}" data-icon="${esc(name)}" title="${esc(label)}" aria-label="${esc(label)}" aria-pressed="false">${icon(name, 19)}</button>`).join("");
  return `<div class="project-editor" id="editor-${esc(c.id)}">
    <div class="identity">
      ${labelled("名称", `<input class="field sm" style="width:150px" data-change="project-name" data-id="${esc(c.id)}" value="${esc(c.name)}" placeholder="项目名称" aria-label="编辑项目名称 ${esc(c.name)}" />`)}
      ${labelled("短名", `<input class="field sm" style="width:84px" data-change="project-short-name" data-id="${esc(c.id)}" value="${esc(c.short_name)}" placeholder="${esc(shortNameFrom(c.name))}" aria-label="${esc(c.name)}在侧栏与菜单栏上的短名，留空自动" />`)}
    </div>
    <div class="s-field"><span class="engraved">图标</span><div class="glyphs project-glyphs" role="group" aria-label="项目图标">${glyphs}</div><details class="more-icons" id="more-icons-${esc(c.id)}" data-preserve-open><summary>${icon("chevron-right", 14)}<span>更多图标</span></summary><div class="glyphs" role="group" aria-label="通用图标">${otherGlyphs}</div></details></div>
    <div class="foot">${btn("删除项目", { kind: "quiet-danger", action: "del-project", data: { id: c.id } })}</div>
  </div>`;
}

// ---------- 时间安排 ----------

function quotaRow(c: CategoryDef, profile: ProfileDef): string {
  const minutes = profile.quotas.find((q) => q.category === c.id)?.minutes ?? 0;
  const key = `${encodeURIComponent(profile.id)}:${encodeURIComponent(c.id)}`;
  const data = `data-profile="${esc(profile.id)}" data-category="${esc(c.id)}"`;
  const adjust = (delta: number, glyph: string, verb: string) =>
    `<button id="quota-${glyph}-${esc(key)}" data-action="step" data-bind="quota" data-delta="${delta}" data-step="15" data-min="0" data-max="1440" ${data} ${delta < 0 ? minutes <= 0 ? "disabled" : "" : minutes >= 1440 ? "disabled" : ""} aria-label="${esc(c.name)}${verb} 15 分钟">${icon(glyph, 13)}</button>`;
  return `<div class="plan-row${minutes === 0 ? " off" : ""}" id="quota-row-${esc(c.id)}">
    <span class="who">${icon(c.icon, 20)}<span class="nm">${esc(c.name)}</span></span>
    <span class="edit">${adjust(-1, "minus", "减少")}<input id="quota-field-${esc(key)}" class="field plan-input" type="number" inputmode="numeric" min="0" max="1440" step="1" value="${esc(minutes)}" data-change="quota-minutes" ${data} aria-label="${esc(c.name)}目标分钟数" />${adjust(1, "plus", "增加")}</span>
    <span class="hrs">${minutes === 0 ? "不排" : esc(meter(minutes * 60))}</span>
  </div>`;
}

function tierPlan(): string {
  const p = prefs();
  const plan = p.profiles[0];
  if (!plan) return `<div class="settings-panel"><p class="t-note">还没有项目。</p></div>`;
  const rows = p.categories.map((c) => quotaRow(c, plan)).join("");
  const total = profileTotalMinutes(plan);
  return `<div class="settings-panel plan">
    <div class="panel-meta"><span class="t-note">单位：分钟</span></div>
    <div class="plan-rows">${rows}</div>
    <div class="plan-total"><span class="engraved">总目标</span><span class="num">${esc(meter(total * 60))}</span></div>
  </div>`;
}

// ---------- 节奏 ----------

function rhythm(): string {
  const p = prefs();
  const idle = p.idle_reminder_enabled ? p.idle_reminder_minutes : 0;
  const uniform = p.uniform_block_minutes;
  return `<div class="settings-panel rows">
    ${settingRow("每格之后休息", "", select({ change: "break-default", value: p.break_minutes, options: withValue(BREAK_OPTIONS, p.break_minutes).map((n) => ({ value: n, label: n === 0 ? "不休息" : `${n} 分` })), width: 104, label: "每格之后休息" }))}
    ${settingRow(
      "统一时长",
      "设置默认时长；每次开始前仍可单独调整。",
      `${uniform > 0 ? select({ change: "uniform-length", value: uniform, options: withValue(BLOCK_OPTIONS, uniform).map((n) => ({ value: n, label: `${n} 分` })), width: 104, label: "统一时长" }) : ""}${toggle({ change: "uniform-toggle", checked: uniform > 0, label: "统一时长" })}`
    )}
    ${uniform === 0 ? p.categories.map((c) => settingRow(c.name, "默认专注时长", select({ change: "block-length", value: c.default_block_minutes, options: withValue(BLOCK_OPTIONS, c.default_block_minutes).map((n) => ({ value: n, label: `${n} 分` })), width: 104, label: `${c.name}默认专注时长`, data: { id: c.id } }))).join("") : ""}
    ${settingRow("暂停提醒", ui.platform?.mobile ? "未开格时定时提醒；本次暂停满 2 小时后不再提醒。" : "未开格时定时提醒。", select({ change: "idle", value: idle, options: withValue(IDLE_OPTIONS, idle).map((n) => ({ value: n, label: n === 0 ? "关闭" : `${n} 分` })), width: 104, label: "暂停提醒间隔" }))}
    ${hasFeature("autostart") ? settingRow("登录时自动启动", ui.autostart === null ? "正在读取系统设置…" : "", toggle({ change: "autostart", checked: ui.autostart === true, disabled: ui.autostart === null, label: "登录时自动启动" })) : ""}
  </div>`;
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
  if (!inTauri) return { title: "预览模式 · 扩展未连接", detail: "预览不拦截网页。请在桌面应用中连接扩展。", problem: false };
  if (b.error) return { title: "扩展连接需要处理", detail: b.error, problem: true };
  if (!b.available) return { title: "本地连接服务未启动", detail: "重启坐功后重试。", problem: true };
  if (!b.connected) return { title: "扩展未连接", detail: "首次连接请复制下方配对码，在 Chrome / Edge 的坐功扩展中保存配对。", problem: false };
  if (!b.synced) return { title: "扩展已连接 · 等待同步", detail: "规则待同步，可在扩展中立即同步。", problem: false };
  if (!prefs().blocked_urls.length) return { title: "扩展已连接 · 未配置精确网址", problem: false };
  return day()
    ? { title: "扩展已同步 · 精确网址生效中", detail: "仅在已连接的浏览器中拦截。", problem: false }
    : { title: "扩展已同步 · 下次学习日生效", problem: false };
}

function wholeSiteBrowserState(): BlockState {
  const b = ui.snap!.blocking.browser;
  if (!inTauri) return { title: "浏览器预览不执行整站拦截。", problem: false };
  if (!b.connected || b.error || !b.available) return { title: "浏览器未连接，请启用扩展并同步。", problem: true };
  if (!b.supports_hosts) return { title: "扩展需更新并重新加载，才能拦截整站。", problem: true };
  if (!b.synced) return { title: "规则待同步，请在扩展中立即同步。", problem: false };
  return { title: !prefs().blocked_hosts.length ? "已连接，添加规则后同步。" : day() ? "浏览器已同步 · 整站拦截生效中" : "浏览器已同步 · 下次学习日生效", problem: false };
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
    ? `<p class="blocking-conflict" role="status">整站规则 <b>${conflicts.map(esc).join("、")}</b> 也会屏蔽收藏和视频。只拦推荐页时，请先解除整站规则。</p>`
    : "";
  const problem = hostState.problem
    ? `<div class="problem-block"><span class="titles"><b>${esc(hostState.title)}</b>${hostState.detail ? `<span>${esc(hostState.detail)}</span>` : ""}</span>${btn("重新应用整站规则", { kind: "plate", cls: "caution", action: "reapply-blocking" })}</div>`
    : "";
  return `<div class="settings-panel website-block">
    <div class="panel-meta"><span class="t-note">${p.blocked_hosts.length + p.blocked_urls.length} / ${MAX_BLOCK_RULES} 条</span></div>
    ${warning}
    <div class="blocking-group-heading"><b>精确网址</b><span class="t-note">只拦这一个网址</span>${statusLabel(browserState)}</div>
    ${browserState.detail ? `<p class="blocking-note${browserState.problem ? " caution" : ""}">${esc(browserState.detail)}</p>` : ""}
    <div class="host-add">${btn("复制扩展配对码", { kind: "plate", action: "copy-browser-pairing", disabled: !inTauri || !ui.snap!.blocking.browser.available })}${btn("重新生成配对码", { kind: "quiet", action: "reset-browser-pairing", disabled: !inTauri || !ui.snap!.blocking.browser.available })}</div>
    <p class="blocking-note">在扩展弹窗中粘贴一次即可连接。重新生成会使已有浏览器配对失效，需要重新配对。</p>
    ${ruleEditor("url")}
    ${ruleRows("url", p.blocked_urls)}
    <details id="blocking-help" class="blocking-help" data-preserve-open><summary>${icon("chevron-right", 14)}<span>安装与用法</span></summary>
      <ol><li>打开 Chrome 的 <span class="mono sel">chrome://extensions</span> 或 Edge 的 <span class="mono sel">edge://extensions</span>，开启「开发者模式」。</li><li>点击下方按钮找到扩展目录，再在浏览器中选择「加载已解压的扩展程序」，选中该目录。</li><li>复制上方配对码，打开坐功扩展弹窗，粘贴后点击「保存配对并同步」。</li><li>保持坐功运行；扩展约每 30 秒尝试同步规则。上方显示「已同步」后，在学习日期间生效。</li></ol>
      ${hasFeature("browser_extension") ? btn("打开扩展文件夹", { kind: "plate", action: "reveal-browser-extension" }) : ""}
      <p class="blocking-note">抖音推荐页：<span class="mono sel">https://www.douyin.com/?recommend=1</span>。收藏页路径不同，可以正常打开。</p>
      <p class="blocking-note">B 站首页：<span class="mono sel">https://www.bilibili.com/</span>。视频页 <span class="mono sel">/video/…</span> 可以正常打开。</p>
      <p class="blocking-note">精确匹配逐字比较：路径、参数、顺序或 # 后内容不同都会放行。拦截发生在页面导航之后，可能一闪。</p>
    </details>
    <div class="blocking-group-heading"><b>整个网站</b><span class="t-note">这个域名下全都不开</span>${statusLabel(hostState)}</div>
    <p class="blocking-note${wholeSiteState.problem ? " caution" : ""}" role="status">${esc(wholeSiteState.title)}</p>
    ${ruleEditor("host")}
    ${ruleRows("host", p.blocked_hosts)}
    ${problem}
    <div class="foot-note"><p>暂停和休息时规则保持生效，收工后解除。写入和解除系统 hosts 各需一次管理员授权。</p>${btn("核对整站规则", { kind: "quiet", action: "recheck-blocking", disabled: ui.snap!.blocking.busy })}</div>
  </div>`;
}

// ---------- 应用屏蔽（手机） ----------

function appBlockPanel(): string {
  const b = appBlocking(prefs());
  const service = ui.systemStatus?.appBlockServiceEnabled ?? null;
  const open = (target: string, label: string) => btn(label, { kind: "plate", action: "system-settings", data: { target } });
  const switchDetail = !b.enabled
    ? "打开后，名单里的应用一打开就被送回坐功。跟学习日无关，随时手动开关。"
    : service === false ? "已打开，但无障碍服务没开，现在拦不住。"
    : b.apps.length ? "开着：打开名单里的应用会被送回坐功。关掉要两步确认。" : "开着，名单还是空的。";
  const rows = [
    settingRow("屏蔽名单里的应用", switchDetail, toggle({ change: "app-blocking", checked: b.enabled, label: "屏蔽名单里的应用", disabled: ui.pendingPrefs > 0 })),
    settingRow("无障碍服务", service === null ? (ui.systemStatusLoading ? "正在读取…" : "尚未读取")
      : service ? "已开启。坐功只读取前台应用的包名，不读屏幕内容。"
      : "未开启，屏蔽不会生效。坐功被划掉或强行停止后，系统会一起关掉这项服务；在无障碍设置里找到「坐功应用屏蔽」并重新打开。", service === false ? open("accessibility", "去开启") : ""),
  ];
  // Android 13 起，浏览器下载安装的应用默认不许开无障碍；系统会提示「受限设置」。
  if (service === false) rows.push(settingRow("开关是灰的", "系统提示「受限设置」时，到应用信息页点右上角 ⋮，选「允许受限制的设置」，再回来打开。", open("app_details", "应用信息"), "guidance"));
  // 屏蔽服务跟着坐功的进程走：系统不许坐功在后台运行时，划掉坐功就会被强行停止，服务也被关掉，
  // 要用户自己再开。坐功读得到「后台活动没被允许」这一状态，据实说明。
  const restricted = ui.systemStatus?.backgroundRestricted ?? null;
  const swipeDetail = restricted === false
    ? "已允许后台活动：从最近任务里划掉坐功只会移出列表，屏蔽照常生效。"
    : honorPhone()
      ? `${restricted ? "坐功的后台活动没有允许：" : "荣耀手机默认不许新应用在后台运行，"}从最近任务里划掉或一键清理时，系统会强行停止坐功，屏蔽服务随之关闭。到「应用启动管理」找到坐功，关闭自动管理，并把自启动、关联启动、后台活动三个开关都打开。`
      : `${restricted ? "坐功的后台活动被系统限制了：" : "部分手机上，"}从最近任务里划掉坐功时系统会强行停止它，屏蔽服务随之关闭。在应用信息的电池或后台设置里允许坐功在后台运行。`;
  rows.push(settingRow("划掉后也生效", swipeDetail,
    honorPhone() ? open("startup", "应用启动管理") : restricted === false ? "" : open("app_details", "应用信息"), "guidance"));
  const list = b.apps.length
    ? `<div class="blocking-rule-list">${b.apps.map((app) => `<div class="blocking-rule">
        <span class="rule-content"><span class="rule-value">${esc(app.label)}</span><span class="rule-kind mono">${esc(app.package_name)}</span></span>
        ${btn("移出", { kind: "quiet-danger", action: "remove-blocked-app", data: { package: app.package_name }, title: b.enabled ? `把${app.label}移出名单（需要两步确认）` : `把${app.label}移出名单` })}
      </div>`).join("")}</div>`
    : `<p class="blocking-note">名单是空的。点「添加应用」，从手机上的应用里选。</p>`;
  return `<div class="app-block-settings">
    <div class="settings-panel rows">${rows.join("")}</div>
    <div class="app-block-list">
      <div class="blocking-group-heading"><b>屏蔽名单</b><span class="t-note">${b.apps.length} / ${MAX_BLOCKED_APPS}</span>${btn("添加应用", { kind: "plate", action: "open-app-picker", icon: "plus", disabled: b.apps.length >= MAX_BLOCKED_APPS || ui.pendingPrefs > 0 })}</div>
      ${list}
    </div>
    <p class="t-note">桌面、设置和拨号永远不会被屏蔽。</p>
  </div>`;
}

// ---------- 身体 ----------

function bodyPanel(): string {
  const p = prefs();
  const water = `${hasFeature("in_app_sound_toggle") ? btn("试听", { kind: "quiet", action: "water-sound-test", disabled: !p.sound_enabled || !inTauri, title: inTauri ? "试听喝水提示音" : "请在桌面应用中试听" }) : ""}${toggle({ change: "water-on", checked: p.water_reminder_enabled, label: "喝水提醒" })}`;
  const waterDetail = ui.platform?.mobile
    ? "学习日进行中，按本地时钟在整点和半点提醒。暂停、休息时也提醒；本次暂停满 2 小时后停止，开下一格或继续这一格后恢复。错过不补发，提示音由系统通知设置管理。"
    : "按本地时钟，整点和半点提醒。暂停、休息时也提醒；休眠错过不补发。使用独立提示音。";
  const stretch = `${p.stretch_reminder_enabled ? stepper({ bind: "stretch-min", value: p.stretch_reminder_minutes, label: `${p.stretch_reminder_minutes} 分`, min: 15, max: 180, step: 5, ariaLabel: "起身提醒间隔" }) : ""}${toggle({ change: "stretch-on", checked: p.stretch_reminder_enabled, label: "起身护眼提醒" })}`;
  return `<div class="settings-panel rows">
    ${settingRow("喝水提醒", waterDetail, water)}
    ${settingRow("起身 / 护眼提醒", "", stretch)}
  </div>`;
}

// ---------- 提醒 ----------

function notificationStatusText(): string {
  switch (ui.notificationStatus) {
    case "granted":
      return "已允许";
    case "denied":
      return "未获系统授权，使用应用内提示。";
    case "unknown":
      return "尚未授权";
    default:
      return "正在检查…";
  }
}

function notificationPanel(): string {
  if (ui.platform?.mobile) return mobileNotificationPanel();
  const p = prefs();
  // 还没授权时才给「申请权限」——授权过之后这个按钮点了也没有反应，留着只会误导。
  const grant = ui.notificationStatus === "granted" || ui.notificationStatus === "checking"
    ? ""
    : btn("申请权限", { kind: "plate", action: "notif-recheck" });
  return `<div class="settings-panel rows">
    ${hasFeature("in_app_sound_toggle") ? settingRow("提示音", "提醒时播放声音。", toggle({ change: "sound", checked: p.sound_enabled, label: "提示音" })) : ""}
    ${settingRow("系统通知", notificationStatusText(), `${grant}${btn("试一条", { kind: "plate", action: "notif-test" })}${btn("打开系统设置", { kind: "quiet", action: "notif-open" })}`)}
  </div>`;
}

/** Android 的 IMPORTANCE_HIGH：到这一级才会从屏幕顶部弹出横幅。 */
const BANNER_IMPORTANCE = 4;
const ALERT_CHANNELS = new Set(["timer", "body", "water"]);

function mobileNotificationPanel(): string {
  const s = ui.systemStatus;
  const open = (target: string, channel?: string) => btn("去设置", { kind: "plate", action: "system-settings", data: { target, ...(channel ? { channel } : {}) }, disabled: !hasFeature("system_settings") });
  const names = { timer: "计时", body: "身体提醒", water: "喝水", status: "进行中" };
  const rows: string[] = [];
  // 只有需要处理的项才给「去设置」，正常的项只写一句状态，免得一页全是按钮。
  rows.push(settingRow("通知权限", !s ? "尚未读取" : s.notificationsEnabled ? "已允许。" : "未开启；锁屏后到点不会提醒。", s && !s.notificationsEnabled ? open("app_notifications") : ""));
  // 渠道以系统里的开关和重要性为准。荣耀等系统会把新应用的提醒渠道降一级，那样到点仍会响，
  // 但不会从屏幕顶部弹出横幅；「进行中」本来就是静默常驻，不算问题。
  const issues = s ? NOTIFICATION_CHANNELS.flatMap((id) => {
    const channel = s.channels.find((c) => c.id === id);
    if (!channel) return [{ id, detail: "尚未创建，重新打开应用后再检查。" }];
    if (!channel.enabled) return [{ id, detail: id === "status" ? "已关闭，通知栏不再显示进行中的倒计时。" : "已关闭，到点不会提醒。" }];
    if (ALERT_CHANNELS.has(id) && channel.importance < BANNER_IMPORTANCE) return [{ id, detail: "横幅未开启：到点会响铃，但不会从屏幕顶部弹出。打开「横幅通知」即可。" }];
    return [];
  }) : [];
  rows.push(settingRow("通知渠道", !s ? "尚未读取通知渠道状态。"
    : issues.length ? `${issues.length} 个通知渠道需要处理。`
    : "计时、身体提醒和喝水会弹出横幅并响铃；进行中的状态静默常驻在通知栏。", ""));
  for (const { id, detail } of issues) rows.push(settingRow(names[id], detail, open("channel", id)));
  // Android 13 起安装即获得精确闹钟权限；只有被关掉时才出现这一项。
  if (s && !s.canScheduleExactAlarms) rows.push(settingRow("精确闹钟", "未允许；锁屏后的提醒可能晚到。", open("exact_alarm")));
  rows.push(settingRow("电池优化", !s ? "尚未读取"
    : s.ignoringBatteryOptimizations ? "已设为不限制。"
    : "未设为不限制。到点提醒不受它影响；如果坐功在后台常被清理，可以改为不限制。", s && !s.ignoringBatteryOptimizations ? open("battery") : ""));
  const honor = honorPhone();
  const status = !s ? "" : s.backgroundRestricted
    ? "后台活动未允许：从最近任务里划掉坐功时，系统会强行停止它，之后的提醒都不会响。"
    : "已允许后台活动。";
  const guidance = honor
    ? `${status}点「去设置」到「应用启动管理」，找到坐功，关闭自动管理，并把自启动、关联启动、后台活动三个开关都打开。最近任务里下拉坐功卡片并锁定，避免一键清理。`
    : `${status}在系统的应用或电池设置中允许坐功后台活动。可在最近任务中锁定坐功，避免一键清理。菜单名称以手机实际显示为准。`;
  rows.push(settingRow("后台运行", guidance, open(honor ? "startup" : "app_details"), "background-guidance"));
  return `<div class="reminder-settings">
    <div class="reminder-check"><p role="status">${ui.systemStatusLoading ? "正在读取系统状态…" : ui.systemStatusError ? "暂时无法读取提醒状态，请重新检查。" : "从系统设置返回后，会自动重新检查。"}</p>${btn("重新检查", { kind: "quiet", action: "system-recheck", disabled: ui.systemStatusLoading })}</div>
    <div class="settings-panel rows">${rows.join("")}</div>
    <div class="reminder-tests">${btn("发测试通知", { kind: "plate", action: "mobile-notif-test" })}${btn("试听喝水提醒", { kind: "plate", action: "mobile-water-test" })}</div>
    <p class="t-note">提示音与振动由系统通知设置管理。</p>
  </div>`;
}

// ---------- 关于 ----------

function aboutPanel(): string {
  return `<div class="settings-panel rows">
    ${settingRow("版本", "", `<span class="mono t-note">${esc(ui.appVersion ?? "读取中…")}</span>`)}
    ${settingRow("数据只存在本机", hasFeature("reveal_state_file") ? ui.snap!.state_path : "", hasFeature("reveal_state_file") ? btn("在 Finder 中显示", { kind: "quiet", action: "reveal-state" }) : "")}
  </div>`;
}
