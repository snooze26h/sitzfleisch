import test from "node:test";
import assert from "node:assert/strict";
import { createBlocker, ALARM_NAME, BRIDGE_URL, CACHE_KEY } from "./worker.js";
import { blockedPageContext, blockedPageUrl } from "./rules.js";

const recommend = "https://www.douyin.com/?recommend=1";
const favorite = "https://www.douyin.com/user/self?from_tab_name=main&showSubTab=video&showTab=favorite_collection";
const home = "https://www.bilibili.com/";
const video = "https://www.bilibili.com/video/BV1234567890/";
const snapshot = (active = true, urls = [recommend, home], revision = "0123456789abcdef") => ({ protocol: 1, active, urls, revision });
const wholeSite = (active = true, hosts = ["live.bilibili.com"], revision = "123456789abcdef0") => ({ ...snapshot(active, [recommend], revision), protocol: 2, hosts });
const response = (value) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
const tick = () => new Promise((resolve) => setImmediate(resolve));
function deferred() { let resolve; const promise = new Promise((r) => { resolve = r; }); return { promise, resolve }; }
function event() {
  const listeners = [];
  return { listeners, addListener(fn) { listeners.push(fn); }, emit(...args) { for (const fn of listeners) fn(...args); } };
}

function harness({ initial = snapshot(), stored = {}, initialTabs = [], timeoutMs = 3000 } = {}) {
  const data = structuredClone(stored);
  const tabs = new Map(initialTabs.map((tab) => [tab.id, { status: "complete", ...tab }]));
  const updates = [];
  const requests = [];
  const alarms = new Map();
  const state = {
    storageGet: async (key) => ({ [key]: data[key] }),
    fetch: async () => response(initial),
    get: async (id) => { if (!tabs.has(id)) throw new Error("closed"); return { ...tabs.get(id) }; },
    query: async () => [...tabs.values()].map((tab) => ({ ...tab })),
    update: async (id, patch) => { if (!tabs.has(id)) throw new Error("closed"); updates.push({ id, ...patch }); Object.assign(tabs.get(id), patch); delete tabs.get(id).pendingUrl; },
    set: async (value) => { Object.assign(data, structuredClone(value)); },
  };
  const chromeApi = {
    runtime: {
      id: "test", getURL: (path) => `chrome-extension://test/${path}`,
      onStartup: event(), onInstalled: event(), onMessage: event(),
    },
    storage: { local: { get: (key) => state.storageGet(key), set: (value) => state.set(value) } },
    alarms: { get: async (name) => alarms.get(name), create: async (name, value) => { alarms.set(name, value); }, onAlarm: event() },
    tabs: { get: (id) => state.get(id), query: () => state.query(), update: (id, patch) => state.update(id, patch), onUpdated: event(), onActivated: event(), onRemoved: event() },
    webNavigation: Object.fromEntries(["onBeforeNavigate", "onCommitted", "onHistoryStateUpdated", "onReferenceFragmentUpdated", "onTabReplaced"].map((name) => [name, event()])),
  };
  const blocker = createBlocker({ chromeApi, timeoutMs, now: () => 1_800_000_000_000, fetchImpl: async (url, options) => { requests.push({ url, options }); return state.fetch(url, options); } });
  return { blocker, chromeApi, data, tabs, updates, requests, alarms, state, async boot() { await blocker.start(); await blocker.sync(); await tick(); } };
}

