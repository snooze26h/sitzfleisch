// 侧栏：字标、三个入口，以及学习日进行中时压在底部的一轮小月与一行实时状态。

import { clock, esc, meter } from "../format";
import { icon } from "../icons";
import { brandIcon, brandMark } from "../brand";
import { dayMoon } from "../moon";
import { suggest } from "../scheduler";
import type { View } from "../types";
import { settingsSections } from "./settings";
import { breakRemaining, day, iconOf, isPaused, nameOf, netSeconds, pauseNowSeconds, prefs, quotaSeconds, resting, shortName, ui } from "../state";

const NAV: [View, string][] = [
  ["today", "今天"],
  ["history", "历史"],
  ["settings", "设置"],
];

export function sidebar(): string {
  // 进了设置，侧栏整个换成设置的分区：一区一页，不再一根滚动条到底。
  const inSettings = ui.view === "settings";
  const nav = inSettings
    ? `<button class="nav-item back" data-action="tab" data-view="today" aria-label="离开设置，回到今天">${icon("chevron-left", 16)}<span>返回</span></button>`
      + `<div class="nav-sep"></div>`
      + settingsSections().map(
          ([id, label, ic]) =>
            `<button class="nav-item${ui.settingsSection === id ? " on" : ""}" data-action="jump-settings" data-id="${id}" aria-current="${ui.settingsSection === id ? "page" : "false"}"><i class="rail"></i>${icon(ic, 16)}<span>${label}</span></button>`
        ).join("")
    : NAV.map(
        ([view, label]) =>
          `<button class="nav-item${ui.view === view ? " on" : ""}" data-action="tab" data-view="${view}" aria-current="${ui.view === view ? "page" : "false"}"><i class="rail"></i>${brandIcon(view)}<span>${label}</span></button>`
      ).join("");
  return `<aside class="sidebar" id="sidebar">
    <div class="wordmark"><span class="wordmark-symbol">${brandMark()}</span><span class="wordmark-type"><span class="wordmark-cn">坐功</span><span class="wordmark-en">SITZFLEISCH</span></span></div>
    <nav class="nav" aria-label="${inSettings ? "设置分区" : "主导航"}">${nav}</nav>
    ${inSettings ? "" : nowBlock()}
  </aside>`;
}

/** 侧栏底部：今天这一轮月的缩影、正在做什么、还剩多久，以及今天学了多久。没开始学习日时不出现。 */
function nowBlock(): string {
  const d = day();
  if (!d) return "";
  let ic: string;
  let tint = "muted";
  let title: string;
  let value: string;
  // 「已暂停」这类刻字标签；只有格被按停时才有。
  let tag = "";
  // 大读数平时是骨白，只有暂停秒表走赭石——它是「时间在流走但没算数」的那个。
  let valueTint = "";
  const t = d.timer;
  const held = !!t && isPaused(d);
  if (resting(d)) {
    ic = "coffee";
    tint = "caution";
    title = "休息";
    value = clock(breakRemaining(d));
  } else if (t && held) {
    // 只有**真的有一格被按停**才是「已暂停」。没有格在走时整天也算暂停中
    // （0.8.0 的规矩），那种时候该报「下一格」，不是把暂停秒表挂在这儿。
    ic = "pause";
    tint = "caution";
    title = nameOf(t.category, d);
    tag = `<em class="tag engraved caution">已暂停</em>`;
    value = clock(pauseNowSeconds(d));
    valueTint = " caution";
  } else if (t) {
    ic = iconOf(t.category);
    tint = "high";
    title = nameOf(t.category, d);
    value = clock(Math.max(0, t.total_seconds - t.elapsed_seconds));
  } else {
    const s = suggest(d, prefs(), ui.now);
    ic = s ? iconOf(s.category) : "check";
    tint = s ? "muted" : "stroke";
    title = s ? "下一格" : "今日达成";
    value = s ? shortName(s.category, d) : "";
  }
  const burning = !!t && !held && !resting(d);
  return `<div class="now${burning ? " burning" : ""}" id="now">
    <div class="now-main">
      <span class="now-moon">${dayMoon(d, "mini")}</span>
      <span class="now-text">
        <span class="now-line">${icon(ic, 14, tint)}<span class="nm">${esc(title)}</span>${tag}</span>
        <span class="val${valueTint}">${esc(value)}</span>
      </span>
    </div>
    <div class="now-net"><span class="engraved">已学</span><span class="val">${esc(meter(netSeconds(d)))} <i>/ ${esc(meter(quotaSeconds(d)))}</i></span></div>
  </div>`;
}
