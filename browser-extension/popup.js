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
document.getElementById("pairing").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = document.getElementById("pairing-code");
  const result = document.getElementById("pairing-result");
  const code = input.value.trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(code)) { result.textContent = "请粘贴主程序复制的完整配对码。"; return; }
  const submit = event.currentTarget.querySelector("button");
  submit.disabled = true;
  try {
    const state = await chrome.runtime.sendMessage({ type: "pair", code });
    if (!state?.ok) throw new Error("pairing-failed");
    input.value = "";
    render(state);
    result.textContent = state.connection === "connected" ? "配对成功。" : "配对码已保存，请检查主程序连接状态。";
  } catch { result.textContent = "配对未能保存，请重新加载扩展后重试。"; }
  finally { submit.disabled = false; }
});
await update("status");
void update("sync");