test("学习日启动同步：拦推荐和首页，已开收藏和视频保持；成功后才 ACK", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: recommend }, { id: 2, url: favorite }, { id: 3, url: home }, { id: 4, url: video }] });
  await h.boot();
  assert.deepEqual(h.updates.map((x) => x.id).sort(), [1, 3]);
  assert.equal(h.tabs.get(2).url, favorite);
  assert.equal(h.tabs.get(4).url, video);
  assert.equal(h.requests[0].options.headers["X-Sitzfleisch-Applied"], undefined);
  assert.equal(h.blocker.status().connection, "connected");
  await h.blocker.sync();
  assert.equal(h.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], snapshot().revision);
  assert.equal(h.data[CACHE_KEY].appliedRevision, snapshot().revision);
  for (const request of h.requests) {
    assert.equal(request.url, BRIDGE_URL);
    assert.equal(request.options.method, "GET");
    assert.equal(request.options.body, undefined);
    assert.equal(request.options.credentials, "omit");
    assert.equal(request.options.redirect, "error");
    assert.equal(request.options.headers["X-Sitzfleisch-Protocol"], "2");
    assert.deepEqual(Object.keys(request.options.headers).filter((key) => key !== "X-Sitzfleisch-Applied"), ["X-Sitzfleisch-Client", "X-Sitzfleisch-Protocol"]);
  }
  assert.equal(h.alarms.get(ALARM_NAME).periodInMinutes, 0.5);
});

test("整站规则同步后替换已打开的直播房间，不依赖网页是否重新解析 DNS", async () => {
  const room = "https://live.bilibili.com/123456?from=search#player";
  const h = harness({ initial: wholeSite(), initialTabs: [{ id: 1, url: room }, { id: 2, url: video }] });
  await h.boot();
  assert.equal(originalForTest(h.tabs.get(1).url), room);
  assert.equal(h.tabs.get(2).url, video);
  assert.equal(h.blocker.status(room).blockedKind, "host");
  assert.equal(h.blocker.status().hostCount, 1);
  assert.equal(h.blocker.status().urlCount, 1);
  assert.equal(h.blocker.status().supportsHosts, true);
  await h.blocker.sync();
  assert.equal(h.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], wholeSite().revision);
  h.tabs.get(1).url = "http://www.live.bilibili.com/another?x=2";
  h.chromeApi.webNavigation.onHistoryStateUpdated.emit({ tabId: 1, frameId: 0, url: h.tabs.get(1).url, timeStamp: 5 });
  await h.blocker.sync(); await tick();
  assert.match(h.tabs.get(1).url, /blocked.html/);
});

function originalForTest(url) { return blockedPageContext(url, "chrome-extension://test/blocked.html")?.original; }

test("整站缓存跨重启离线生效，解除整站后恢复房间且精确规则仍在", async () => {
  const room = "https://live.bilibili.com/234567";
  const first = harness({ initial: wholeSite() });
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: room }, { id: 2, url: recommend }] });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.equal(h.blocker.status().connection, "cache");
  assert.equal(originalForTest(h.tabs.get(1).url), room);
  h.state.fetch = async () => response(wholeSite(true, [], "aaaaaaaaaaaaaaaa"));
  await h.blocker.sync();
  assert.equal(h.tabs.get(1).url, room);
  assert.equal(originalForTest(h.tabs.get(2).url), recommend);
  h.state.fetch = async () => response(wholeSite(true, ["live.bilibili.com"], "bbbbbbbbbbbbbbbb"));
  await h.blocker.sync();
  h.state.fetch = async () => response(wholeSite(false, [], "cccccccccccccccc"));
  await h.blocker.sync();
  assert.equal(h.tabs.get(1).url, room);
  assert.equal(h.tabs.get(2).url, recommend);
});

test("旧主程序不伪报支持整站，协议降级不会丢掉有效整站缓存", async () => {
  const legacy = harness();
  await legacy.boot();
  assert.equal(legacy.blocker.status().supportsHosts, false);
  const h = harness({ initial: wholeSite() });
  await h.boot();
  h.state.fetch = async () => response(snapshot());
  await h.blocker.sync();
  assert.equal(h.blocker.status().connection, "cache");
  assert.equal(h.blocker.status("https://live.bilibili.com/1").blocked, true);
  h.state.fetch = async () => response(snapshot(false, [], "dddddddddddddddd"));
  await h.blocker.sync();
  assert.equal(h.blocker.status().active, false);
});

