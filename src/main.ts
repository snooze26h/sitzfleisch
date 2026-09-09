// 主循环：快照进来 → 拼 HTML → morphdom 打补丁 → 画运行图。所有交互走事件委托。

import morphdom from "morphdom";
import { invoke, onQuitBlocked, onReminder, onSnapshot, setWindowTitle } from "./api";
import { conflictingHost, MAX_BLOCK_RULES, normalizeHost, normalizeUrl } from "./blocking";
import type { Preferences, Snapshot, View } from "./types";
import { SHORT_NAME_WIDTH, dayLabel, displayWidth, duration, esc, nowUnix } from "./format";
import {
  MAX_PROJECTS,
  QUIT_DIALOG_ID,
  day,
  dialogIsBusy,
  history,
  isPaused,
  nameOf,
  parseTasks,
  prefs,
  remainingOf,
  topOverlay,
  ui,
  type Dialog,
  type DialogOutcome,
} from "./state";
import { btn } from "./components";
import { blockMinutes, suggest } from "./scheduler";
import { mergeSnapshot } from "./snapshots";
import { markdownForDay } from "./markdown";
import { sidebar } from "./views/shell";
import { todayPage } from "./views/today";
import { historyEmpty, historyPage } from "./views/history";
import { blockState, settingsPage } from "./views/settings";
import { overlays, toastView } from "./views/overlays";
import { drawDiagram } from "./views/diagram";

const app = document.getElementById("app")!;
const TITLES: Record<View, string> = { today: "今天", history: "历史", settings: "设置" };
let toastTimer = 0;
let pulseTimer = 0;
let prevSnap: Snapshot | null = null;
let historyRevision = -1;
let lastView: View | null = null;
let preferencesQueue: Promise<void> = Promise.resolve();
let dialogToken = 0;
/** 把框叫出来的那个元素；关框之后焦点还给它。 */
let dialogOpener: HTMLElement | null = null;

// ---------- 渲染 ----------

function pageHtml(): string {
  // id 让 morphdom 按键匹配：提示条、对话框出现或消失时，页面本体绝不会被整棵重建。
  switch (ui.view) {
    case "today":
      return `<div class="page" id="page-today">${todayPage()}</div>`;
    case "history":
      return `<div class="page${historyEmpty() ? " fill" : ""}" id="page-history">${historyPage()}</div>`;
    case "settings":
      return `<div class="page narrow" id="page-settings">${settingsPage()}</div>`;
  }
}

function reconcileSelection() {
  const d = day();
  if (!d || d.timer) return;
  if (ui.selectedCategory && d.categories.some((c) => c.id === ui.selectedCategory)) return;
  ui.selectedCategory = suggest(d, prefs(), ui.now)?.category ?? d.categories[0]?.id ?? null;
  ui.minutesDraft = null;
}

function render() {
  if (!ui.snap) return;
  ui.now = nowUnix();
  reconcileSelection();
  // 状态文件损坏或来自更高版本时 App 不落盘：这件事必须在每一页都看得见。
  const protect = ui.snap.write_protected
    ? `<div class="protect-banner" id="protect"><b>状态文件处于保护模式，本次运行不会保存任何改动。</b><span>${esc(ui.snap.write_protected)} 应用不会覆盖原文件：${esc(ui.snap.state_path)}</span></div>`
    : "";
  // 保护模式本来就不写盘，那不叫保存失败；只有真的写不进去才报这一条。
  const saveFailed = ui.snap.save_error && !ui.snap.write_protected
    ? `<div class="protect-banner save-banner" id="save-banner"><b>上次保存失败：${esc(ui.snap.save_error)}。改动还在内存里，先别退出。</b>${btn("立即重试", { kind: "plate", action: "retry-save" })}</div>`
    : "";
  const html = `<div id="app"><div class="shell" id="shell">${sidebar()}<main class="content" id="content" tabindex="-1" data-scroll>${protect}${saveFailed}${pageHtml()}${toastView()}</main></div>${overlays()}</div>`;
  morphdom(app, html, {
    onBeforeElUpdated(from, to) {
      // 正在编辑的控件不动，免得打断输入；它的其它属性会在失焦后的下一次渲染补上。
      if (from === document.activeElement && (from instanceof HTMLInputElement || from instanceof HTMLTextAreaElement || from instanceof HTMLSelectElement)) {
        // 保留光标时仍同步校验状态，让读屏与输入框下方的即时错误一致。
        if (to.hasAttribute("aria-invalid")) from.setAttribute("aria-invalid", to.getAttribute("aria-invalid")!);
        return false;
      }
      // 用户输入只改 DOM 属性，HTML 仍可能相等；保存被拒时也要恢复权威值。
      if (from instanceof HTMLInputElement && to instanceof HTMLInputElement && (from.value !== to.value || from.checked !== to.checked)) return true;
      // 说明折叠由浏览器原生交互控制，心跳重绘不能删掉用户刚打开的 open。
      // 只保留展开属性，内容变化仍交给 morphdom 更新。
      if (from instanceof HTMLDetailsElement && to instanceof HTMLDetailsElement && from.hasAttribute("data-preserve-open") && to.hasAttribute("data-preserve-open")) {
        to.open = from.open;
      }
      if (from.isEqualNode(to) && !from.querySelector("input, textarea, select")) return false;
      return true;
    },
  });
  if (lastView !== ui.view) {
    lastView = ui.view;
    const main = app.querySelector<HTMLElement>("[data-scroll]");
    if (main) main.scrollTop = 0;
  }
  drawDiagrams();
  void setWindowTitle(TITLES[ui.view]);
}

