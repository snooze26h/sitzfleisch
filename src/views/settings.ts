// 「设置」：分区导航、项目目录与配额矩阵，改动自动保存。
// 每个控件改完立即生效，没有「保存」按钮；文字框在失焦或回车时提交。

import type { CategoryDef, ProfileDef } from "../types";
import { ICON_NAMED, ROLES, roleLabel } from "../types";
import { esc, meter, shortNameFrom } from "../format";
import { icon } from "../icons";
import { btn, hair, labelled, plate, sectionLabel, select, stepper, toggle } from "../components";
import { BLOCK_OPTIONS, BREAK_OPTIONS, IDLE_OPTIONS, day, defaultProfileId, normalizeHost, prefs, profileTotalMinutes, ui, withValue } from "../state";

export function settingsPage(): string {
  const panels = [
    ["projects", "项目", projectCatalog()],
    ["tiers", "三档安排", tierPlan()],
    ["rhythm", "节奏", rhythm()],
    ["hosts", "网站屏蔽", websiteBlock()],
    ["body", "身体", bodyPanel()],
    ["notify", "提醒", notificationPanel()],
    ["about", "关于", aboutPanel()],
  ];
  const navigation = panels.map(([id, title]) => `<button data-action="jump-settings" data-id="${esc(id)}">${esc(title)}</button>`).join("");
  return `<header class="page-heading"><h1>设置</h1><p role="status">${ui.pendingPrefs > 0 ? "正在保存…" : "更改自动保存"}</p></header>
    <nav class="settings-nav" aria-label="设置分区">${navigation}</nav>` + panels
    .map(([id, title, html]) => html.replace('<section class="plate"', `<section class="plate settings-section" id="panel-${id}" tabindex="-1" aria-label="${esc(title)}"`))
    .join("");
}

function settingRow(title: string, detail: string, control: string): string {
  return `<div class="setting-row"><span class="titles"><b>${esc(title)}</b>${detail ? `<span>${esc(detail)}</span>` : ""}</span><span class="ctl">${control}</span></div>`;
}

// ---------- 项目 ----------

function projectCatalog(): string {
  const p = prefs();
  const rows = p.categories.map(projectRow).join(hair());
  const inner = `<div class="settings-panel">
    ${sectionLabel("项目", { icon: "layers", trailing: btn("添加项目", { kind: "plate", action: "add-project" }), cls: "pb6" })}
    ${rows}
  </div>`;
  return plate(inner);
}