test("整站下超长和带凭据的导航仍被拦，不在阻止页保存敏感地址", async () => {
  for (const url of [`https://live.bilibili.com/?q=${"x".repeat(5000)}`, "https://user:secret@live.bilibili.com/1"]) {
    const h = harness({ initial: wholeSite(), initialTabs: [{ id: 1, url }] });
    await h.boot();
    assert.equal(h.tabs.get(1).url, "chrome-extension://test/blocked.html");
    assert.equal(h.blocker.status().connection, "connected");
  }
});

test("收工同步取消生效规则并恢复阻止页；下次开始重新拦截", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: recommend }] });
  await h.boot();
  h.state.fetch = async () => response(snapshot(false, [recommend], "1111111111111111"));
  await h.blocker.sync();
  assert.equal(h.tabs.get(1).url, recommend);
  assert.equal(h.blocker.status().active, false);
  assert.equal(h.blocker.status().ruleCount, 0);
  h.state.fetch = async () => response(snapshot(true, [recommend], "2222222222222222"));
  await h.blocker.sync();
  assert.match(h.tabs.get(1).url, /^chrome-extension:\/\/test\/blocked.html#/);
});

test("暂停或主程序离线继续拦，浏览器与 worker 重启从本地恢复", async () => {
  const first = harness();
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: recommend }, { id: 2, url: favorite }] });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.equal(h.blocker.status().connection, "cache");
  assert.equal(h.blocker.status().active, true);
  assert.match(h.tabs.get(1).url, /blocked.html/);
  assert.equal(h.tabs.get(2).url, favorite);
  assert.equal(h.blocker.status().lastSyncAt, 1_800_000_000_000);
  assert.equal(h.requests[0].options.headers["X-Sitzfleisch-Applied"], undefined);
});

for (const eventName of ["onBeforeNavigate", "onCommitted", "onHistoryStateUpdated", "onReferenceFragmentUpdated"]) {
  test(`${eventName} 覆盖普通和 SPA 导航，子 frame 不拦截`, async () => {
    const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
    await h.boot();
    h.tabs.get(1).url = recommend;
    h.chromeApi.webNavigation[eventName].emit({ tabId: 1, frameId: 1, url: recommend, timeStamp: 1 });
    await tick();
    assert.equal(h.updates.length, 0);
    h.chromeApi.webNavigation[eventName].emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 2 });
    await h.blocker.sync(); await tick();
    assert.match(h.tabs.get(1).url, /blocked.html/);
  });
}

test("tabs 更新与激活触发同步，片段网址严格匹配", async () => {
  const target = "https://example.com/view#Study";
  const h = harness({ initial: snapshot(true, [target]), initialTabs: [{ id: 1, url: "https://example.com/view#study" }] });
  await h.boot();
  assert.equal(h.updates.length, 0);
  h.tabs.get(1).url = target;
  h.chromeApi.tabs.onUpdated.emit(1, { url: target });
  await h.blocker.sync(); await tick();
  assert.match(h.tabs.get(1).url, /blocked.html/);
  const before = h.requests.length;
  h.chromeApi.tabs.onActivated.emit({ tabId: 1 });
  await h.blocker.sync();
  assert.equal(h.requests.length, before + 1);
});

test("旧导航异步查询不能把已切到收藏的标签拉回阻止页", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
  await h.boot();
  const held = deferred();
  const originalGet = h.state.get;
  let first = true;
  h.state.get = async (id) => { if (first) { first = false; return held.promise; } return originalGet(id); };
  h.tabs.get(1).url = recommend;
  h.chromeApi.webNavigation.onBeforeNavigate.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 10 });
  await Promise.resolve();
  h.tabs.get(1).url = favorite;
  h.chromeApi.webNavigation.onHistoryStateUpdated.emit({ tabId: 1, frameId: 0, url: favorite, timeStamp: 20 });
  held.resolve({ id: 1, url: recommend });
  await h.blocker.sync(); await tick();
  // 连迟到的旧事件也不能使导航代次倒退。
  h.chromeApi.webNavigation.onCommitted.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 15 });
  await tick();
  assert.equal(h.tabs.get(1).url, favorite);
  assert.equal(h.updates.length, 0);
});