function drawDiagrams() {
  const d = day();
  if (!d) return;
  for (const canvas of app.querySelectorAll<HTMLCanvasElement>("canvas[data-diagram]")) {
    drawDiagram(canvas, d, ui.now, null, (cid) => nameOf(cid, d));
  }
}

new ResizeObserver(() => requestAnimationFrame(drawDiagrams)).observe(app);
// 字体装载完成前 canvas 会用回退字体画刻度，装好后重画一次。
void document.fonts.ready.then(() => drawDiagrams());

// ---------- 快照 ----------

function detectTransitions(prev: Snapshot | null, next: Snapshot) {
  if (!prev) return;
  const beforeDay = prev.state.day;
  const after = next.state.day;
  if (!beforeDay || !after) return;
  // 心跳空档让计时按停了：说清楚空档没有计入，不推断系统原因。
  if (after.suspend_seconds > beforeDay.suspend_seconds && isPaused(after)) {
    toast("检测到计时中断，已自动暂停，空档没有计入。");
  }
  const before = beforeDay.timer;
  if (!before) return;
  const finished = after.ledger.length > beforeDay.ledger.length;
  const naturally = before.total_seconds - before.elapsed_seconds <= 1;
  // 自然走完的格：亮一下配额条，并报一句还差多少。手动结束的由按钮那边报。
  if (finished && naturally) {
    const entry = after.ledger[after.ledger.length - 1];
    if (entry?.accepted) {
      pulseQuota(entry.category);
      creditToast(after, entry.category, entry.seconds);
    }
  }
}

function pulseQuota(categoryId: string) {
  ui.creditPulse = categoryId;
  window.clearTimeout(pulseTimer);
  pulseTimer = window.setTimeout(() => {
    ui.creditPulse = null;
    render();
  }, 1300);
}

function creditToast(after: NonNullable<Snapshot["state"]["day"]>, categoryId: string, seconds: number) {
  const cat = after.categories.find((c) => c.id === categoryId);
  const remaining = cat ? remainingOf(cat) : 0;
  const name = nameOf(categoryId, after);
  toast(remaining <= 0 ? `${name}今天的配额满了。` : `记下 ${duration(seconds)}。${name}还差 ${duration(remaining)}。`);
}

function applySnapshot(snap: Snapshot) {
  const merged = mergeSnapshot(ui.snap, ui.history, historyRevision, snap);
  historyRevision = merged.historyRevision;
  if (merged.snapshot === ui.snap && merged.history === ui.history) return;
  if (merged.snapshot !== ui.snap) {
    detectTransitions(prevSnap, merged.snapshot);
    prevSnap = merged.snapshot;
    ui.snap = merged.snapshot;
  }
  // 较新的心跳不带历史；较早命令的归档结果仍可能是最新的一份历史。
  ui.history = merged.history;
  if (ui.selectedHistoryDay !== null && !history().some((entry) => entry.day.started_at === ui.selectedHistoryDay)) ui.selectedHistoryDay = null;
  render();
}

async function act(command: string, args: Record<string, unknown> = {}): Promise<Snapshot | null> {
  try {
    const snap = await invoke<Snapshot>(command, args);
    applySnapshot(snap);
    return snap;
  } catch (error) {
    toast(String(error));
    return null;
  }
}

function toast(message: string, ms = 6000) {
  ui.toast = { message };
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    ui.toast = null;
    render();
  }, ms);
  render();
}

function ask(dialog: Omit<Dialog, "token">) {
  ui.menu = null;
  const current = ui.dialog;
  // 同一个 id 再来一次（比如又按了一次退出）：只换正文，不叠新框，也不打断正在进行的那一下。
  if (current && dialog.id && current.id === dialog.id) {
    current.message = dialog.message;
    render();
    return;
  }
  // 记下是谁把框叫出来的：关掉之后焦点得还回去，不能把用户扔回文档开头。
  // 已经有框在的时候不覆盖——那一份记的才是最外层的来路。
  if (!current && document.activeElement instanceof HTMLElement) dialogOpener = document.activeElement;
  ui.dialog = {
    ...dialog,
    // 破坏性的框停在取消键上：回车不该顺手把东西删了。其余停在确认键。
    focus: dialog.focus ?? (dialog.destructive ? "cancel" : "confirm"),
    token: ++dialogToken,
  };
  render();
  focusDialog(ui.dialog);
}

