import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { initializeBlockedPage } from "./blocked-page.js";
import { blockedPageUrl } from "./rules.js";

const recommend = "https://www.douyin.com/?recommend=1";
const favorite = "https://www.douyin.com/user/self?showTab=favorite_collection";
const tick = () => new Promise((resolve) => setImmediate(resolve));
const active = { ok: true, blocked: true, active: true, connection: "connected", ruleCount: 4 };
function deferred() { let resolve; const promise = new Promise((r) => { resolve = r; }); return { promise, resolve }; }

function harness({ url, timeoutMs = 5000, send = async () => active } = {}) {
  const elements = Object.fromEntries(["original", "detail", "sync", "back", "back-hint", "scope"].map((id) => [id, {
    textContent: "", disabled: false, listeners: {},
    addEventListener(type, listener) { this.listeners[type] = listener; },
  }]));
  const requests = [];
  const replaced = [];
  const state = { send, storageChanged: null };
  const chromeApi = {
    runtime: { getURL: (path) => `chrome-extension://test/${path}`, sendMessage(message) { requests.push(message); return state.send(message); } },
    storage: { onChanged: { addListener(listener) { state.storageChanged = listener; } } },
  };
  initializeBlockedPage({
    chromeApi, document: { getElementById: (id) => elements[id] }, timeoutMs,
    location: { href: url || blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite), replace: (next) => replaced.push(next) },
  });
  return { elements, requests, replaced, state, click: (id) => elements[id].listeners.click() };
}

test("检查按钮给出等待和已检查反馈，重复点击只发起一次同步", async () => {
  const h = harness();
  await tick();
  const held = deferred();
  h.state.send = () => held.promise;
  h.click("sync");
  h.click("sync");
  assert.equal(h.elements.sync.disabled, true);
  assert.equal(h.elements.sync.textContent, "正在检查…");
  assert.equal(h.elements.detail.textContent, "正在检查收工状态…");
  assert.equal(h.requests.length, 2);
  held.resolve(active);
  await tick();
  assert.equal(h.elements.sync.disabled, false);
  assert.match(h.elements.detail.textContent, /^已检查，当前学习日仍未结束。/);
});

test("整站拦截页明确说明全站范围，不误称同站其他页面可用", async () => {
  const h = harness({ send: async () => ({ ...active, blockedKind: "host", hostCount: 1, urlCount: 3, supportsHosts: true }) });
  await tick();
  assert.match(h.elements.scope.textContent, /网站的所有页面已被屏蔽/);
  assert.doesNotMatch(h.elements.scope.textContent, /其他网址仍可打开/);
  assert.match(h.elements.detail.textContent, /1 条整站规则、3 条精确网址规则/);
});

test("离线检查说明沿用规则，收工后自动恢复原网址", async () => {
  const h = harness();
  await tick();
  h.state.send = async () => ({ ...active, connection: "cache", error: "connection-error" });
  h.click("sync");
  await tick();
  assert.match(h.elements.detail.textContent, /暂时无法连接主程序/);
  assert.match(h.elements.detail.textContent, /继续屏蔽 4 个网址/);
  h.state.send = async () => ({ ...active, blocked: false, active: false });
  h.state.storageChanged();
  await tick();
  assert.deepEqual(h.replaced, [recommend]);
});

test("返回通过 worker 核对，检查结果迟到不会把用户拉回原网址", async () => {
  const held = deferred();
  const h = harness({ send: (message) => message.type === "back" ? Promise.resolve({ ok: true }) : held.promise });
  await h.click("back");
  assert.deepEqual(h.requests.at(-1), { type: "back" });
  assert.equal(h.elements.back.disabled, true);
  held.resolve({ ...active, blocked: false, active: false });
  await tick();
  assert.deepEqual(h.replaced, []);
});

test("worker 断开或拒绝返回时直接打开空白页", async () => {
  for (const send of [async () => { throw new Error("extension invalidated"); }, async () => ({ ok: false })]) {
    const h = harness({ send });
    await tick();
    assert.equal(h.elements.sync.disabled, false);
    assert.match(h.elements.detail.textContent, /可先打开空白页离开/);
    await h.click("back");
    assert.deepEqual(h.replaced, ["about:blank"]);
  }
});

test("无响应的 worker 有超时恢复，返回也不会无限等待", async () => {
  const h = harness({ send: () => new Promise(() => {}), timeoutMs: 5 });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(h.elements.sync.disabled, false);
  assert.match(h.elements.detail.textContent, /无法检查学习日状态/);
  await h.click("back");
  assert.deepEqual(h.replaced, ["about:blank"]);
});

test("旧页和无效原网址解释退出方式，HTML 保留不依赖脚本的出口", async () => {
  const h = harness({ url: "chrome-extension://test/blocked.html#url=javascript:alert(1)" });
  assert.equal(h.requests.length, 0);
  assert.equal(h.elements.sync.disabled, true);
  assert.match(h.elements["back-hint"].textContent, /打开空白页/);
  const html = await readFile(new URL("./blocked.html", import.meta.url), "utf8");
  assert.match(html, /<a[^>]+href="about:blank"[^>]*>打开空白页<\/a>/);
});