test("尚未显示的新导航优先于旧页面，避免推荐页离开时误拦", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
  await h.boot();
  h.tabs.set(1, { id: 1, url: recommend, pendingUrl: favorite });
  h.chromeApi.webNavigation.onBeforeNavigate.emit({ tabId: 1, frameId: 0, url: favorite, timeStamp: 1 });
  await h.blocker.sync(); await tick();
  assert.equal(h.updates.length, 0);
});

test("同步 single flight 避免并发旧写，事件共用进行中的请求", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
  await h.boot();
  const held = deferred();
  h.state.fetch = () => held.promise;
  const before = h.requests.length;
  const first = h.blocker.sync();
  assert.equal(first, h.blocker.sync());
  h.chromeApi.alarms.onAlarm.emit({ name: ALARM_NAME });
  h.chromeApi.tabs.onActivated.emit({ tabId: 1 });
  await Promise.resolve();
  assert.equal(h.requests.length, before + 1);
  held.resolve(response(snapshot(false, [], "3333333333333333")));
  await first;
  assert.equal(h.blocker.status().active, false);
  assert.equal(h.data[CACHE_KEY].rules.revision, "3333333333333333");
});

test("错误 HTTP、JSON、过大响应或不合法规则保留缓存，不伪报新同步", async () => {
  const h = harness();
  await h.boot();
  for (const invalid of [
    () => new Response("error", { status: 503 }),
    () => new Response("{}", { headers: { "content-type": "text/html" } }),
    () => new Response("{", { headers: { "content-type": "application/json" } }),
    () => new Response("{}", { headers: { "content-type": "application/json", "content-length": "999999999" } }),
    () => response({ ...snapshot(false), active: "false" }),
    () => response({ ...snapshot(false), urls: ["javascript:alert(1)"] }),
  ]) {
    h.state.fetch = async () => invalid();
    await h.blocker.sync();
    assert.equal(h.blocker.status().connection, "cache");
    assert.equal(h.blocker.status().active, true);
    assert.equal(h.blocker.status().error, "response-error");
    assert.deepEqual(h.data[CACHE_KEY].rules, snapshot());
  }
});

test("首次连接超时保持等待，AbortController 结束网络等待", async () => {
  const h = harness({ timeoutMs: 20 });
  h.state.fetch = async (_url, options) => new Promise((_resolve, reject) => {
    options.signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
  });
  await h.boot();
  assert.equal(h.requests[0].options.signal.aborted, true);
  assert.equal(h.blocker.status().connection, "waiting");
  assert.equal(h.blocker.status().lastSyncAt, null);
  assert.equal(h.data[CACHE_KEY], undefined);
});

test("存储或标签应用失败撤销 ACK，下一次请求不声称已成功应用", async () => {
  const h = harness();
  await h.boot();
  h.state.set = async () => { throw new Error("storage unavailable"); };
  await h.blocker.sync();
  assert.equal(h.blocker.status().error, "apply-error");
  await h.blocker.sync();
  assert.equal(h.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], undefined);
  const h2 = harness({ initialTabs: [{ id: 1, url: recommend }] });
  h2.state.update = async () => { throw new Error("denied"); };
  await h2.boot();
  assert.equal(h2.blocker.status().connection, "cache");
  assert.equal(h2.data[CACHE_KEY].appliedRevision, null);
  await h2.blocker.sync();
  assert.equal(h2.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], undefined);
});