/** 打开或恢复可点状态之后，把焦点放回它自己指定的那个键。 */
function focusDialog(dialog: Dialog) {
  if (!dialog.focus) return;
  const action = dialog.focus === "confirm" ? "dialog-confirm" : "dialog-cancel";
  app.querySelector<HTMLElement>(`.dialog [data-action="${action}"]`)?.focus();
}

/** 框里可以停焦点的东西，按文档顺序。Tab 就在这一圈里绕。 */
function dialogFocusables(): HTMLElement[] {
  return [...app.querySelectorAll<HTMLElement>(".dialog button:not([disabled]), .dialog input:not([disabled]), .dialog textarea:not([disabled])")];
}

/** 关框：焦点还给打开它的那个元素；元素已经不在了（重绘换过节点）就退回主内容区。 */
function closeDialog() {
  ui.dialog = null;
  render();
  const opener = dialogOpener;
  dialogOpener = null;
  if (opener?.isConnected) opener.focus();
  else app.querySelector<HTMLElement>("#content")?.focus();
}

/**
 * 对话框上的一次动作。忙碌态跟着实例走：await 期间这个框可能已经被关掉或被别的框顶替，
 * 那就什么都不做，绝不误关新出现的框。
 */
async function runDialogAction(dialog: Dialog, handler: (self: Dialog) => DialogOutcome | Promise<DialogOutcome>) {
  // 判据与 dialogView() 的禁用判据是同一条：只有**这个框自己**在忙才拒绝。
  // 用全局的 `ui.dialogBusy !== null` 会让旧实例 pending 时新框画成可点、点了却没反应。
  if (dialogIsBusy()) return;
  const token = dialog.token;
  ui.dialogBusy = token;
  render();
  let keep = false;
  let failure: unknown = null;
  try {
    keep = (await handler(dialog)) === "keep";
  } catch (error) {
    // 处理器自己没兜住的意外。不能让它把异常抛穿出去：那样下面的重绘不会发生，
    // 三个键就一直是禁用的样子，要等下一次心跳才恢复。
    failure = error;
  } finally {
    // 无论成功、抛错还是被打断，都先把忙碌态还回去；只清自己那一份。
    if (ui.dialogBusy === token) ui.dialogBusy = null;
  }
  if (failure !== null) toast(String(failure));
  if (ui.dialog?.token !== token) {
    render();
    return;
  }
  if (keep) {
    render();
    focusDialog(ui.dialog);
    return;
  }
  closeDialog();
}

function savePrefs(mutate: (p: Preferences) => void | string, message?: string): Promise<boolean> {
  ui.pendingPrefs++;
  render();
  // 排队后再读取快照，连续编辑不同控件也不会用旧偏好覆盖刚保存的改动。
  const commit = preferencesQueue.then(async () => {
    const next = structuredClone(prefs());
    const problem = mutate(next);
    if (typeof problem === "string") {
      toast(problem);
      return false;
    }
    const snap = await act("update_preferences", { prefs: next });
    if (snap && message) toast(message);
    return !!snap;
  }).finally(() => {
    ui.pendingPrefs--;
    render();
  });
  preferencesQueue = commit.then(() => undefined, () => undefined);
  return commit;
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}

async function loadSettingsExtras() {
  try {
    ui.notificationStatus = await invoke<string>("notification_status");
  } catch {
    ui.notificationStatus = "unknown";
  }
  try {
    ui.autostart = await invoke<boolean>("autostart_status");
  } catch {
    ui.autostart = false;
  }
  try {
    ui.appVersion = await invoke<string>("app_version");
  } catch {
    ui.appVersion = null;
  }
  render();
}

