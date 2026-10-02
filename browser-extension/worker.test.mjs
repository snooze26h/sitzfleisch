import test from "node:test";
import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { createBlocker, ALARM_NAME, BRIDGE_URL, CACHE_KEY, SESSION_URL } from "./worker.js";
import { blockedPageContext, blockedPageUrl } from "./rules.js";
import { PAIRING_KEY } from "./auth.js";

// 固定测试向量，对应 Rust 测试中的 [7; 32]；不是用户配对凭据。
const TEST_CODE = "07".repeat(32);
const TEST_SESSION = "09".repeat(32);

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

function harness({ initial = snapshot(), stored = {}, initialTabs = [], timeoutMs = 3000, code = TEST_CODE } = {}) {
  const data = { ...(code ? { [PAIRING_KEY]: code } : {}), ...structuredClone(stored) };
  const tabs = new Map(initialTabs.map((tab) => [tab.id, { status: "complete", ...tab }]));
  const updates = [];
  const requests = [];
  const sessions = [];
  const alarms = new Map();
  const state = {
    authenticate: true,
    authenticateSession: true,
    sessionFetch: async () => response({ session: TEST_SESSION }),
    signingCode: TEST_CODE,
    setAccessLevel: async () => {},
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
    storage: { local: { get: (key) => state.storageGet(key), set: (value) => state.set(value), setAccessLevel: async ({ accessLevel }) => { assert.equal(accessLevel, "TRUSTED_CONTEXTS"); await state.setAccessLevel(); } } },
    alarms: { get: async (name) => alarms.get(name), create: async (name, value) => { alarms.set(name, value); }, onAlarm: event() },
    tabs: { get: (id) => state.get(id), query: () => state.query(), update: (id, patch) => state.update(id, patch), onUpdated: event(), onActivated: event(), onRemoved: event() },
    webNavigation: Object.fromEntries(["onBeforeNavigate", "onCommitted", "onHistoryStateUpdated", "onReferenceFragmentUpdated", "onTabReplaced"].map((name) => [name, event()])),
  };
  const blocker = createBlocker({ chromeApi, timeoutMs, now: () => 1_800_000_000_000, fetchImpl: async (url, options) => {
    const session = url === SESSION_URL;
    (session ? sessions : requests).push({ url, options });
    const result = await (session ? state.sessionFetch : state.fetch)(url, options);
    if (session ? state.authenticateSession : state.authenticate) {
      const text = await result.clone().text();
      const status = result.status === 200 ? "200 OK" : "503 Service Unavailable";
      const proof = createHmac("sha256", Buffer.from(state.signingCode, "hex")).update(`sitzfleisch-response-v1\n${options.headers["X-Sitzfleisch-Nonce"]}\n${status}\n${text}`).digest("hex");
      result.headers.set("X-Sitzfleisch-Proof", proof);
    }
    return result;
  } });
  return { blocker, chromeApi, data, tabs, updates, requests, sessions, alarms, state, async boot() { await blocker.start(); await blocker.sync(); await tick(); } };
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
    assert.deepEqual(Object.keys(request.options.headers).filter((key) => key !== "X-Sitzfleisch-Applied"), ["X-Sitzfleisch-Client", "X-Sitzfleisch-Protocol", "X-Sitzfleisch-Nonce", "X-Sitzfleisch-Time", "X-Sitzfleisch-Session", "X-Sitzfleisch-Proof"]);
    assert.match(request.options.headers["X-Sitzfleisch-Nonce"], /^[0-9a-f]{64}$/);
    assert.match(request.options.headers["X-Sitzfleisch-Proof"], /^[0-9a-f]{64}$/);
    assert(!Object.values(request.options.headers).includes(TEST_CODE));
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

test("完整退出后断开连接，解除两类规则并恢复屏蔽页；重新连接后恢复学习日屏蔽", async () => {
  const room = "https://live.bilibili.com/234567";
  const h = harness({ initial: wholeSite(), initialTabs: [{ id: 1, url: room }, { id: 2, url: recommend }] });
  await h.boot();
  assert.match(h.tabs.get(1).url, /blocked.html/);
  h.state.fetch = async () => { throw new Error("app exited"); };
  await h.blocker.sync();
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.blocker.status().active, false);
  assert.equal(h.data[CACHE_KEY].rules.active, false);
  assert.equal(h.data[CACHE_KEY].appliedRevision, null);
  assert.equal(h.tabs.get(1).url, room);
  assert.equal(h.tabs.get(2).url, recommend);
  h.state.fetch = async () => response(wholeSite());
  await h.blocker.sync();
  assert.equal(h.requests.at(-1).options.headers["X-Sitzfleisch-Applied"], undefined);
  assert.equal(h.blocker.status().active, true);
  assert.match(h.tabs.get(1).url, /blocked.html/);
});

