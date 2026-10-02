import { blockedPageContext, blockedPageUrl, matchingRuleKind, matchesUrl, normalizeUrl, originalBlockedUrl, validateRules } from "./rules.js";
import { PAIRING_KEY, requestAuthentication, sessionChallenge, validPairingCode, verifyResponse } from "./auth.js";

export const BRIDGE_URL = "http://127.0.0.1:47832/v1/rules";
export const SESSION_URL = "http://127.0.0.1:47832/v1/session";
export const CACHE_KEY = "sitzfleisch.rules.v1";
export const ALARM_NAME = "sitzfleisch.sync.v1";
const MAX_RESPONSE_BYTES = 2 * 1024 * 1024;

async function readResponse(response, code, nonce, sessionOnly = false) {
  if (!/^[0-9a-f]{64}$/.test(response.headers.get("x-sitzfleisch-proof") || "")) throw new Error("authentication-error");
  if (![200, 503].includes(response.status) || response.redirected
    || !/^application\/json(?:\s*;|$)/i.test(response.headers.get("content-type") || "")) {
    throw new Error("invalid-response");
  }
  const declaredLength = response.headers.get("content-length");
  const limit = sessionOnly ? 1024 : MAX_RESPONSE_BYTES;
  if (declaredLength !== null && (!/^\d+$/.test(declaredLength) || Number(declaredLength) > limit)) {
    throw new Error("invalid-response");
  }
  if (!response.body) throw new Error("invalid-response");
  const reader = response.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let bytes = 0;
  let text = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > limit) throw new Error("invalid-response");
      text += decoder.decode(value, { stream: true });
    }
    text += decoder.decode();
    await verifyResponse(response, text, code, nonce);
    if (response.status !== 200) throw new Error("rules-unavailable");
    const value = JSON.parse(text);
    if (sessionOnly) {
      if (!value || typeof value !== "object" || Object.keys(value).length !== 1 || !validPairingCode(value.session)) throw new Error("authentication-error");
      return value.session;
    }
    return validateRules(value);
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