function projectRow(c: CategoryDef): string {
  const open = ui.expandedProject === c.id;
  const meta = `一格 ${c.default_block_minutes} 分`;
  const header = `<button class="project-row" id="project-${esc(c.id)}" data-action="expand-project" data-id="${esc(c.id)}" aria-expanded="${open}" aria-label="${open ? "收起" : "编辑"}项目 ${esc(c.name)}">
      ${icon(open ? "chevron-down" : "chevron-right", 11, "chev")}
      <span class="glyph-badge">${icon(c.icon, 15)}</span>
      <span class="nm">${esc(c.name)}</span>
      <span class="role">${esc(roleLabel(c.role))}</span>
      <span class="hints">${esc(meta)}</span>
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
  return `<div class="project-editor" id="editor-${esc(c.id)}">
    <div class="identity">
      ${labelled("名称", `<input class="field sm" style="width:150px" data-change="project-name" data-id="${esc(c.id)}" value="${esc(c.name)}" placeholder="项目名称" aria-label="编辑项目名称 ${esc(c.name)}" />`)}
      ${labelled("短名", `<input class="field sm" style="width:84px" data-change="project-short-name" data-id="${esc(c.id)}" value="${esc(c.short_name)}" placeholder="${esc(shortNameFrom(c.name))}" aria-label="${esc(c.name)}在侧栏与菜单栏上的短名，留空自动" />`)}
      ${labelled("一格", select({ change: "block-length", data: { id: c.id }, value: c.default_block_minutes, options: withValue(BLOCK_OPTIONS, c.default_block_minutes).map((n) => ({ value: n, label: `${n} 分` })), width: 84, label: `${c.name}默认一格多长`, disabled: prefs().uniform_block_minutes > 0 }))}
      ${labelled("排序", select({ change: "project-role", data: { id: c.id }, value: c.role, options: ROLES.map(([value, label]) => ({ value, label })), width: 108, label: `${c.name}在下一格建议里的性质` }))}
    </div>
    <div class="s-field"><span class="engraved">图标</span><div class="glyphs">${glyphs}</div></div>
    <div class="foot">${btn("删除项目", { kind: "quiet-danger", action: "del-project", data: { id: c.id } })}</div>
  </div>`;
}

// ---------- 三档安排 ----------

function quotaCell(c: CategoryDef, profile: ProfileDef, scaleMinutes: number): string {
  const minutes = profile.quotas.find((q) => q.category === c.id)?.minutes ?? 0;
  const key = `${encodeURIComponent(profile.id)}:${encodeURIComponent(c.id)}`;
  const label = `${profile.name}档 ${c.name}`;
  const data = `data-profile="${esc(profile.id)}" data-category="${esc(c.id)}"`;
  const adjust = (delta: number, glyph: string, verb: string) => `<button id="quota-${glyph}-${esc(key)}" data-action="step" data-bind="quota" data-delta="${delta}" data-step="15" data-min="0" data-max="1440" ${data} ${delta < 0 ? minutes <= 0 ? "disabled" : "" : minutes >= 1440 ? "disabled" : ""} aria-label="${esc(label)}${verb} 15 分钟">${icon(glyph, 12)}</button>`;
  // 表里填的是分钟，脑子里想的是小时；换算放在格子下面，输入本身仍然是分钟。
  // 短条是**整张表同一把尺**（分母是表里最大的那个数），所以横竖都能直接比长短。
  const hours = minutes === 0 ? "—" : meter(minutes * 60);
  const share = scaleMinutes > 0 ? Math.min(100, (minutes / scaleMinutes) * 100) : 0;
  return `<td><div class="quota-edit${minutes === 0 ? " zero" : ""}">
    ${adjust(-1, "minus", "减少")}
    <input id="quota-field-${esc(key)}" class="field quota-input" type="number" inputmode="numeric" min="0" max="1440" step="1" value="${esc(minutes)}" data-change="quota-minutes" ${data} aria-label="${esc(label)}目标分钟数" aria-describedby="quota-help" />
    ${adjust(1, "plus", "增加")}
  </div>
  <div class="quota-scale" aria-hidden="true"><span class="bar"><i style="width:${share.toFixed(2)}%"></i></span><span class="hrs">${esc(hours)}</span></div></td>`;
}

function tierPlan(): string {
  const p = prefs();
  const columns = p.profiles.map((profile) => `<th scope="col">${esc(profile.name)}</th>`).join("");
  // 一把尺量整张表：分母是所有格里最大的那个，空表兜个 1 免得除零。
  const scaleMinutes = Math.max(1, ...p.profiles.flatMap((x) => x.quotas.map((q) => q.minutes)));
  const rows = p.categories.map((c) => `<tr id="quota-row-${esc(c.id)}"><th scope="row"><span class="quota-project">${icon(c.icon, 15)}<span>${esc(c.name)}</span></span></th>${p.profiles.map((profile) => quotaCell(c, profile, scaleMinutes)).join("")}</tr>`).join("");
  const totals = p.profiles
    .map((profile) => {
      const total = profileTotalMinutes(profile);
      return `<td><span class="quota-total">${esc(meter(total * 60))}</span><span class="quota-total-minutes" aria-hidden="true">${esc(total)} 分</span></td>`;
    })
    .join("");
  const inner = `<div class="settings-panel">
    ${sectionLabel("三档安排", { icon: "sliders-horizontal", trailing: `<span class="t-note">单位：分钟</span>`, cls: "pb12" })}
    <div class="quota-table-wrap"><table class="quota-table" aria-label="各项目在三档安排中的目标分钟数" aria-describedby="quota-help">
      <thead><tr><th scope="col">项目</th>${columns}</tr></thead>
      <tbody>${rows}</tbody>
      <tfoot><tr><th scope="row">总目标</th>${totals}</tr></tfoot>
    </table></div>
    <p class="quota-help" id="quota-help">直接输入分钟数，或每次增减 15 分钟；0 表示不安排。</p>
    <p class="quota-help">用于未来学习日；今天不会自动改变，主动切换档位时才采用新配额。</p>
  </div>`;
  return plate(inner);
}

// ---------- 节奏 ----------

function rhythm(): string {
  const p = prefs();
  const active = !!day();
  const idle = p.idle_reminder_enabled ? p.idle_reminder_minutes : 0;
  const uniform = p.uniform_block_minutes;
  const inner = `<div class="settings-panel">
    ${sectionLabel("节奏", { icon: "timer" })}
    ${settingRow("默认档位", active ? "今天进行中，去今天页切换" : "", select({ change: "default-profile", value: defaultProfileId(), options: p.profiles.map((x) => ({ value: x.id, label: `${x.name} · ${meter(profileTotalMinutes(x) * 60)}` })), width: 134, label: "默认档位", disabled: active }))}
    ${hair()}
    ${settingRow("每格之后休息", "", select({ change: "break-default", value: p.break_minutes, options: withValue(BREAK_OPTIONS, p.break_minutes).map((n) => ({ value: n, label: n === 0 ? "不休息" : `${n} 分` })), width: 104, label: "每格之后休息" }))}
    ${hair()}
    ${settingRow(
      "统一块长",
      uniform > 0 ? "所有项目都用这个长度，项目里的「一格」暂时不生效" : "按每个项目各自的「一格」",
      `${uniform > 0 ? select({ change: "uniform-length", value: uniform, options: withValue(BLOCK_OPTIONS, uniform).map((n) => ({ value: n, label: `${n} 分` })), width: 84, label: "统一块长" }) : ""}${toggle({ change: "uniform-toggle", checked: uniform > 0, label: "统一块长" })}`
    )}
    ${hair()}
    ${settingRow("闲置多久开始催", "", select({ change: "idle", value: idle, options: withValue(IDLE_OPTIONS, idle).map((n) => ({ value: n, label: n === 0 ? "不催" : `${n} 分` })), width: 104, label: "闲置提醒间隔" }))}
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
  hostLabel: string;
}

