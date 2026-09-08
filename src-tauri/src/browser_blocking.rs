//! 浏览器执行精确网址与整站规则。这里只提供回环读取，不接收浏览记录或修改应用状态的命令。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sitzfleisch_core as core;
use tauri::Manager;

const ADDRESS: &str = "127.0.0.1:47832";
const MAX_HEADERS: usize = 8192;
const CLIENT: &str = "browser-extension-v1";
const CONNECTED_FOR: Duration = Duration::from_secs(90);

#[derive(Clone, Default, Serialize)]
pub(crate) struct Status {
    pub available: bool,
    pub connected: bool,
    pub synced: bool,
    pub supports_hosts: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct Bridge {
    available: bool,
    last_seen: Option<Instant>,
    applied: Option<String>,
    protocol: u8,
    error: Option<String>,
}

impl Bridge {
    pub fn status(&self, rules: &Rules) -> Status {
        let connected = self.available
            && self.last_seen.is_some_and(|at| at.elapsed() < CONNECTED_FOR);
        Status {
            available: self.available,
            connected,
            synced: connected && self.applied.as_deref() == Some(&rules.revision_for(self.protocol)),
            supports_hosts: connected && self.protocol == 2,
            error: self.error.clone(),
        }
    }

    fn received(&mut self, protocol: u8, applied: Option<String>) {
        self.last_seen = Some(Instant::now());
        self.applied = applied;
        self.protocol = protocol;
    }
}

#[derive(Serialize)]
pub(crate) struct Rules {
    protocol: u8,
    active: bool,
    urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hosts: Option<Vec<String>>,
    revision: String,
}

impl Rules {
    pub fn from_state(state: &core::State) -> Self {
        Self::for_protocol(state, 2)
    }

    fn for_protocol(state: &core::State, protocol: u8) -> Self {
        let active = state.day.is_some();
        // 收工后不传无效规则，也让扩展明确清空上次学习日的缓存。
        let urls = if active { state.preferences.blocked_urls.clone() } else { vec![] };
        let hosts = (protocol == 2).then(|| if active { state.preferences.blocked_hosts.clone() } else { vec![] });
        let mut rules = Self { protocol, active, urls, hosts, revision: String::new() };
        rules.revision = rules.revision_for(protocol);
        rules
    }

    fn revision_for(&self, protocol: u8) -> String {
        // 老扩展仍可同步精确网址与收工状态；只有显式支持 v2 的扩展才收到整站字段。
        let encoded = if protocol == 2 {
            serde_json::to_vec(&(self.active, &self.urls, &self.hosts))
        } else {
            serde_json::to_vec(&(self.active, &self.urls))
        }.expect("网址规则可以序列化");
        // 版本标记只比较是否已应用同一组规则，不作为身份凭据或安全摘要。
        let fingerprint = encoded.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        format!("{fingerprint:016x}")
    }
}

/// 浏览器普通网页不能携带自定义头跨域读取此接口；同时拒绝网页 Origin 与重绑定 Host。
/// 协议头不提供身份认证：本机客户端（包括其他登录账号）可读取配置的规则。
/// 接口只返回学习日开关与生效网址，不提供计时记录、浏览记录或写入命令。
fn parse_request(bytes: &[u8]) -> Result<(u8, Option<String>), ()> {
    if bytes.len() > MAX_HEADERS || !bytes.is_ascii() || bytes.windows(4).position(|part| part == b"\r\n\r\n") != bytes.len().checked_sub(4) {
        return Err(());
    }
    let request = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut lines = request.split("\r\n");
    if lines.next() != Some("GET /v1/rules HTTP/1.1") {
        return Err(());
    }
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if line.is_empty() { break; }
        let (name, value) = line.split_once(':').ok_or(())?;
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            return Err(());
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim_matches([' ', '\t']);
        if value.bytes().any(|c| c.is_ascii_control()) || headers.insert(name, value).is_some() {
            return Err(());
        }
    }
    if headers.get("host") != Some(&ADDRESS) || headers.get("x-sitzfleisch-client") != Some(&CLIENT) {
        return Err(());
    }
    if let Some(origin) = headers.get("origin") {
        let id = origin.strip_prefix("chrome-extension://").ok_or(())?;
        if id.len() != 32 || !id.bytes().all(|c| (b'a'..=b'p').contains(&c)) {
            return Err(());
        }
    }
    if headers.contains_key("transfer-encoding")
        || headers.get("content-length").is_some_and(|length| *length != "0")
    {
        return Err(());
    }
    let applied = headers.get("x-sitzfleisch-applied").map(|value| value.to_string());
    if applied.as_ref().is_some_and(|value| {
        value.len() != 16 || !value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }) {
        return Err(());
    }
    let protocol = match headers.get("x-sitzfleisch-protocol").copied() {
        None | Some("1") => 1,
        Some("2") => 2,
        _ => return Err(()),
    };
    Ok((protocol, applied))
}