test("最终写盘期间的新导航失败不能被旧同步覆盖为已应用", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
  await h.boot();
  const held = deferred();
  const finalWriteStarted = deferred();
  const originalSet = h.state.set;
  h.state.set = async (value) => {
    if (value[CACHE_KEY].appliedRevision !== null) {
      finalWriteStarted.resolve();
      await held.promise;
    }
    await originalSet(value);
  };
  const flight = h.blocker.sync();
  await finalWriteStarted.promise;
  h.tabs.get(1).url = recommend;
  h.state.update = async () => { throw new Error("navigation denied"); };
  h.chromeApi.webNavigation.onHistoryStateUpdated.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 1 });
  await tick();
  assert.equal(h.blocker.status().error, "apply-error");
  held.resolve();
  await flight;
  assert.equal(h.blocker.status().connection, "cache");
  assert.equal(h.blocker.status().error, "apply-error");
  assert.equal(h.data[CACHE_KEY].appliedRevision, null);
  await h.blocker.sync();
  assert.equal(h.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], undefined);
});

test("标签仍存在时 tabs.get 失败不能被当成正常关闭并 ACK", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: recommend }] });
  h.state.get = async () => { throw new Error("tabs API failure"); };
  await h.boot();
  assert.equal(h.blocker.status().error, "apply-error");
  assert.equal(h.data[CACHE_KEY].appliedRevision, null);
});

test("损坏缓存不作为有效屏蔽规则，闹钟和浏览器启动仍会重试", async () => {
  const h = harness({ stored: { [CACHE_KEY]: { rules: { ...snapshot(), revision: "bad" } } } });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.equal(h.blocker.status().connection, "waiting");
  assert.equal(h.blocker.status().active, false);
  h.state.fetch = async () => response(snapshot());
  h.chromeApi.runtime.onStartup.emit();
  await h.blocker.sync();
  assert.equal(h.blocker.status().connection, "connected");
});

test("规则未知不能视为已经收工，缓存读取失败或损坏且离线时保留已有阻止页", async () => {
  const blockedUrl = `chrome-extension://test/blocked.html#url=${encodeURIComponent(recommend)}`;
  for (const brokenStorage of [true, false]) {
    const h = harness({
      stored: { [CACHE_KEY]: { rules: { ...snapshot(), revision: "bad" } } },
      initialTabs: [{ id: 1, url: blockedUrl }],
    });
    if (brokenStorage) h.state.storageGet = async () => { throw new Error("storage unavailable"); };
    h.state.fetch = async () => { throw new Error("offline"); };
    await h.boot();
    assert.equal(h.updates.length, 0);
    assert.equal(h.tabs.get(1).url, blockedUrl);
    assert.equal(h.blocker.status(recommend).blocked, null);
  }
});

test("消息仅接受自己的 popup / 阻止页，不能成为网页代理或临时放行接口", async () => {
  const h = harness();
  await h.boot();
  const listener = h.chromeApi.runtime.onMessage.listeners[0];
  const own = { id: "test", url: "chrome-extension://test/popup.html" };
  assert.equal(listener({ type: "sync" }, { id: "test", url: "https://example.com/" }, () => {}), false);
  assert.equal(listener({ type: "sync", target: "https://example.com/" }, own, () => {}), false);
  assert.equal(listener({ type: "allow", url: recommend }, own, () => {}), false);
  const result = await new Promise((resolve) => {
    assert.equal(listener({ type: "status", url: recommend }, own, resolve), true);
  });
  assert.equal(result.blocked, true);
});

async function goBack(h, tabId = 1, senderUrl = h.tabs.get(tabId).url) {
  const listener = h.chromeApi.runtime.onMessage.listeners[0];
  return new Promise((resolve) => {
    assert.equal(listener({ type: "back" }, { id: "test", url: senderUrl, tab: { id: tabId } }, resolve), true);
  });
}