export function blockState(): BlockState {
  const b = ui.snap!.blocking;
  const hosts = prefs().blocked_hosts.length;
  const active = !!day();
  if (b.busy) return { title: "正在核对系统规则", problem: false, hostLabel: "正在核对系统规则" };
  if (b.error) return { title: "需要处理", detail: b.error, problem: true, hostLabel: "当前规则尚未确认" };
  if (active) {
    if (hosts === 0) return { title: "学习日进行中 · 未配置", problem: false, hostLabel: "当前学习日正在屏蔽" };
    if (b.active) return { title: `正在屏蔽 ${hosts} 个`, problem: false, hostLabel: "当前学习日正在屏蔽" };
    return { title: "系统规则与设置不一致", detail: `应该屏蔽 ${hosts} 个，实际检测到 0 个。`, problem: true, hostLabel: "当前规则尚未确认" };
  }
  return { title: hosts === 0 ? "还没有屏蔽网址" : `已配置 ${hosts} 个`, problem: false, hostLabel: "下次学习日生效" };
}

function websiteBlock(): string {
  const p = prefs();
  const state = blockState();
  const preview = ui.hostDraft.trim() ? normalizeHost(ui.hostDraft) : null;
  const previewLine = preview
    ? "host" in preview
      ? `<p class="t-caption sel" style="font-size:12px;padding-top:8px">将保存为 ${esc(preview.host)}，会挡住 ${esc(preview.host)} 和 www.${esc(preview.host)}。</p>`
      : `<p class="t-note caution" style="padding-top:8px">${esc(preview.error)}</p>`
    : "";
  const rows = p.blocked_hosts.length
    ? `<div class="host-list">${p.blocked_hosts
        .map(
          (h) => `<div class="host-chip"><span class="mono">${esc(h)}</span>${btn("解除", { kind: "quiet-danger", action: "remove-host", data: { host: h }, title: `解除对 ${h} 的屏蔽（需要两步确认）` })}</div>`
        )
        .join("")}</div>`
    : `<p class="t-note" style="font-size:12.5px;padding:4px 0">列表是空的。</p>`;
  const problem = state.problem
    ? `<div class="problem-block"><span class="titles"><b>${esc(state.title)}</b>${state.detail ? `<span>${esc(state.detail)}</span>` : ""}</span>${btn("重新核对", { kind: "quiet", action: "recheck-blocking" })}${btn("重新应用", { kind: "plate", cls: "caution", action: "reapply-blocking" })}</div>`
    : "";
  const statusLabel = `<span class="status-dot${state.problem ? " problem" : ""}"><i></i>${esc(state.title)}</span>`;
  const inner = `<div class="settings-panel">
    ${sectionLabel("网站屏蔽", { icon: "shield", trailing: statusLabel, cls: "pb12" })}
    <div class="host-add"><input class="field md" style="flex:1" data-input="host" value="${esc(ui.hostDraft)}" aria-label="要屏蔽的网址或域名" placeholder="粘贴网址或输入域名" autocomplete="off" spellcheck="false" />${btn("添加", { kind: "plate", action: "add-host", disabled: !(preview && "host" in preview) })}</div>
    ${previewLine}
    ${hair("mt13")}
    ${rows}
    ${problem ? hair("mt13") + problem : ""}
    <div class="foot-note"><p>学习日开始时写入系统 hosts，收工或放弃时解除；写入需要一次管理员授权。</p>${state.problem ? "" : btn("重新核对", { kind: "quiet", action: "recheck-blocking" })}</div>
  </div>`;
  return plate(inner);
}