fn read_request(stream: &mut impl Read) -> Result<Vec<u8>, ()> {
    read_request_with_budget(stream, Duration::from_secs(1))
}

fn read_request_with_budget(stream: &mut impl Read, budget: Duration) -> Result<Vec<u8>, ()> {
    let started = Instant::now();
    let mut request = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        // 单次 read 的超时不足以防止慢速逐字节占用；总时限保证其他浏览器仍能同步。
        if started.elapsed() >= budget { return Err(()); }
        let count = stream.read(&mut chunk).map_err(|_| ())?;
        if count == 0 || started.elapsed() >= budget { return Err(()); }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > MAX_HEADERS { return Err(()); }
        if request.windows(4).any(|part| part == b"\r\n\r\n") {
            return Ok(request);
        }
    }
}

fn write_response(stream: &mut impl Write, status: &str, body: &[u8]) -> std::io::Result<()> {
    // 不开放网页 CORS、不缓存；一个请求后关闭连接，避免持久连接及管线化增加解析面。
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn serve(stream: &mut TcpStream, shared: &super::Shared) {
    let timeout = Some(Duration::from_secs(1));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return;
    }
    let (protocol, applied) = match read_request(stream).and_then(|bytes| parse_request(&bytes)) {
        Ok(request) => request,
        Err(()) => {
            let _ = write_response(stream, "403 Forbidden", br#"{"error":"request rejected"}"#);
            return;
        }
    };
    let response = {
        let state = shared.state.lock().unwrap();
        // 保护模式不把临时空状态同步过去，否则读档失败反而会解除浏览器里仍有效的屏蔽。
        if shared.write_protected.lock().unwrap().is_some()
            || state.preferences.blocked_urls.len() + state.preferences.blocked_hosts.len() > core::MAX_BLOCK_RULES
            || state.preferences.blocked_urls.iter().any(|url| core::validate_url(url).as_ref() != Ok(url))
            || state.preferences.blocked_hosts.iter().any(|host| core::validate_host(host).as_ref() != Ok(host))
        {
            None
        } else {
            let rules = Rules::for_protocol(&state, protocol);
            shared.browser_bridge.lock().unwrap().received(protocol, applied);
            Some(serde_json::to_vec(&rules).expect("网址规则可以序列化"))
        }
    };
    match response {
        Some(body) => { let _ = write_response(stream, "200 OK", &body); }
        None => { let _ = write_response(stream, "503 Service Unavailable", br#"{"error":"rules unavailable"}"#); }
    }
}

pub(crate) fn start(app: &tauri::AppHandle, isolated: bool) {
    if isolated {
        // 隔离存档不能把 active=false 发给用户真正的扩展，更不能抢占正式应用端口。
        app.state::<super::Shared>().browser_bridge.lock().unwrap().error = Some(
            "隔离数据目录下不连接真实浏览器扩展。".into(),
        );
        return;
    }
    let listener = match TcpListener::bind(ADDRESS) {
        Ok(listener) => listener,
        Err(_) => {
            app.state::<super::Shared>().browser_bridge.lock().unwrap().error = Some(
                "无法启动浏览器连接（本机端口 47832 不可用）。关闭另一份坐功后重启本应用。".into(),
            );
            return;
        }
    };
    app.state::<super::Shared>().browser_bridge.lock().unwrap().available = true;
    let app = app.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => serve(&mut stream, &app.state::<super::Shared>()),
                Err(_) => {
                    let shared = app.state::<super::Shared>();
                    let mut bridge = shared.browser_bridge.lock().unwrap();
                    bridge.available = false;
                    bridge.error = Some("浏览器连接已停止，请重启坐功。".into());
                    break;
                }
            }
        }
    });
}