// 依赖注入让测试直接驱动生产用的 worker，包括持久化、导航和异步竞争。
export function createBlocker({ chromeApi, fetchImpl = fetch, now = Date.now, timeoutMs = 3000 }) {
  const blockedPage = chromeApi.runtime.getURL("blocked.html");
  const extensionRoot = chromeApi.runtime.getURL("");
  let rules = null;
  let pairingCode = null;
  let pairingStorageAvailable = false;
  let appliedRevision = null;
  let lastSyncAt = null;
  let connection = "waiting";
  let error = null;
  let syncFlight = null;
  let rulesConfirmedInFlight = false;
  let ready = null;
  let started = false;
  let enforcementFailureGeneration = 0;
  const navigationVersions = new Map();
  const navigationTimes = new Map();
  const lastAllowedUrls = new Map();

  function status(url) {
    return {
      ok: true,
      paired: pairingCode !== null,
      connection,
      active: rules?.active ?? false,
      ruleCount: rules?.active ? rules.urls.length + (rules.hosts?.length ?? 0) : 0,
      hostCount: rules?.active ? rules.hosts?.length ?? 0 : 0,
      urlCount: rules?.active ? rules.urls.length : 0,
      supportsHosts: rules?.protocol === 2,
      lastSyncAt,
      error,
      ...(url === undefined ? {} : { blocked: rules === null ? null : matchesUrl(rules, url), blockedKind: matchingRuleKind(rules, url) }),
    };
  }

  async function initialize() {
    try {
      // MV3 storage 默认也开放给内容脚本；凭据必须限制在扩展自身的可信页面与 worker。
      await chromeApi.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" });
      const storedCode = (await chromeApi.storage.local.get(PAIRING_KEY))[PAIRING_KEY];
      if (validPairingCode(storedCode)) pairingCode = storedCode;
      pairingStorageAvailable = true;
    } catch { error = "pairing-storage-error"; }
    try {
      const stored = (await chromeApi.storage.local.get(CACHE_KEY))[CACHE_KEY];
      if (stored !== undefined) {
        const cached = validateRules(stored.rules);
        if (!(stored.lastSyncAt === null || (Number.isSafeInteger(stored.lastSyncAt) && stored.lastSyncAt >= 0))
          || !(stored.appliedRevision === null || stored.appliedRevision === cached.revision)) {
          throw new Error("invalid-cache");
        }
        // 磁盘缓存可能来自旧的未认证版本；重启后必须重新验证服务端再启用规则。
        rules = { protocol: 2, active: false, urls: [], hosts: [], revision: "0000000000000000" };
        // 新 worker 先重新检查当前标签，不沿用上次进程的应用成功声明。
        appliedRevision = null;
        lastSyncAt = stored.lastSyncAt;
        connection = "cache";
      }
    } catch {
      error = "cache-error";
    }
    try {
      const alarm = await chromeApi.alarms.get(ALARM_NAME);
      if (alarm?.periodInMinutes !== 0.5) {
        await chromeApi.alarms.create(ALARM_NAME, { delayInMinutes: 0.5, periodInMinutes: 0.5 });
      }
    } catch {
      error = "alarm-error";
    }
  }

  function nextNavigation(tabId, timestamp) {
    if (!Number.isInteger(tabId) || tabId < 0) return null;
    if (typeof timestamp === "number") {
      if (timestamp < (navigationTimes.get(tabId) ?? -Infinity)) return null;
      navigationTimes.set(tabId, timestamp);
    }
    const version = (navigationVersions.get(tabId) ?? 0) + 1;
    navigationVersions.set(tabId, version);
    return version;
  }

  async function inspectTab(tabId, version, eventUrl, committed = false) {
    await ready;
    let tab;
    try {
      tab = await chromeApi.tabs.get(tabId);
    } catch (cause) {
      // 标签关闭是正常竞争；仍存在的标签查询失败属于应用失败，不能发布成功确认。
      if ((await chromeApi.tabs.query({})).some((current) => current.id === tabId)) throw cause;
      return;
    }
    if (navigationVersions.get(tabId) !== version) return;
    const currentUrl = tab.pendingUrl || tab.url;
    if (typeof currentUrl !== "string") return;
    // 事件排队或 tabs.get 等待时用户可能已切到收藏页，禁止用旧事件强制跳回。
    if (eventUrl !== undefined && eventUrl !== currentUrl) return;
    // 待导航网址还不是用户访问过的页面。只记一个已提交的可用网址，且不落盘。
    if (!tab.pendingUrl && (committed || tab.status === "complete") && !matchesUrl(rules, tab.url)) {
      try { lastAllowedUrls.set(tabId, normalizeUrl(tab.url)); } catch { /* 内部页不作为返回目标。 */ }
    }
    const original = originalBlockedUrl(currentUrl, blockedPage);
    const returnUrl = lastAllowedUrls.get(tabId);
    const destination = original && rules !== null && !matchesUrl(rules, original)
      ? original
      : matchesUrl(rules, currentUrl)
        ? blockedPageUrl(blockedPage, currentUrl, returnUrl && !matchesUrl(rules, returnUrl) ? returnUrl : null)
        : null;
    if (destination) await chromeApi.tabs.update(tabId, { url: destination });
  }

  async function inspectAllTabs() {
    const tabs = await chromeApi.tabs.query({});
    await Promise.all(tabs.map((tab) => {
      if (!Number.isInteger(tab.id) || tab.id < 0) return undefined;
      // 不信任 query 返回的旧 URL，更新前重新读取标签，并与导航代次核对。
      const version = navigationVersions.get(tab.id) ?? 0;
      if (!navigationVersions.has(tab.id)) navigationVersions.set(tab.id, version);
      return inspectTab(tab.id, version);
    }));
  }

  async function synchronize() {
    await ready;
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), timeoutMs);
    let phase = "connection-error";
    try {
      if (!pairingStorageAvailable) { phase = "pairing-storage-error"; throw new Error(phase); }
      if (!pairingCode) { phase = "pairing-required"; throw new Error(phase); }
      const code = pairingCode;
      const common = { "X-Sitzfleisch-Client": "browser-extension-v1", "X-Sitzfleisch-Protocol": "2" };
      const challenge = sessionChallenge();
      const sessionResponse = await fetchImpl(SESSION_URL, {
        method: "GET", headers: { ...common, ...challenge.headers }, cache: "no-store", credentials: "omit",
        redirect: "error", signal: controller.signal,
      });
      phase = "authentication-error";
      const session = await readResponse(sessionResponse, code, challenge.nonce, true);
      const authentication = await requestAuthentication(code, appliedRevision, now(), session);
      const headers = { ...common };
      Object.assign(headers, authentication.headers);
      if (appliedRevision) headers["X-Sitzfleisch-Applied"] = appliedRevision;
      phase = "connection-error";
      const response = await fetchImpl(BRIDGE_URL, {
        method: "GET", headers, cache: "no-store", credentials: "omit",
        redirect: "error", signal: controller.signal,
      });
      phase = "response-error";
      const next = await readResponse(response, code, authentication.nonce);
      // 连接到旧主程序时不能悄悄丢掉已生效的整站缓存；明确收工仍可解除。
      if (rules?.hosts?.length && next.protocol === 1 && next.active) throw new Error("host-rules-unavailable");
      phase = "apply-error";
      appliedRevision = null;
      const syncedAt = now();
      // 解除（收工、退出）不能被写盘失败拦住：内存里的规则先换成不拦，再尽力持久化。
      if (!next.active) rules = next;
      // 先保存完整快照；只有持久化和已开标签检查完成后才报告已应用版本。
      await chromeApi.storage.local.set({ [CACHE_KEY]: { rules: next, lastSyncAt, appliedRevision: null } });
      rules = next;
      rulesConfirmedInFlight = true;
      appliedRevision = null;
      const failureGeneration = enforcementFailureGeneration;
      await inspectAllTabs();
      if (enforcementFailureGeneration !== failureGeneration) throw new Error("navigation-apply-failed");
      await chromeApi.storage.local.set({ [CACHE_KEY]: { rules: next, lastSyncAt: syncedAt, appliedRevision: next.revision } });
      // 写盘期间发生的新导航失败比本轮检查更新，旧成功结果不得覆盖它。
      if (enforcementFailureGeneration !== failureGeneration) throw new Error("navigation-apply-failed");
      appliedRevision = next.revision;
      lastSyncAt = syncedAt;
      connection = "connected";
      error = null;
    } catch (cause) {
      if (cause?.message === "authentication-error") phase = "authentication-error";
      connection = phase === "connection-error" ? "disconnected" : rules ? "cache" : "waiting";
      error = phase;
      if (["connection-error", "authentication-error", "pairing-required", "pairing-storage-error"].includes(phase)) {
        // 完整退出后本机服务不再响应：解除执行状态，不能无限期沿用旧学习日。
        // 这是本地释放，不是主程序的成功同步，因此不产生 ACK，也不改 lastSyncAt。
        rules = { protocol: 2, active: false, urls: [], hosts: [], revision: "0000000000000000" };
        appliedRevision = null;
        try {
          await chromeApi.storage.local.set({ [CACHE_KEY]: { rules, lastSyncAt, appliedRevision: null } });
        } catch { error = "release-save-error"; }
      } else if (phase === "apply-error") {
        appliedRevision = null;
        if (rules !== null) {
          try {
            await chromeApi.storage.local.set({ [CACHE_KEY]: { rules, lastSyncAt, appliedRevision: null } });
          } catch { /* 存储不可用时仍在内存中撤销确认；新 worker 也不继承旧确认。 */ }
        }
      }
      // 断开时恢复已有阻止页；主程序仍在线但返回坏数据时，保留原有效规则。
      try { await inspectAllTabs(); } catch { error = "apply-error"; }
    } finally {
      rulesConfirmedInFlight = false;
      clearTimeout(timeout);
    }
    return status();
  }

  async function pair(code) {
    await ready;
    if (!pairingStorageAvailable || !validPairingCode(code)) return { ok: false };
    if (syncFlight) await syncFlight;
    const released = { protocol: 2, active: false, urls: [], hosts: [], revision: "0000000000000000" };
    await chromeApi.storage.local.set({ [PAIRING_KEY]: code, [CACHE_KEY]: { rules: released, lastSyncAt: null, appliedRevision: null } });
    pairingCode = code;
    rules = released;
    appliedRevision = null;
    lastSyncAt = null;
    return sync();
  }

  function sync() {
    if (!syncFlight) {
      syncFlight = synchronize().finally(() => { syncFlight = null; });
    }
    return syncFlight;
  }

  function inspectEvent(tabId, url, timestamp, committed = false) {
    const version = nextNavigation(tabId, timestamp);
    if (version === null) return;
    // 新导航先确认主程序仍在线，避免退出后的旧缓存先拦截、再立即放行。
    // 本轮已收到有效响应时可直接检查，保留应用过程中的并发导航错误追踪。
    const flight = sync();
    const inspect = () => inspectTab(tabId, version, url, committed);
    void (rulesConfirmedInFlight ? inspect() : flight.then(inspect)).catch(() => {
      enforcementFailureGeneration += 1;
      appliedRevision = null;
      if (connection !== "disconnected") connection = rules ? "cache" : "waiting";
      error = "apply-error";
    });
  }

  async function leaveBlockedPage(sender) {
    await ready;
    const context = blockedPageContext(sender.url, blockedPage);
    const tabId = sender.tab?.id;
    if (!context || !Number.isInteger(tabId) || tabId < 0) return { ok: false };
    const tab = await chromeApi.tabs.get(tabId);
    if ((tab.pendingUrl || tab.url) !== sender.url) return { ok: false };
    // history.back() 会回到刚被替换的屏蔽网址。使用已知可用页，并按最新规则复核。
    const candidate = context.returnUrl || lastAllowedUrls.get(tabId);
    const destination = rules !== null && candidate && !matchesUrl(rules, candidate) ? candidate : "about:blank";
    nextNavigation(tabId);
    await chromeApi.tabs.update(tabId, { url: destination });
    return { ok: true };
  }

  function start() {
    if (started) return ready;
    started = true;
    // MV3 的事件订阅必须在模块求值时同步完成，不能放到 storage/fetch 之后。
    const onNavigation = (details, committed) => {
      if (details.frameId !== 0 || (details.documentLifecycle && details.documentLifecycle !== "active")) return;
      inspectEvent(details.tabId, details.url, details.timeStamp, committed);
    };
    for (const event of ["onBeforeNavigate", "onCommitted", "onHistoryStateUpdated", "onReferenceFragmentUpdated"]) {
      chromeApi.webNavigation[event].addListener((details) => onNavigation(details, event !== "onBeforeNavigate"));
    }
    chromeApi.webNavigation.onTabReplaced.addListener(({ tabId }) => inspectEvent(tabId));
    chromeApi.tabs.onUpdated.addListener((tabId, changeInfo) => {
      if (typeof changeInfo.url === "string" || changeInfo.status === "loading") inspectEvent(tabId, changeInfo.url);
    });
    chromeApi.tabs.onActivated.addListener(({ tabId }) => inspectEvent(tabId));
    chromeApi.tabs.onRemoved.addListener((tabId) => {
      navigationVersions.delete(tabId);
      navigationTimes.delete(tabId);
      lastAllowedUrls.delete(tabId);
    });
    chromeApi.alarms.onAlarm.addListener((alarm) => { if (alarm.name === ALARM_NAME) void sync(); });
    chromeApi.runtime.onStartup.addListener(() => { void sync(); });
    chromeApi.runtime.onInstalled.addListener(() => { void sync(); });
    chromeApi.runtime.onMessage.addListener((message, sender, sendResponse) => {
      if (sender.id !== chromeApi.runtime.id || ![`${extensionRoot}popup.html`, blockedPage].some((page) => sender.url === page || sender.url?.startsWith(`${page}#`))) return false;
      if (message?.type === "pair") {
        if (sender.url !== `${extensionRoot}popup.html` || Object.keys(message).some((key) => !["type", "code"].includes(key)) || !validPairingCode(message.code)) return false;
        void pair(message.code).then(sendResponse).catch(() => sendResponse({ ok: false }));
        return true;
      }
      if (!message || typeof message !== "object" || !["status", "sync", "back"].includes(message.type)
        || Object.keys(message).some((key) => !["type", "url"].includes(key))) return false;
      if (message.type === "back") {
        if (Object.keys(message).length !== 1 || !blockedPageContext(sender.url, blockedPage)) return false;
        void leaveBlockedPage(sender).then(sendResponse).catch(() => sendResponse({ ok: false }));
        return true;
      }
      let url;
      if (Object.hasOwn(message, "url")) {
        try { url = normalizeUrl(message.url); } catch { sendResponse({ ok: false }); return false; }
      }
      void (message.type === "sync" ? sync() : ready).then(() => sendResponse(status(url))).catch(() => sendResponse({ ok: false }));
      return true;
    });
    ready = initialize();
    void sync();
    return ready;
  }

  return { start, sync, status };
}
