import { describeStatus } from "./status.js";

const button = document.getElementById("sync");
function render(state) {
  if (!state?.ok) throw new Error("worker-unavailable");
  const copy = describeStatus(state);
  document.getElementById("connection").textContent = copy.title;
  document.getElementById("detail").textContent = copy.detail;
  const lastSync = document.getElementById("last-sync");
  lastSync.hidden = state.lastSyncAt === null;
  lastSync.textContent = state.lastSyncAt === null ? "" : `上次同步：${new Date(state.lastSyncAt).toLocaleString("zh-CN")}`;
}
async function update(type) {
  if (type === "sync") button.disabled = true;
  try {
    render(await chrome.runtime.sendMessage({ type }));
  } catch {
    document.getElementById("connection").textContent = "扩展暂时不可用";
    document.getElementById("detail").textContent = "请在浏览器的扩展管理页重新加载坐功扩展。";
  } finally {
    if (type === "sync") button.disabled = false;
  }
}
button.addEventListener("click", () => { void update("sync"); });
await update("status");
void update("sync");