pub(crate) fn new_bridge() -> Mutex<Bridge> { Mutex::new(Bridge::default()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn request(extra: &str) -> Vec<u8> {
        format!("GET /v1/rules HTTP/1.1\r\nHost: {ADDRESS}\r\nX-Sitzfleisch-Client: {CLIENT}\r\n{extra}\r\n").into_bytes()
    }

    #[test]
    fn only_accepts_bounded_read_only_extension_requests() {
        assert_eq!(parse_request(&request("")), Ok((1, None)));
        let ack = "0123456789abcdef";
        assert_eq!(parse_request(&request(&format!("X-Sitzfleisch-Applied: {ack}\r\nOrigin: chrome-extension://{}\r\n", "a".repeat(32)))), Ok((1, Some(ack.into()))));
        assert_eq!(parse_request(&request("X-Sitzfleisch-Protocol: 2\r\n")), Ok((2, None)));
        assert!(parse_request(&request("X-Sitzfleisch-Protocol: 3\r\n")).is_err());
        for extra in [
            "Origin: https://evil.example\r\n", "Origin: null\r\n",
            "Origin: chrome-extension://bad\r\n", "Host: evil.example\r\n",
            "Content-Length: 1\r\n", "Transfer-Encoding: chunked\r\n",
            "X-Sitzfleisch-Applied: invalid\r\n", " Bad: folded\r\n",
        ] {
            assert!(parse_request(&request(extra)).is_err(), "{extra}");
        }
        let valid = String::from_utf8(request("")).unwrap();
        for changed in [
            valid.replace("GET ", "POST "), valid.replace("GET ", "OPTIONS "),
            valid.replace("/v1/rules ", "/v1/rules?write=1 "),
            valid.replace(ADDRESS, "localhost:47832"),
            valid.replace(&format!("X-Sitzfleisch-Client: {CLIENT}\r\n"), ""),
            format!("{valid}GET /another HTTP/1.1\r\n\r\n"),
            String::from_utf8(request(&format!("X-Large: {}\r\n", "a".repeat(MAX_HEADERS)))).unwrap(),
        ] {
            assert!(parse_request(changed.as_bytes()).is_err());
        }
    }

    #[test]
    fn partial_headers_and_response_lengths_are_bounded() {
        assert!(read_request(&mut std::io::Cursor::new(b"GET /v1/rules HTTP/1.1\r\n")).is_err());
        assert!(read_request(&mut std::io::Cursor::new(vec![b'a'; MAX_HEADERS + 1])).is_err());
        assert_eq!(read_request(&mut std::io::Cursor::new(request(""))).unwrap(), request(""));
        let mut response = vec![];
        write_response(&mut response, "200 OK", "学习".as_bytes()).unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.contains("Content-Length: 6\r\n"));
        assert!(!response.contains("Access-Control-Allow-Origin"));
    }

    #[test]
    fn slow_drip_cannot_hold_the_rules_server_indefinitely() {
        struct SlowReader;
        impl Read for SlowReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_millis(15));
                buffer[0] = b'a';
                Ok(1)
            }
        }
        assert!(read_request_with_budget(&mut SlowReader, Duration::from_millis(25)).is_err());
    }

    #[test]
    fn acknowledgements_track_rule_changes_day_end_and_disconnection() {
        let mut state = core::State::new(1000);
        state.preferences.blocked_urls = vec!["https://www.douyin.com/?recommend=1".into()];
        let idle = Rules::from_state(&state);
        assert!(!idle.active);
        assert!(idle.urls.is_empty());
        state.start_day("standard", 1000).unwrap();
        let active = Rules::from_state(&state);
        assert_ne!(active.revision, idle.revision);
        assert_eq!(active.urls, state.preferences.blocked_urls);
        let mut bridge = Bridge { available: true, ..Bridge::default() };
        assert!(!bridge.status(&active).connected);
        bridge.received(2, None);
        assert!(bridge.status(&active).connected);
        assert!(!bridge.status(&active).synced);
        bridge.received(2, Some(active.revision.clone()));
        assert!(bridge.status(&active).synced);
        // 暂停不会解除精确网址；只有收工或删除规则改变同步版本。
        state.tick(1001);
        assert_eq!(Rules::from_state(&state).revision, active.revision);
        state.preferences.blocked_urls.push("https://www.bilibili.com/".into());
        assert!(!bridge.status(&Rules::from_state(&state)).synced);
        state.abandon_day().unwrap();
        assert_eq!(Rules::from_state(&state).revision, idle.revision);
        bridge.received(2, Some(idle.revision.clone()));
        assert!(bridge.status(&idle).synced);
        bridge.last_seen = Some(Instant::now() - CONNECTED_FOR);
        assert!(!bridge.status(&idle).connected);
        assert!(!bridge.status(&idle).synced);
    }

    /// 一天里所有「没在跑格」的状态都必须继续屏蔽：手动按停、休息、
    /// 睡醒后的自动暂停、以及压根还没开格。只有收工 / 放弃才解除。
    /// 旧测试只跑了一次 tick，从没真的按停过一格，这条把整条路径走完。
    #[test]
    fn every_paused_state_keeps_blocking_until_the_day_ends() {
        let mut state = core::State::new(1_000);
        state.preferences.blocked_hosts = vec!["live.bilibili.com".into()];
        state.preferences.blocked_urls = vec!["https://www.bilibili.com/".into()];
        let idle = Rules::from_state(&state);
        assert!(!idle.active, "没开学习日就不该屏蔽");

        state.start_day("standard", 1_000).unwrap();
        // 还没开格——0.8.0 之后这本身就算暂停。
        let day = Rules::from_state(&state);
        assert!(day.active && state.day.as_ref().unwrap().is_paused());

        state.start_block("main", 25, vec![]).unwrap();
        state.tick(1_060);
        assert!(!state.day.as_ref().unwrap().is_paused(), "格在跑");
        assert_eq!(Rules::from_state(&state).revision, day.revision, "开格不改规则");

        // 手动按停。
        state.toggle_pause(1_060).unwrap();
        assert!(state.day.as_ref().unwrap().is_paused());
        let paused = Rules::from_state(&state);
        assert!(paused.active, "按停时必须还在屏蔽");
        assert_eq!(paused.revision, day.revision, "暂停不该让扩展重新同步");
        assert_eq!(paused.urls, state.preferences.blocked_urls);
        assert_eq!(paused.hosts.as_ref().unwrap(), &state.preferences.blocked_hosts);

        // 继续 → 结束这一格 → 进入休息。
        state.toggle_pause(1_120).unwrap();
        state.finish_block(1_200).unwrap();
        assert!(state.day.as_ref().unwrap().is_paused(), "结束一格就进暂停");
        assert!(Rules::from_state(&state).active, "休息时必须还在屏蔽");

        // 合盖 / 休眠：心跳跨过 120 秒，自动暂停。
        state.tick(1_500);
        assert!(state.day.as_ref().unwrap().is_paused());
        assert_eq!(Rules::from_state(&state).revision, day.revision, "自动暂停也不改规则");

        // 只有收工才解除。
        state.end_day(1_600).unwrap();
        let ended = Rules::from_state(&state);
        assert!(!ended.active);
        assert!(ended.urls.is_empty() && ended.hosts.as_ref().unwrap().is_empty());
        assert_eq!(ended.revision, idle.revision);
    }

    #[test]
    fn whole_site_changes_require_a_new_ack_and_legacy_clients_remain_compatible() {
        let mut state = core::State::new(1000);
        state.preferences.blocked_hosts = vec!["live.bilibili.com".into()];
        state.preferences.blocked_urls = vec!["https://www.douyin.com/?recommend=1".into()];
        state.start_day("standard", 1000).unwrap();
        let active = Rules::from_state(&state);
        assert_eq!(active.hosts.as_ref().unwrap(), &state.preferences.blocked_hosts);
        let legacy = Rules::for_protocol(&state, 1);
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert!(encoded.get("hosts").is_none(), "老扩展严格检查字段，不能直接塞入 hosts");
        assert_eq!(encoded["protocol"], 1);
        let mut bridge = Bridge { available: true, ..Bridge::default() };
        bridge.received(1, Some(legacy.revision));
        assert!(bridge.status(&active).synced);
        assert!(!bridge.status(&active).supports_hosts);
        bridge.received(2, Some(active.revision));
        assert!(bridge.status(&Rules::from_state(&state)).supports_hosts);
        assert!(bridge.status(&Rules::from_state(&state)).synced);
        state.preferences.blocked_hosts.clear();
        assert!(!bridge.status(&Rules::from_state(&state)).synced);
        state.abandon_day().unwrap();
        let idle = Rules::from_state(&state);
        assert!(idle.hosts.unwrap().is_empty());
        assert!(idle.urls.is_empty());
    }
}