for (const eventName of ["onCommitted", "onHistoryStateUpdated", "onReferenceFragmentUpdated"]) {
  test(`${eventName} 拦截后返回已访问的收藏页，不重入推荐页`, async () => {
    const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
    await h.boot();
    h.tabs.get(1).url = recommend;
    h.chromeApi.webNavigation[eventName].emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 1 });
    await h.blocker.sync(); await tick();
    assert.equal(blockedPageContext(h.tabs.get(1).url, "chrome-extension://test/blocked.html").returnUrl, favorite);
    assert.deepEqual(await goBack(h), { ok: true });
    assert.equal(h.tabs.get(1).url, favorite);
    await h.blocker.sync();
    assert.equal(h.tabs.get(1).url, favorite);
  });
}

test("返回目标跟随阻止页恢复，worker 重启和离线仍可返回", async () => {
  const page = blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite);
  const first = harness();
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: page }] });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.deepEqual(await goBack(h), { ok: true });
  assert.equal(h.tabs.get(1).url, favorite);
});

test("旧阻止页、无历史和返回目标也被屏蔽时有空白页出口", async () => {
  const blockedPage = "chrome-extension://test/blocked.html";
  for (const page of [blockedPageUrl(blockedPage, recommend), blockedPageUrl(blockedPage, recommend, home)]) {
    const h = harness({ initialTabs: [{ id: 1, url: page }] });
    await h.boot();
    assert.deepEqual(await goBack(h), { ok: true });
    assert.equal(h.tabs.get(1).url, "about:blank");
  }
});

test("返回目标后来被加入规则时不放行，规则未知也不放行", async () => {
  const page = blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite);
  for (const unknown of [false, true]) {
    const h = harness({ initial: snapshot(true, [recommend, favorite]), initialTabs: [{ id: 1, url: page }] });
    if (unknown) h.state.fetch = async () => { throw new Error("offline"); };
    await h.boot();
    assert.deepEqual(await goBack(h), { ok: true });
    assert.equal(h.tabs.get(1).url, "about:blank");
  }
});

test("未提交的网址不成为返回目标，关闭标签后清理旧返回值", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite, status: "loading" }] });
  await h.boot();
  h.chromeApi.webNavigation.onBeforeNavigate.emit({ tabId: 1, frameId: 0, url: favorite, timeStamp: 1 });
  await h.blocker.sync(); await tick();
  h.tabs.get(1).url = recommend;
  h.chromeApi.webNavigation.onCommitted.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 2 });
  await h.blocker.sync(); await tick();
  assert.equal(blockedPageContext(h.tabs.get(1).url, "chrome-extension://test/blocked.html").returnUrl, null);
  await goBack(h);
  assert.equal(h.tabs.get(1).url, "about:blank");

  h.tabs.get(1).url = favorite;
  h.chromeApi.webNavigation.onCommitted.emit({ tabId: 1, frameId: 0, url: favorite, timeStamp: 3 });
  await h.blocker.sync(); await tick();
  h.chromeApi.tabs.onRemoved.emit(1);
  h.tabs.get(1).url = recommend;
  await h.blocker.sync();
  assert.equal(blockedPageContext(h.tabs.get(1).url, "chrome-extension://test/blocked.html").returnUrl, null);
});

test("返回消息不接受 popup、自选目标或过时页面，不能覆盖正在离开的标签", async () => {
  const page = blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite);
  const h = harness({ initialTabs: [{ id: 1, url: page }] });
  await h.boot();
  const listener = h.chromeApi.runtime.onMessage.listeners[0];
  const sender = { id: "test", url: page, tab: { id: 1 } };
  assert.equal(listener({ type: "back" }, { ...sender, url: "chrome-extension://test/popup.html" }, () => {}), false);
  assert.equal(listener({ type: "back", url: recommend }, sender, () => {}), false);
  h.tabs.get(1).pendingUrl = video;
  assert.deepEqual(await goBack(h, 1, page), { ok: false });
  assert.equal(h.updates.length, 0);
});