test("退出后的首次导航先检查连接，不用旧缓存把可访问网址弹回屏蔽页", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
  await h.boot();
  h.state.fetch = async () => { throw new Error("app exited"); };
  h.tabs.get(1).url = recommend;
  h.chromeApi.webNavigation.onBeforeNavigate.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 1 });
  await h.blocker.sync(); await tick();
  assert.equal(h.tabs.get(1).url, recommend);
  assert.equal(h.updates.length, 0);
});

test("断开时存储失败也在内存解除，下次 worker 启动仍会清理旧缓存", async () => {
  const first = harness();
  await first.boot();
  first.state.fetch = async () => { throw new Error("app exited"); };
  first.state.set = async () => { throw new Error("storage unavailable"); };
  await first.blocker.sync();
  assert.equal(first.blocker.status(home).blocked, false);
  assert.equal(first.blocker.status().connection, "disconnected");
  assert.equal(first.blocker.status().error, "release-save-error");
  const restarted = harness({ stored: first.data, initialTabs: [{ id: 1, url: home }] });
  restarted.state.fetch = async () => { throw new Error("app exited"); };
  await restarted.boot();
  assert.equal(restarted.tabs.get(1).url, home);
  assert.equal(restarted.data[CACHE_KEY].rules.active, false);
});

test("断开后旧屏蔽页恢复失败可以重试，不影响新导航放行", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: home }] });
  await h.boot();
  const update = h.state.update;
  h.state.fetch = async () => { throw new Error("app exited"); };
  h.state.update = async () => { throw new Error("tab temporarily unavailable"); };
  await h.blocker.sync();
  assert.equal(h.blocker.status(home).blocked, false);
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.blocker.status().error, "apply-error");
  h.state.update = update;
  await h.blocker.sync();
  assert.equal(h.tabs.get(1).url, home);
});

test("旧整站缓存离线启动时解除，重新连接后按最新两类规则执行", async () => {
  const room = "https://live.bilibili.com/234567";
  const first = harness({ initial: wholeSite() });
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: room }, { id: 2, url: recommend }] });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.tabs.get(1).url, room);
  assert.equal(h.tabs.get(2).url, recommend);
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

test("收工时存储写入失败也立即解除，不让阻止页一直挡着", async () => {
  const h = harness({ initialTabs: [{ id: 1, url: recommend }] });
  await h.boot();
  assert.match(h.tabs.get(1).url, /^chrome-extension:\/\/test\/blocked.html#/);
  h.state.set = async () => { throw new Error("quota"); };
  h.state.fetch = async () => response(snapshot(false, [recommend], "1111111111111111"));
  await h.blocker.sync().catch(() => {});
  assert.equal(h.blocker.status().active, false, "主程序已说收工，内存里的规则必须先换成不拦");
});

test("浏览器与 worker 重启后清理旧缓存，主程序离线时不再拦截", async () => {
  const first = harness();
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: recommend }, { id: 2, url: favorite }] });
  h.state.fetch = async () => { throw new Error("offline"); };
  await h.boot();
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.blocker.status().active, false);
  assert.equal(h.tabs.get(1).url, recommend);
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
  const getStarted = deferred();
  const originalGet = h.state.get;
  let first = true;
  h.state.get = async (id) => { if (first) { first = false; getStarted.resolve(); return held.promise; } return originalGet(id); };
  h.tabs.get(1).url = recommend;
  h.chromeApi.webNavigation.onBeforeNavigate.emit({ tabId: 1, frameId: 0, url: recommend, timeStamp: 10 });
  await getStarted.promise;
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
  const entered = deferred();
  h.state.fetch = () => { entered.resolve(); return held.promise; };
  const before = h.requests.length;
  const first = h.blocker.sync();
  assert.equal(first, h.blocker.sync());
  h.chromeApi.alarms.onAlarm.emit({ name: ALARM_NAME });
  h.chromeApi.tabs.onActivated.emit({ tabId: 1 });
  await entered.promise;
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