function uid(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1e4).toString(36)}`;
}

// ---------- 动作 ----------

async function handleAction(action: string, el: HTMLElement) {
  const id = el.dataset.id ?? "";
  const d = day();
  switch (action) {
    case "tab": {
      const view = el.dataset.view as View;
      ui.view = view;
      ui.menu = null;
      render();
      if (view === "settings") void loadSettingsExtras();
      break;
    }
    case "menu":
      ui.menu = ui.menu === id ? null : id;
      render();
      break;
    case "toast-close":
      ui.toast = null;
      window.clearTimeout(toastTimer);
      render();
      break;
    case "dialog-cancel":
      // 正在跑的那一下没结束前，取消、Esc、点背景都不算数。
      if (dialogIsBusy()) break;
      closeDialog();
      break;
    case "dialog-confirm":
    case "dialog-alt": {
      const dialog = ui.dialog;
      if (!dialog) break;
      const handler = action === "dialog-alt" ? dialog.alt?.onAlt : dialog.onConfirm;
      if (handler) await runDialogAction(dialog, handler);
      break;
    }
    case "retry-save":
      await act("retry_save");
      break;

    // ----- 今天 -----
    case "start-day":
      if (await act("start_day", { profileId: id })) toast("开始了。这一天从现在算起，不看几点钟。");
      break;
    case "discard-day":
      ask({
        title: "返回开始页？",
        message: "今天记下的所有格、暂停和喝水都会删掉，不归档，也无法恢复。设置和历史记录不受影响。",
        confirmLabel: "丢弃并返回",
        cancelLabel: "继续今天",
        destructive: true,
        onConfirm: async () => {
          if (await act("abandon_day")) toast("已返回开始页，这一天没有归档。");
        },
      });
      break;
    case "end-day":
      ask({
        title: "结束今天？",
        message: "正在走的那一格按已走时长记进今天，然后归档。今天没完成的小时不会滚到明天。",
        confirmLabel: "结束并归档",
        cancelLabel: "再撑一会儿",
        destructive: true,
        onConfirm: async () => {
          if (await act("end_day")) toast("今天结束。没完成的小时不会滚到明天。");
        },
      });
      break;
    case "copy-today-md":
      ui.menu = null;
      if (d) toast((await copyText(markdownForDay(d, null, ui.now))) ? "已复制今天的 Markdown 总结。" : "复制失败。");
      break;
    case "toggle": {
      const wasPaused = d ? isPaused(d) : false;
      const snap = await act("toggle_pause");
      const after = snap?.state.day;
      if (!after) break;
      if (wasPaused) {
        const t = after.timer;
        toast(t ? `继续，${nameOf(t.category, after)}接着走。` : "继续。");
      } else {
        toast("已暂停，这段时间不算数。");
      }
      break;
    }
    case "choose-cat":
      if (!d || d.timer || !d.categories.some((c) => c.id === id)) break;
      // 换项目就把任务框清掉：上一个项目的任务留在这里只会写错。
      if (ui.selectedCategory !== id) ui.taskDraft = "";
      ui.selectedCategory = id;
      ui.minutesDraft = null;
      render();
      break;
    case "adopt": {
      if (!d) break;
      const s = suggest(d, prefs(), ui.now);
      if (s) {
        if (ui.selectedCategory !== s.category) ui.taskDraft = "";
        ui.selectedCategory = s.category;
        ui.minutesDraft = s.minutes;
        render();
      }
      break;
    }
    case "start-block":
      await startBlock();
      break;
    case "extend":
      if (await act("extend_block", { minutes: 10 })) toast("再加 10 分钟。");
      break;
    case "finish":
      await finishBlock();
      break;
    case "toggle-task":
      await act("toggle_task", { index: Number(el.dataset.index) });
      break;
    case "add-task":
      ui.addingTask = true;
      ui.addTaskDraft = "";
      render();
      app.querySelector<HTMLInputElement>("[data-input='add-task']")?.focus();
      break;
    case "commit-task": {
      const text = ui.addTaskDraft.trim();
      if (!text) break;
      if (await act("add_task", { text })) {
        ui.addTaskDraft = "";
        render();
        app.querySelector<HTMLInputElement>("[data-input='add-task']")?.focus();
      }
      break;
    }
    case "end-break":
      if (await act("end_break")) toast("不休息了。开一格就继续。");
      break;
    case "abandon-block": {
      ask({
        title: "放弃这一格？",
        message: "已经过去的时间会记在台账上，但不计入今天的进度。",
        confirmLabel: "确认",
        cancelLabel: "返回",
        destructive: true,
        onConfirm: async () => {
          if (await act("abandon_block")) toast("这一格已记为未计入。");
        },
      });
      break;
    }
    case "water": {
      const snap = await act("drink_water");
      const after = snap?.state.day;
      if (after && after.cups === snap.state.preferences.hydration_goal_cups) toast("今天的水喝够了。");
      break;
    }
    case "water-undo":
      await act("undo_water");
      break;

    // ----- 历史 -----
    case "open-chart-day": {
      const key = Number(id);
      if (!history().some((entry) => entry.day.started_at === key)) break;
      ui.selectedHistoryDay = key;
      ui.expandedDays.add(key);
      render();
      const row = document.getElementById(`archive-${key}`);
      row?.focus({ preventScroll: true });
      row?.scrollIntoView({ block: "start" });
      break;
    }
    case "toggle-day": {
      const key = Number(id);
      if (ui.expandedDays.has(key)) {
        ui.expandedDays.delete(key);
        if (ui.selectedHistoryDay === key) ui.selectedHistoryDay = null;
      }
      else ui.expandedDays.add(key);
      render();
      break;
    }
    case "copy-md": {
      const entry = history().find((h) => h.day.started_at === Number(id));
      if (entry) toast((await copyText(markdownForDay(entry.day, entry.ended_at, ui.now))) ? "已复制这一天的 Markdown。" : "复制失败。");
      break;
    }
    case "delete-day": {
      const entry = history().find((h) => h.day.started_at === Number(id));
      if (!entry) break;
      const label = dayLabel(entry.day.started_at);
      ask({
        title: "永久删除这一天？",
        message: `${label} 的归档记录及其中所有学习格都会从本机删除，无法恢复。`,
        confirmLabel: "永久删除",
        cancelLabel: "取消",
        destructive: true,
        onConfirm: async () => {
          if (await act("delete_history_day", { startedAt: entry.day.started_at })) toast(`已删除 ${label} 的归档记录。`);
        },
      });
      break;
    }

    // ----- 设置：项目 -----
    case "jump-settings": {
      // 换分区＝换页：滚动条回顶，焦点落到新分区上，读屏也跟着走。
      ui.view = "settings";
      ui.settingsSection = id;
      ui.expandedProject = null;
      render();
      document.getElementById("content")?.scrollTo({ top: 0 });
      document.getElementById(`panel-${id}`)?.focus({ preventScroll: true });
      break;
    }
    case "add-project": {
      const p = prefs();
      if (p.categories.length >= MAX_PROJECTS) {
        toast(`最多保留 ${MAX_PROJECTS} 个项目。`);
        break;
      }
      const newId = uid("c");
      let name = "新项目";
      for (let n = 2; p.categories.some((c) => c.name === name); n++) name = `新项目 ${n}`;
      const ok = await savePrefs((x) => {
        x.categories.push({ id: newId, name, short_name: "", default_block_minutes: 50, icon: "ruler", role: "general", block_rationale: "" });
        for (const profile of x.profiles) profile.quotas.push({ category: newId, minutes: 0 });
      });
      if (ok) {
        ui.expandedProject = newId;
        render();
      }
      break;
    }
    case "set-icon":
      await savePrefs((x) => { const c = x.categories.find((c) => c.id === id); if (c) c.icon = el.dataset.icon ?? c.icon; });
      break;
    case "expand-project":
      ui.expandedProject = ui.expandedProject === id ? null : id;
      render();
      break;
    case "del-project": {
      const cat = prefs().categories.find((c) => c.id === id);
      if (!cat) break;
      ask({
        title: `删除“${cat.name || "这个项目"}”？`,
        message: "它会从时间安排里移除；今天和旧历史不会被直接改写。至少要保留一个项目。",
        confirmLabel: "删除项目",
        cancelLabel: "取消",
        destructive: true,
        onConfirm: async () => {
          const ok = await savePrefs((x) => {
            if (x.categories.length <= 1) return "至少要保留一个项目。";
            x.categories = x.categories.filter((c) => c.id !== id);
            for (const profile of x.profiles) profile.quotas = profile.quotas.filter((q) => q.category !== id);
            if (x.profiles.some((profile) => !profile.quotas.some((q) => q.minutes > 0))) return "删掉它以后时间安排里就没有任何配时的项目了，先给其它项目配时。";
          });
          if (ok && ui.expandedProject === id) ui.expandedProject = null;
          render();
        },
      });
      break;
    }
    case "step":
      await stepValue(el);
      break;

    // ----- 设置：网站屏蔽 -----
    case "add-host":
      await addBlockingRule("host");
      break;
    case "add-url":
      await addBlockingRule("url");
      break;
    case "remove-rule":
      ui.removal = { kind: el.dataset.kind === "url" ? "url" : "host", value: el.dataset.value ?? "", step: 1, typed: "" };
      render();
      break;
    case "removal-next":
      if (ui.removal) ui.removal.step = 2;
      render();
      setTimeout(() => app.querySelector<HTMLInputElement>("[data-input='removal']")?.focus(), 30);
      break;
    case "removal-back":
      if (ui.removal) {
        ui.removal.step = 1;
        ui.removal.typed = "";
      }
      render();
      break;
    case "removal-cancel":
      ui.removal = null;
      render();
      break;
    case "removal-confirm": {
      const r = ui.removal;
      if (!r || r.typed.trim() !== r.value || ui.pendingPrefs > 0) break;
      const ok = await savePrefs((x) => {
        if (r.kind === "url") x.blocked_urls = x.blocked_urls.filter((url) => url !== r.value);
        else x.blocked_hosts = x.blocked_hosts.filter((host) => host !== r.value);
      }, r.kind === "url" ? "已移除精确网址规则，浏览器扩展将在同步后解除。" : "已从设置移除整站规则。系统解除后才会恢复访问，请留意下方整站状态。");
      if (ok && ui.removal === r) ui.removal = null;
      render();
      break;
    }
    case "reveal-browser-extension":
      await invoke("reveal_browser_extension").catch((error) => toast(String(error)));
      break;
    case "recheck-blocking":
      if (await act("check_blocking")) toast(blockState().detail ?? blockState().title);
      break;
    case "reapply-blocking":
      await act("reapply_blocking");
      break;

    // ----- 设置：提醒 / 关于 -----
    case "notif-recheck":
      try {
        ui.notificationStatus = await invoke<string>("request_notification_permission");
      } catch (error) {
        toast(String(error));
      }
      render();
      break;
    case "notif-test":
      await invoke("test_notification").catch((error) => toast(String(error)));
      toast("已经发出去了。没看到系统横幅的话，去系统设置的「通知」里把坐功打开。", 9000);
      break;
    case "notif-open":
      await invoke("open_notification_settings").catch((error) => toast(String(error)));
      break;

    // ----- 设置：关于 -----
    case "reveal-state":
      await invoke("reveal_state_file").catch((error) => toast(String(error)));
      break;
    default:
      break;
  }
}

async function startBlock() {
  const d = day();
  if (!d || d.timer) return;
  const p = prefs();
  const s = suggest(d, p, ui.now);
  const selected = ui.selectedCategory ?? s?.category ?? null;
  const cat = d.categories.find((c) => c.id === selected);
  if (!cat) {
    toast("这个项目不在今天的计划里。");
    return;
  }
  // 任务可以一条都不写。
  const tasks = parseTasks(ui.taskDraft);
  const minutes = ui.minutesDraft ?? (selected === s?.category && s ? s.minutes : blockMinutes(cat, p));
  const breakMinutes = ui.breakDraft ?? p.break_minutes;
  const snap = await act("start_block", { categoryId: cat.id, minutes, tasks, breakMinutes });
  if (!snap) return;
  ui.taskDraft = "";
  ui.minutesDraft = null;
  ui.addingTask = false;
  toast(tasks.length ? `开始${cat.name}：${tasks.map((t) => t.text).join(" · ")}` : `开始${cat.name}，${minutes} 分钟。`);
  if (breakMinutes !== p.break_minutes) await savePrefs((x) => { x.break_minutes = breakMinutes; });
}

/** 结束这一格：直接计入，不问任何问题。 */
async function finishBlock() {
  const d = day();
  const t = d?.timer;
  if (!d || !t) return;
  const snap = await act("finish_block");
  const after = snap?.state.day;
  if (!after) return;
  ui.addingTask = false;
  const entry = after.ledger[after.ledger.length - 1];
  if (entry?.accepted) {
    pulseQuota(entry.category);
    creditToast(after, entry.category, entry.seconds);
  }
}

function updateQuota(p: Preferences, profileId: string, categoryId: string, update: (minutes: number) => number): void | string {
  const profile = p.profiles.find((item) => item.id === profileId);
  if (!profile || !p.categories.some((c) => c.id === categoryId)) return "项目已改变，请重新编辑。";
  const quota = profile.quotas.find((q) => q.category === categoryId);
  const minutes = update(quota?.minutes ?? 0);
  if (!Number.isInteger(minutes) || minutes < 0 || minutes > 1440) return "配额请输入 0 到 1440 的整数分钟。";
  if (quota) quota.minutes = minutes;
  else profile.quotas.push({ category: categoryId, minutes });
}

async function stepValue(el: HTMLElement) {
  const delta = Number(el.dataset.delta) * Number(el.dataset.step);
  const min = Number(el.dataset.min);
  const max = Number(el.dataset.max);
  if (![delta, min, max].every(Number.isFinite)) return;
  const adjust = (value: number) => Math.max(min, Math.min(max, value + delta));
  switch (el.dataset.bind) {
    case "quota": {
      const profileId = el.dataset.profile ?? "";
      const categoryId = el.dataset.category ?? "";
      await savePrefs((x) => updateQuota(x, profileId, categoryId, adjust));
      break;
    }
    case "water-min":
      await savePrefs((x) => { x.water_reminder_minutes = adjust(x.water_reminder_minutes); });
      break;
    case "stretch-min":
      await savePrefs((x) => { x.stretch_reminder_minutes = adjust(x.stretch_reminder_minutes); });
      break;
    case "goal":
      await savePrefs((x) => { x.hydration_goal_cups = adjust(x.hydration_goal_cups); });
      break;
    default:
      break;
  }
}

async function addBlockingRule(kind: "url" | "host") {
  if (ui.pendingPrefs > 0) return;
  const draftKey = kind === "url" ? "urlDraft" : "hostDraft";
  const draft = ui[draftKey];
  const preview = kind === "url" ? normalizeUrl(draft) : normalizeHost(draft);
  if ("error" in preview) {
    toast(preview.error);
    return;
  }
  let value = "url" in preview ? preview.url : preview.host;
  try {
    value = kind === "url"
      ? await invoke<string>("normalize_url", { url: value })
      : await invoke<string>("normalize_host", { host: value });
  } catch (error) {
    toast(String(error));
    return;
  }
  const p = prefs();
  if ((kind === "url" ? p.blocked_urls : p.blocked_hosts).includes(value)) {
    toast("这条规则已经在屏蔽列表里。");
    return;
  }
  if (p.blocked_hosts.length + p.blocked_urls.length >= MAX_BLOCK_RULES) {
    toast(`整站和精确网址合计最多添加 ${MAX_BLOCK_RULES} 条规则。`);
    return;
  }
  const conflict = kind === "url" ? conflictingHost(value, p.blocked_hosts) : undefined;
  const message = kind === "url"
    ? conflict ? `已保存。但 ${conflict} 的整站规则仍会屏蔽收藏和视频，请解除对应整站规则。` : "已保存精确网址。浏览器扩展同步后，在学习日生效。"
    : day() ? "已保存整站规则，等待系统应用。请完成管理员授权，并确认下方整站状态。" : "已保存整站规则，下次学习日开始时会申请管理员授权。";
  if (await savePrefs((x) => {
    const rules = kind === "url" ? x.blocked_urls : x.blocked_hosts;
    if (rules.includes(value)) return "这条规则已经在屏蔽列表里。";
    if (x.blocked_hosts.length + x.blocked_urls.length >= MAX_BLOCK_RULES) return `最多添加 ${MAX_BLOCK_RULES} 条规则。`;
    rules.push(value);
  }, message)) {
    if (ui[draftKey] === draft) {
      ui[draftKey] = "";
      // 回车提交时输入框仍有焦点，morphdom 会保留它，需同步清空可见内容。
      const input = app.querySelector<HTMLInputElement>(`#blocked-${kind}-input`);
      if (input) input.value = "";
    }
    render();
  }
}