// ---------- 身体 ----------

function bodyPanel(): string {
  const p = prefs();
  const water = `${p.water_reminder_enabled ? stepper({ bind: "water-min", value: p.water_reminder_minutes, label: `${p.water_reminder_minutes} 分`, min: 10, max: 180, step: 5, ariaLabel: "喝水提醒间隔" }) : ""}${toggle({ change: "water-on", checked: p.water_reminder_enabled, label: "喝水提醒" })}`;
  const stretch = `${p.stretch_reminder_enabled ? stepper({ bind: "stretch-min", value: p.stretch_reminder_minutes, label: `${p.stretch_reminder_minutes} 分`, min: 15, max: 180, step: 5, ariaLabel: "起身提醒间隔" }) : ""}${toggle({ change: "stretch-on", checked: p.stretch_reminder_enabled, label: "起身护眼提醒" })}`;
  const inner = `<div class="settings-panel">
    ${sectionLabel("身体", { icon: "heart-pulse" })}
    ${settingRow("喝水提醒", "", water)}
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
    ${sectionLabel("提醒", { icon: "bell" })}
    ${settingRow("提示音", "提醒时响一声，窗口关着也听得见", toggle({ change: "sound", checked: p.sound_enabled, label: "提示音" }))}
    ${hair()}
    ${settingRow("系统通知", notificationStatusText(), `${grant}${btn("试一条", { kind: "plate", action: "notif-test" })}${btn("打开系统设置", { kind: "quiet", action: "notif-open" })}`)}
  </div>`;
  return plate(inner);
}

// ---------- 关于 ----------

function aboutPanel(): string {
  const inner = `<div class="settings-panel">
    ${sectionLabel("关于", { icon: "info" })}
    ${settingRow("版本", "坐功 · Sitzfleisch", `<span class="mono t-note">${esc(ui.appVersion ?? "读取中…")}</span>`)}
    ${hair()}
    ${settingRow("数据只存在本机", ui.snap!.state_path, btn("在 Finder 中显示", { kind: "quiet", action: "reveal-state" }))}
  </div>`;
  return plate(inner);
}
