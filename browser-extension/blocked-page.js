import { blockedPageContext } from "./rules.js";
import { describeStatus } from "./status.js";

export function initializeBlockedPage({ chromeApi, document, location, timeoutMs = 5000 }) {
  const context = blockedPageContext(location.href, chromeApi.runtime.getURL("blocked.html"));
  const original = context?.original;
  const detail = document.getElementById("detail");
  const syncButton = document.getElementById("sync");
  const backButton = document.getElementById("back");
  const backHint = document.getElementById("back-hint");
  let updating = false;
  let leaving = false;

  // worker 重启、扩展重新加载等情况下也要结束等待，并给出可以离开的入口。
  async function request(message) {
    let timer;
    try {
      return await Promise.race([
        chromeApi.runtime.sendMessage(message),
        new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("message-timeout")), timeoutMs); }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }

  async function update(type, manual = false) {
    if (!original || updating || leaving) return;
    updating = true;
    syncButton.disabled = true;
    syncButton.textContent = "正在检查…";
    if (manual) detail.textContent = "正在检查收工状态…";
    try {
      const state = await request({ type, url: original });
      if (leaving) return;
      if (!state?.ok) throw new Error("worker-unavailable");
      document.getElementById("scope").textContent = state.blockedKind === "host"
        ? "当前学习日中，这个网站的所有页面已被屏蔽。"
        : "当前学习日中，这个完整网址已被屏蔽。同站的其他网址仍可打开。";
      if (state.blocked === false) {
        leaving = true;
        detail.textContent = "该网址已解除屏蔽，正在返回…";
        location.replace(original);
      } else {
        const checked = manual && state.connection === "connected" && state.blocked === true
          ? "已检查，当前学习日仍未结束。" : "";
        detail.textContent = `${checked}${describeStatus(state).detail}`;
      }
    } catch {
      if (!leaving) detail.textContent = "无法检查学习日状态，请重新加载扩展后刷新本页；也可先打开空白页离开。";
    } finally {
      updating = false;
      syncButton.disabled = leaving;
      syncButton.textContent = "检查收工状态";
    }
  }

  document.getElementById("original").textContent = original || "未保留原网址。收工后请从地址栏重新打开需要的页面。";
  backHint.textContent = context?.returnUrl
    ? "返回最近访问的未屏蔽页面；若该页也已屏蔽，将打开空白页。"
    : "若没有可返回的未屏蔽页面，将打开空白页。";
  backButton.addEventListener("click", async () => {
    if (leaving) return;
    leaving = true;
    backButton.disabled = true;
    backButton.textContent = "正在返回…";
    syncButton.disabled = true;
    try {
      const result = await request({ type: "back" });
      if (!result?.ok) throw new Error("back-unavailable");
    } catch {
      location.replace("about:blank");
    }
  });
  syncButton.addEventListener("click", () => { void update("sync", true); });
  chromeApi.storage.onChanged.addListener(() => { void update("status"); });
  if (original) void update("sync");
  else {
    syncButton.disabled = true;
    detail.textContent = "原网址过长或无法安全保存。可打开空白页离开，收工后重新打开原页面。";
  }
}