async function handleChange(key: string, el: HTMLInputElement | HTMLSelectElement) {
  const id = el.dataset.id ?? "";
  const value = el.value;
  const checked = el instanceof HTMLInputElement && el.checked;
  switch (key) {
    case "quota-minutes": {
      const profileId = el.dataset.profile ?? "";
      const categoryId = el.dataset.category ?? "";
      const minutes = Number(value);
      if (!/^\d+$/.test(value) || !Number.isInteger(minutes) || minutes < 0 || minutes > 1440) {
        el.value = String(prefs().profiles.find((p) => p.id === profileId)?.quotas.find((q) => q.category === categoryId)?.minutes ?? 0);
        toast("配额请输入 0 到 1440 的整数分钟，已恢复原值。");
        break;
      }
      await savePrefs((x) => updateQuota(x, profileId, categoryId, () => minutes));
      break;
    }
    case "minutes":
      ui.minutesDraft = Number(value);
      break;
    case "break":
      ui.breakDraft = Number(value);
      break;
    case "block-length":
      await savePrefs((x) => { const c = x.categories.find((c) => c.id === id); if (c) c.default_block_minutes = Number(value); });
      break;
    case "uniform-toggle":
      // 勾上 = 开启统一块长；关掉时把值清成 0，重新打开时尽量沿用上次的数。
      await savePrefs((x) => { x.uniform_block_minutes = checked ? (x.uniform_block_minutes > 0 ? x.uniform_block_minutes : 60) : 0; });
      break;
    case "uniform-length":
      await savePrefs((x) => { x.uniform_block_minutes = Number(value); });
      break;
    case "break-default":
      await savePrefs((x) => { x.break_minutes = Number(value); });
      break;
    case "idle":
      await savePrefs((x) => {
        const n = Number(value);
        x.idle_reminder_enabled = n > 0;
        if (n > 0) x.idle_reminder_minutes = n;
      });
      break;
    case "water-on":
      await savePrefs((x) => { x.water_reminder_enabled = checked; });
      break;
    case "stretch-on":
      await savePrefs((x) => { x.stretch_reminder_enabled = checked; });
      break;
    case "sound":
      await savePrefs((x) => { x.sound_enabled = checked; });
      break;
    case "autostart":
      try {
        ui.autostart = await invoke<boolean>("set_autostart", { enabled: checked });
      } catch (error) {
        toast(String(error));
      }
      render();
      break;
    case "project-name": {
      const name = value.trim();
      const before = prefs().categories.find((c) => c.id === id)?.name ?? "";
      let problem: string | null = null;
      if (!name) problem = "项目名称不能为空。";
      else if ([...name].length > 24) problem = "项目名称最多 24 个字。";
      else if (prefs().categories.some((c) => c.id !== id && c.name === name)) problem = `已经有一个叫「${name}」的项目。`;
      if (problem) {
        el.value = before;
        toast(problem);
        break;
      }
      // 不动 short_name：非空的是用户明确设过的，永远保留；空的本来就表示「自动」，
      // 而自动那一路读的是显示名，改完名字自己就跟着变了。
      if (name !== before) await savePrefs((x) => { const c = x.categories.find((c) => c.id === id); if (c) c.name = name; });
      break;
    }
    case "project-short-name": {
      const short = value.trim();
      const before = prefs().categories.find((c) => c.id === id)?.short_name ?? "";
      if (displayWidth(short) > SHORT_NAME_WIDTH) {
        el.value = before;
        toast(`短名最多 ${SHORT_NAME_WIDTH} 个字符宽（4 个汉字）。`);
        break;
      }
      // 清空 = 回到自动，这是唯一的清空口子。
      if (short !== before) await savePrefs((x) => { const c = x.categories.find((c) => c.id === id); if (c) c.short_name = short; });
      break;
    }
    case "project-icon":
      await savePrefs((x) => { const c = x.categories.find((c) => c.id === id); if (c) c.icon = value; });
      break;
    default:
      break;
  }
  render();
}