test("首次连接超时不阻止访问，AbortController 结束网络等待", async () => {
  const h = harness({ timeoutMs: 20 });
  h.state.fetch = async (_url, options) => new Promise((_resolve, reject) => {
    options.signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
  });
  await h.boot();
  assert.equal(h.requests[0].options.signal.aborted, true);
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.blocker.status().lastSyncAt, null);
  assert.equal(h.data[CACHE_KEY].rules.active, false);
  assert.equal(h.data[CACHE_KEY].appliedRevision, null);
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
  assert.equal(h.blocker.status().connection, "disconnected");
  assert.equal(h.blocker.status().active, false);
  h.state.fetch = async () => response(snapshot());
  h.chromeApi.runtime.onStartup.emit();
  await h.blocker.sync();
  assert.equal(h.blocker.status().connection, "connected");
});

test("缓存读取失败或损坏且主程序离线时，也恢复已有阻止页", async () => {
  const blockedUrl = `chrome-extension://test/blocked.html#url=${encodeURIComponent(recommend)}`;
  for (const brokenStorage of [true, false]) {
    const h = harness({
      stored: { [CACHE_KEY]: { rules: { ...snapshot(), revision: "bad" } } },
      initialTabs: [{ id: 1, url: blockedUrl }],
    });
    if (brokenStorage) h.state.storageGet = async () => { throw new Error("storage unavailable"); };
    h.state.fetch = async () => { throw new Error("offline"); };
    await h.boot();
    assert.equal(h.tabs.get(1).url, recommend);
    assert.equal(h.blocker.status(recommend).blocked, false);
    assert.equal(h.blocker.status().connection, brokenStorage ? "waiting" : "disconnected");
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

test("worker 重启后不使用旧规则，首次认证失败时恢复已有阻止页", async () => {
  const page = blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite);
  const first = harness();
  await first.boot();
  const h = harness({ stored: first.data, initialTabs: [{ id: 1, url: page }] });
  h.state.fetch = async () => new Response("unavailable", { status: 503 });
  await h.boot();
  assert.equal(h.tabs.get(1).url, recommend);
  assert.equal(h.blocker.status().active, false);
});

test("未配对时不请求服务也不启用旧的未认证缓存", async () => {
  const old = { [CACHE_KEY]: { rules: wholeSite(), appliedRevision: wholeSite().revision, lastSyncAt: 1 } };
  const page = blockedPageUrl("chrome-extension://test/blocked.html", recommend, favorite);
  const h = harness({ code: null, stored: old, initialTabs: [{ id: 1, url: page }] });
  await h.boot();
  assert.equal(h.requests.length, 0);
  assert.equal(h.blocker.status().paired, false);
  assert.equal(h.blocker.status().error, "pairing-required");
  assert.equal(h.blocker.status().active, false);
  assert.equal(h.data[CACHE_KEY].rules.active, false);
  assert.equal(h.tabs.get(1).url, recommend);
});

test("不能限制 storage 到可信上下文时拒绝配对与网络同步", async () => {
  const h = harness({ code: null });
  h.state.setAccessLevel = async () => { throw new Error("cannot restrict storage"); };
  await h.boot();
  assert.equal(h.requests.length, 0);
  assert.equal(h.blocker.status().error, "pairing-storage-error");
  const listener = h.chromeApi.runtime.onMessage.listeners[0];
  const result = await new Promise((resolve) => listener({ type: "pair", code: TEST_CODE }, { id: "test", url: "chrome-extension://test/popup.html" }, resolve));
  assert.deepEqual(result, { ok: false });
  assert.equal(h.data[PAIRING_KEY], undefined);
});

test("伪造端口服务、错误配对密钥和重放响应都不能持久化或应用攻击规则", async () => {
  for (const attack of ["unsigned", "wrong-key", "replay", "tamper"]) {
    const h = harness({ initialTabs: [{ id: 1, url: favorite }] });
    await h.boot();
    let saved;
    h.state.fetch = async () => { saved = response(snapshot(true, [favorite], "aaaaaaaaaaaaaaaa")); return saved; };
    if (attack === "wrong-key") h.state.signingCode = "08".repeat(32);
    if (attack === "unsigned") { h.state.authenticate = false; h.state.authenticateSession = false; }
    if (attack === "replay" || attack === "tamper") {
      // 有效的旧响应仍绑定上一轮 nonce；修改正文也不会保留其有效签名。
      const previousNonce = h.requests.at(-1).options.headers["X-Sitzfleisch-Nonce"];
      const original = JSON.stringify(snapshot());
      const proof = createHmac("sha256", Buffer.from(TEST_CODE, "hex")).update(`sitzfleisch-response-v1\n${previousNonce}\n200 OK\n${original}`).digest("hex");
      h.state.authenticate = false;
      h.state.fetch = async (_url, options) => {
        const nonce = attack === "tamper" ? options.headers["X-Sitzfleisch-Nonce"] : previousNonce;
        const signature = attack === "tamper" ? createHmac("sha256", Buffer.from(TEST_CODE, "hex")).update(`sitzfleisch-response-v1\n${nonce}\n200 OK\n${original}`).digest("hex") : proof;
        return new Response(JSON.stringify(snapshot(true, [favorite], "aaaaaaaaaaaaaaaa")), { headers: { "content-type": "application/json", "x-sitzfleisch-proof": signature } });
      };
    }
    await h.blocker.sync();
    assert.equal(h.blocker.status().error, "authentication-error", attack);
    assert.equal(h.blocker.status().active, false, attack);
    assert.equal(h.tabs.get(1).url, favorite, attack);
    assert.equal(h.data[CACHE_KEY].rules.active, false, attack);
    assert(!JSON.stringify(h.data[CACHE_KEY]).includes(favorite), attack);
    assert.equal(h.data[PAIRING_KEY], TEST_CODE);
  }
});

test("每次同步生成新的 nonce，请求证明用独立实现核验且不包含配对密钥", async () => {
  const h = harness();
  await h.boot();
  await h.blocker.sync();
  const nonces = h.requests.map(({ options }) => options.headers["X-Sitzfleisch-Nonce"]);
  assert.equal(new Set(nonces).size, nonces.length);
  for (const { options } of h.requests) {
    const headers = options.headers;
    const message = `sitzfleisch-request-v1\nGET /v1/rules\n2\n${headers["X-Sitzfleisch-Applied"] ?? ""}\n${headers["X-Sitzfleisch-Time"]}\n${headers["X-Sitzfleisch-Nonce"]}\n${headers["X-Sitzfleisch-Session"]}`;
    assert.equal(headers["X-Sitzfleisch-Proof"], createHmac("sha256", Buffer.from(TEST_CODE, "hex")).update(message).digest("hex"));
    assert(!JSON.stringify(options).includes(TEST_CODE));
  }
});

test("先验证服务端握手，伪造或重放握手不能取得本轮请求证明", async () => {
  for (const attack of ["unsigned", "wrong-key", "replay"]) {
    const h = harness();
    await h.boot();
    const requestsBefore = h.requests.length;
    if (attack === "unsigned") h.state.authenticateSession = false;
    if (attack === "wrong-key") h.state.signingCode = "08".repeat(32);
    if (attack === "replay") {
      const oldNonce = h.sessions.at(-1).options.headers["X-Sitzfleisch-Nonce"];
      const body = JSON.stringify({ session: TEST_SESSION });
      const proof = createHmac("sha256", Buffer.from(TEST_CODE, "hex")).update(`sitzfleisch-response-v1\n${oldNonce}\n200 OK\n${body}`).digest("hex");
      h.state.authenticateSession = false;
      h.state.sessionFetch = async () => new Response(body, { headers: { "content-type": "application/json", "x-sitzfleisch-proof": proof } });
    }
    await h.blocker.sync();
    assert.equal(h.blocker.status().error, "authentication-error");
    assert.equal(h.requests.length, requestsBefore, "服务端身份未验证时不发送带证明的规则请求");
    assert.equal(h.blocker.status().active, false);
  }
});

test("配对只允许本扩展 popup 的完整有效输入，凭据不进入状态回复", async () => {
  const h = harness({ code: null });
  await h.boot();
  const listener = h.chromeApi.runtime.onMessage.listeners[0];
  const popup = { id: "test", url: "chrome-extension://test/popup.html" };
  for (const sender of [{ id: "other", url: popup.url }, { id: "test", url: "https://example.com" }, { id: "test", url: "chrome-extension://test/blocked.html" }]) {
    assert.equal(listener({ type: "pair", code: TEST_CODE }, sender, () => {}), false);
  }
  for (const code of [null, {}, "short", "0".repeat(65), "g".repeat(64)]) {
    assert.equal(listener({ type: "pair", code }, popup, () => {}), false);
  }
  assert.equal(listener({ type: "pair", code: TEST_CODE, extra: true }, popup, () => {}), false);
  const result = await new Promise((resolve) => { assert.equal(listener({ type: "pair", code: TEST_CODE }, popup, resolve), true); });
  assert.equal(result.connection, "connected");
  assert.equal(h.data[PAIRING_KEY], TEST_CODE);
  assert(!JSON.stringify(result).includes(TEST_CODE));
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
    if (unknown) h.state.fetch = async () => new Response("unavailable", { status: 503 });
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
