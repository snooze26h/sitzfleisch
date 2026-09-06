// 侧栏：字标、三个入口，以及学习日进行中时导航下方的一行实时状态。

import { clock, esc, meter } from "../format";
import { icon } from "../icons";
import { hair, tickedTrack } from "../components";
import { suggest } from "../scheduler";
import type { View } from "../types";
import { breakRemaining, day, iconOf, isPaused, nameOf, netSeconds, pauseNowSeconds, prefs, quotaSeconds, resting, shortName, ui } from "../state";

const NAV: [View, string, string][] = [
  ["today", "timer", "今天"],
  ["history", "calendar-days", "历史"],
  ["settings", "sliders-horizontal", "设置"],
];

export function sidebar(): string {
  const nav = NAV.map(
    ([view, ic, label]) =>
      `<button class="nav-item${ui.view === view ? " on" : ""}" data-action="tab" data-view="${view}" aria-current="${ui.view === view ? "page" : "false"}"><i class="rail"></i>${icon(ic, 14)}<span>${label}</span></button>`
  ).join("");
  return `<aside class="sidebar" id="sidebar">
    <div class="wordmark"><span class="wordmark-en">SITZFLEISCH</span><span class="wordmark-cn">坐功</span></div>
    ${hair()}
    <nav class="nav">${nav}</nav>
    ${nowBlock()}
  </aside>`;
}

/** 导航下方的一行：正在做什么、还剩多久，以及今天学了多久。没开始学习日时不出现。 */
function nowBlock(): string {
  const d = day();
  if (!d) return "";
  let ic: string;
  let tint = "muted";
  let title: string;
  let value: string;
  let track = "";
  // 「已暂停」这类刻字标签；只有格被按停时才有。
  let tag = "";
  // 大读数平时是骨白，只有暂停秒表走赭石——它是「时间在流走但没算数」的那个。
  let valueTint = "";
  const t = d.timer;
  if (resting(d)) {
    ic = "coffee";
    title = "休息";
    value = clock(breakRemaining(d));
  } else if (t && isPaused(d)) {
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
    // 不上朱红：DESIGN 点名的三处是「面板头部的字形 + 运行图上那一小段块」「久坐提示」
    // 「当前时刻的旗标」，侧栏这个字形不在名单里。「真的在跑」由倒计时和下面那条轨承担。
    tint = "high";
    title = nameOf(t.category, d);
    value = clock(Math.max(0, t.total_seconds - t.elapsed_seconds));
    // 进度条走骨白：全 App 的朱红只留给「正在跑的那一格」的图标、久坐提醒和运行图上的此刻。
    track = tickedTrack(t.total_seconds > 0 ? t.elapsed_seconds / t.total_seconds : 0, Math.max(1, Math.round(t.total_seconds / 900)), { mini: true });
  } else {
    const s = suggest(d, prefs(), ui.now);
    ic = s ? iconOf(s.category) : "check";
    tint = s ? "muted" : "stroke";
    title = s ? "下一格" : "今日达成";
    value = s ? shortName(s.category, d) : "";
  }
  return `<div class="now" id="now">
    ${hair()}
    <div class="now-line">${icon(ic, 13, tint)}<span class="nm">${esc(title)}</span>${tag}<span class="val${valueTint}">${esc(value)}</span></div>
    ${track}
    <div class="now-net"><span class="engraved">已学</span><span class="val">${esc(meter(netSeconds(d)))} <i>/ ${esc(meter(quotaSeconds(d)))}</i></span></div>
  </div>`;
}