// ---------- 事件委托 ----------

document.addEventListener("click", (event) => {
  const target = event.target as HTMLElement;
  const el = target.closest<HTMLElement>("[data-action]");
  if (ui.menu && !target.closest(".anchor")) {
    ui.menu = null;
    if (!el) {
      render();
      return;
    }
  }
  if (!el) return;
  if (el.classList.contains("backdrop") && target.closest("[data-stop]")) return;
  if (el.hasAttribute("disabled")) return;
  void handleAction(el.dataset.action ?? "", el);
});

document.addEventListener("input", (event) => {
  const el = event.target as HTMLInputElement | HTMLTextAreaElement;
  switch (el.dataset.input) {
    case "task":
      ui.taskDraft = el.value;
      break;
    case "add-task":
      ui.addTaskDraft = el.value;
      break;
    case "host":
      ui.hostDraft = el.value;
      render();
      break;
    case "url":
      ui.urlDraft = el.value;
      render();
      break;
    case "removal":
      if (ui.removal) ui.removal.typed = el.value;
      render();
      break;
    default:
      break;
  }
});

document.addEventListener("change", (event) => {
  const el = event.target as HTMLInputElement | HTMLSelectElement;
  const key = el.dataset.change;
  if (key) void handleChange(key, el);
});

document.addEventListener("focusout", (event) => {
  const target = event.target;
  if (target instanceof HTMLInputElement && target.dataset.change === "quota-minutes") requestAnimationFrame(render);
});

document.addEventListener("keydown", (event) => {
  // 焦点不许跑出对话框：Tab 在框内那一圈里绕。判据与渲染共用 topOverlay()。
  if (event.key === "Tab" && topOverlay() === "dialog") {
    const items = dialogFocusables();
    if (!items.length) return;
    event.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = event.shiftKey ? (at <= 0 ? items.length - 1 : at - 1) : at === -1 || at === items.length - 1 ? 0 : at + 1;
    items[next]?.focus();
    return;
  }
  if (event.key === "Escape") {
    // 关的必须是**画在最上面**的那一个，所以判据和 overlays() 共用 topOverlay()。
    const top = topOverlay();
    if (top === "removal") ui.removal = null;
    else if (top === "dialog") {
      if (dialogIsBusy()) return;
      closeDialog();
      return;
    } else if (ui.menu) ui.menu = null;
    else {
      const target = event.target;
      if (!(target instanceof HTMLInputElement) || target.dataset.change !== "quota-minutes") return;
      target.value = String(prefs().profiles.find((p) => p.id === target.dataset.profile)?.quotas.find((q) => q.category === target.dataset.category)?.minutes ?? 0);
      target.blur();
    }
    render();
    return;
  }
  if (event.key !== "Enter") return;
  const target = event.target as HTMLElement;
  if (target instanceof HTMLInputElement) {
    switch (target.dataset.input) {
      case "task":
        event.preventDefault();
        void startBlock();
        return;
      case "host":
      case "url":
        event.preventDefault();
        void addBlockingRule(target.dataset.input);
        return;
      case "removal":
        event.preventDefault();
        void handleAction("removal-confirm", target);
        return;
      case "add-task":
        event.preventDefault();
        void handleAction("commit-task", target);
        return;
      default:
        if (target.dataset.change === "project-name" || target.dataset.change === "quota-minutes") target.blur();
        return;
    }
  }
  if (event.metaKey && ui.view === "today" && !ui.dialog && !ui.removal) {
    event.preventDefault();
    void startBlock();
  }
});

// ---------- 启动 ----------

const params = new URLSearchParams(location.hash.replace(/^#/, ""));
const initialView = params.get("view");
if (initialView === "today" || initialView === "history" || initialView === "settings") ui.view = initialView;

void onSnapshot(applySnapshot);
// 退出前那次保存没写进去：外壳不退出，在这里问用户怎么办。
void onQuitBlocked((reason) => {
  ask({
    id: QUIT_DIALOG_ID,
    title: "退出前保存失败",
    message: `${reason}。现在退出会丢掉自上次成功保存以来的改动。`,
    confirmLabel: "重试并退出",
    cancelLabel: "留在这里",
    destructive: false,
    focus: "confirm",
    onConfirm: async (self): Promise<"keep"> => {
      // 成功的话进程已经没了，走不到下一行；失败就把最新的原因换进正文，框留着。
      try {
        await invoke("quit_after_save");
      } catch (error) {
        // 认 token 不认 id：await 期间这个框若已被换掉，改的就是别人的正文了。
        if (ui.dialog?.token === self.token) ui.dialog.message = `${String(error)}。现在退出会丢掉自上次成功保存以来的改动。`;
      }
      return "keep";
    },
    alt: {
      label: "不保存退出",
      onAlt: async () => {
        try {
          await invoke("quit_without_saving");
        } catch (error) {
          toast(String(error));
        }
      },
    },
  });
});
// 系统通知发不出去时（未签名的本地构建很常见），至少界面里看得见。
void onReminder((reminder) => {
  // 系统横幅显不显示由系统设置决定，App 判断不了，所以界面里这条一律也出。
  toast(`${reminder.title}：${reminder.body}`, 8000);
});
void invoke<Snapshot>("get_snapshot").then((snap) => {
  const qaView = snap.initial_view;
  if (!initialView && (qaView === "today" || qaView === "history" || qaView === "settings")) ui.view = qaView;
  applySnapshot(snap);
  if (ui.view === "settings") void loadSettingsExtras();
  if (snap.initial_scroll) {
    requestAnimationFrame(() => {
      const main = app.querySelector<HTMLElement>("[data-scroll]");
      if (main) main.scrollTop = snap.initial_scroll ?? 0;
    });
  }
});
