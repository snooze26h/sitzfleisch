//! Supabase 的有界 HTTP 通道。错误只返回固定说明，不回显服务端响应、邮箱或令牌。

use std::io::Read;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{blocking::Client, redirect::Policy, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sitzfleisch_core::sync::SyncDocument;

const MAX_REPLY_BYTES: u64 = 2 * 1024 * 1024 + 64 * 1024;
const MAX_AUTH_BYTES: u64 = 32 * 1024;

#[derive(Clone)]
pub struct Config {
    pub url: String,
    key: String,
}

impl Config {
    pub fn compiled() -> Result<Option<Self>, String> {
        match (option_env!("SITZFLEISCH_SUPABASE_URL"), option_env!("SITZFLEISCH_SUPABASE_PUBLISHABLE_KEY")) {
            (None, None) | (Some(""), Some("")) => Ok(None),
            (Some(url), Some(key)) => Self::parse(url, key, false).map(Some),
            _ => Err("云同步配置不完整，请安装配置完整的版本。".into()),
        }
    }

    fn parse(url: &str, key: &str, local_test: bool) -> Result<Self, String> {
        if url.len() > 2048 || key.len() > 4096 { return Err("云同步配置过长。".into()); }
        let parsed = Url::parse(url).map_err(|_| "云同步服务地址无效。")?;
        let loopback = local_test && parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if (parsed.scheme() != "https" && !loopback) || parsed.host_str().is_none()
            || !parsed.username().is_empty() || parsed.password().is_some()
            || parsed.query().is_some() || parsed.fragment().is_some() || parsed.path() != "/"
        { return Err("云同步服务必须使用不含账号、路径和参数的 HTTPS 地址。".into()); }
        let publishable = key.starts_with("sb_publishable_") && key.len() >= 24
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        let legacy_anon = key.split('.').count() == 3 && key.len() >= 40
            && key.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            && key.split('.').nth(1).and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|claims| claims.get("role").and_then(Value::as_str) == Some("anon"));
        if !publishable && !legacy_anon { return Err("云同步只能配置公开 publishable/anon 密钥，不能使用服务端密钥。".into()); }
        Ok(Self { url: parsed.as_str().trim_end_matches('/').to_owned(), key: key.to_owned() })
    }

    #[cfg(test)]
    pub fn testing(url: &str) -> Self {
        Self::parse(url, "sb_publishable_sitzfleisch_test_only", true).unwrap()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
}

// 不实现 Debug：调试 HTTP 或存储失败时也不能意外打印会话。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub user: AuthUser,
}

pub fn valid_account_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().enumerate().all(|(i, c)| {
        if [8, 13, 18, 23].contains(&i) { c == b'-' } else { c.is_ascii_digit() || (b'a'..=b'f').contains(&c) }
    })
}

pub fn validate_email(email: &str) -> Result<String, String> {
    let value = email.trim();
    if value.len() > 254 || value.len() < 3 || !value.is_ascii() || value.bytes().any(|c| c.is_ascii_control() || c.is_ascii_whitespace()) {
        return Err("请输入有效的邮箱地址。".into());
    }
    let parts: Vec<_> = value.split('@').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[0].len() > 64 || !parts[1].contains('.')
        || parts[1].split('.').any(|part| part.is_empty() || part.starts_with('-') || part.ends_with('-')
            || !part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    { return Err("请输入有效的邮箱地址。".into()); }
    Ok(value.to_owned())
}

impl Session {
    pub fn validate(&self) -> Result<(), String> {
        let token_ok = |token: &str| !token.is_empty() && token.len() <= 16384
            && token.bytes().all(|c| c.is_ascii_graphic());
        if !valid_account_id(&self.user.id) || validate_email(&self.user.email).is_err()
            || !token_ok(&self.access_token) || !token_ok(&self.refresh_token)
            || self.expires_at <= 0 || self.expires_at > 32_503_680_000
        { return Err("登录会话格式无效，请重新登录。".into()); }
        Ok(())
    }

    fn from_response(value: Value, now: i64) -> Result<Self, String> {
        let field = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned).ok_or("登录响应格式无效。");
        let user = value.get("user").ok_or("登录响应缺少用户信息。")?;
        let expires_in = value.get("expires_in").and_then(Value::as_i64).filter(|v| (1..=86400).contains(v))
            .ok_or("登录响应有效期无效。")?;
        let session = Self {
            access_token: field("access_token")?, refresh_token: field("refresh_token")?,
            expires_at: now.saturating_add(expires_in),
            user: AuthUser {
                id: user.get("id").and_then(Value::as_str).ok_or("登录响应格式无效。")?.to_owned(),
                email: user.get("email").and_then(Value::as_str).ok_or("登录响应格式无效。")?.to_owned(),
            },
        };
        session.validate()?;
        Ok(session)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadReply {
    pub revision: i64,
    pub document: Option<SyncDocument>,
    pub server_time: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteReply {
    pub applied: bool,
    pub revision: i64,
    pub committed_revision: Option<i64>,
    pub document: Option<SyncDocument>,
    pub server_time: i64,
}

fn validate_reply(revision: i64, time: i64, document: Option<&SyncDocument>) -> Result<(), String> {
    if !(0..=9_007_199_254_740_991).contains(&revision) || !(1..=32_503_680_000).contains(&time) {
        return Err("云端返回了无效的版本或时间，已保留本机数据。".into());
    }
    if let Some(document) = document { document.validate()?; }
    Ok(())
}

pub struct Http {
    config: Config,
    client: Client,
}

impl Http {
    pub fn new(config: Config) -> Result<Self, String> {
        let mut builder = Client::builder().timeout(Duration::from_secs(12)).connect_timeout(Duration::from_secs(5))
            .redirect(Policy::none()).referer(false).user_agent("Sitzfleisch/1 sync");
        // 只有测试构造器允许回环 HTTP；测试不能经过用户代理泄露合成登录请求。
        if config.url.starts_with("http://") { builder = builder.no_proxy(); }
        let client = builder.build().map_err(|_| "无法初始化安全网络连接。")?;
        Ok(Self { config, client })
    }

    fn post(&self, path: &str, session: Option<&Session>, payload: Value, limit: u64) -> Result<Value, String> {
        let mut request = self.client.post(format!("{}{path}", self.config.url))
            .header("apikey", &self.config.key).json(&payload);
        if let Some(session) = session { request = request.bearer_auth(&session.access_token); }
        let response = request.send().map_err(|_| "暂时无法连接云端，本机记录已保留；联网后会重试。")?;
        if !response.status().is_success() {
            if path == "/auth/v1/token?grant_type=refresh_token"
                && matches!(response.status(), StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED)
            { return Err("登录已过期，请重新登录；本机记录已保留。".into()); }
            return Err(match response.status() {
                StatusCode::UNAUTHORIZED => "登录已过期，请重新登录；本机记录已保留。",
                StatusCode::FORBIDDEN => "云端拒绝了访问，请检查项目权限或重新登录。",
                StatusCode::TOO_MANY_REQUESTS => "请求过于频繁，请稍后再试。",
                StatusCode::NOT_FOUND => "云端同步接口尚未部署，请先完成数据库部署。",
                _ => "云端未接受这次请求，本机数据未被覆盖。",
            }.into());
        }
        if response.content_length().is_some_and(|length| length > limit) { return Err("云端响应过大，已拒绝读取。".into()); }
        let mut bytes = Vec::new();
        response.take(limit + 1).read_to_end(&mut bytes).map_err(|_| "云端响应没有完整接收，将稍后重试。")?;
        if bytes.len() as u64 > limit { return Err("云端响应过大，已拒绝读取。".into()); }
        if bytes.is_empty() { return Ok(Value::Null); }
        serde_json::from_slice(&bytes).map_err(|_| "云端响应格式无效，已保留本机数据。".into())
    }

    pub fn send_code(&self, email: &str) -> Result<(), String> {
        let email = validate_email(email)?;
        self.post("/auth/v1/otp", None, json!({"email":email,"create_user":true}), MAX_AUTH_BYTES).map(|_| ())
    }

    pub fn verify(&self, email: &str, code: &str, now: i64) -> Result<Session, String> {
        let email = validate_email(email)?;
        if !(6..=10).contains(&code.len()) || !code.bytes().all(|c| c.is_ascii_digit()) {
            return Err("请输入邮件中的数字验证码。".into());
        }
        Session::from_response(self.post("/auth/v1/verify", None, json!({"email":email,"token":code,"type":"email"}), MAX_AUTH_BYTES)?, now)
    }

    pub fn refresh(&self, session: &Session, now: i64) -> Result<Session, String> {
        let next = Session::from_response(self.post("/auth/v1/token?grant_type=refresh_token", None,
            json!({"refresh_token":session.refresh_token}), MAX_AUTH_BYTES)?, now)?;
        if next.user.id != session.user.id { return Err("刷新会话返回了不同账号，已停止同步。".into()); }
        Ok(next)
    }

    pub fn read(&self, session: &Session, known_revision: i64) -> Result<ReadReply, String> {
        let result: ReadReply = serde_json::from_value(self.post("/rest/v1/rpc/sitz_sync_read", Some(session),
            json!({"p_known_revision":known_revision}), MAX_REPLY_BYTES)?).map_err(|_| "云端同步响应格式无效。")?;
        validate_reply(result.revision, result.server_time, result.document.as_ref())?;
        if (result.revision == 0 && result.document.is_some())
            || (result.revision > 0 && result.revision != known_revision && result.document.is_none())
            || result.revision < known_revision.max(0)
        { return Err("云端版本不完整或倒退，已停止覆盖。".into()); }
        Ok(result)
    }

    pub fn write(&self, session: &Session, revision: i64, operation_id: &str, device_id: &str, document: &SyncDocument) -> Result<WriteReply, String> {
        document.validate()?;
        let result: WriteReply = serde_json::from_value(self.post("/rest/v1/rpc/sitz_sync_write", Some(session), json!({
            "p_expected_revision":revision,"p_operation_id":operation_id,"p_device_id":device_id,"p_document":document,
        }), MAX_REPLY_BYTES)?).map_err(|_| "云端同步响应格式无效。")?;
        validate_reply(result.revision, result.server_time, result.document.as_ref())?;
        if result.revision < revision || (result.revision > 0 && result.document.is_none())
            || (result.applied && !result.committed_revision.is_some_and(|v| v > revision && v <= result.revision))
            || (!result.applied && result.committed_revision.is_some())
        { return Err("云端提交确认无效，待同步记录已保留。".into()); }
        Ok(result)
    }

    pub fn logout(&self, session: &Session) -> Result<(), String> {
        // 只撤销当前设备的会话；退出 Mac 不能把手机也踢下线。
        self.post("/auth/v1/logout?scope=local", Some(session), json!({}), MAX_AUTH_BYTES).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    struct Request { path: String, authorization: Option<String>, body: Value }

    fn server(replies: Vec<String>) -> (String, thread::JoinHandle<Vec<Request>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(value) => break value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "合成客户端未在期限内发出请求");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => panic!("测试回环连接失败"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                let mut authorization = None;
                loop {
                    line.clear(); reader.read_line(&mut line).unwrap();
                    if line == "\r\n" { break; }
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse::<usize>().unwrap(); }
                        if key.eq_ignore_ascii_case("authorization") { authorization = Some(value.trim().to_owned()); }
                    }
                }
                assert!(length < MAX_REPLY_BYTES as usize);
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                requests.push(Request { path, authorization, body: serde_json::from_slice(&bytes).unwrap() });
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn response(status: u16, body: Value) -> String {
        let body = body.to_string();
        format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    fn auth_response(access: &str, refresh: &str) -> Value {
        json!({"access_token":access,"refresh_token":refresh,"expires_in":3600,
            "user":{"id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","email":"person@example.com","other_server_field":true}})
    }

    fn test_session() -> Session { Session::from_response(auth_response("synthetic-access", "synthetic-refresh"), 1000).unwrap() }

    #[test]
    fn configuration_rejects_credentials_and_untrusted_transport_shapes() {
        let key = "sb_publishable_sitzfleisch_test_only";
        for url in ["http://example.com", "https://user@example.com", "https://example.com/path", "https://example.com/?token=x"] {
            assert!(Config::parse(url, key, false).is_err());
        }
        assert!(Config::parse("https://example.com", "sb_secret_forbidden", false).is_err());
        let claims = URL_SAFE_NO_PAD.encode(br#"{"role":"service_role"}"#);
        assert!(Config::parse("https://example.com", &format!("aaaaaaaaaaaa.{claims}.bbbbbbbbbbbb"), false).is_err());
        assert!(Config::parse("https://example.com", key, false).is_ok());
    }

    #[test]
    fn account_and_email_validation_reject_path_and_header_injection() {
        assert!(valid_account_id("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        for value in ["../../account", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeeg"] { assert!(!valid_account_id(value)); }
        for value in ["person@example.com\r\nX: bad", "a@@example.com", "a@-example.com", "a@localhost"] { assert!(validate_email(value).is_err()); }
        assert_eq!(validate_email(" person+tag@example.com ").unwrap(), "person+tag@example.com");
    }

    #[test]
    fn email_login_refresh_and_local_logout_use_the_documented_routes() {
        let (url, server) = server(vec![response(200, json!({})), response(200, auth_response("access-one", "refresh-one")),
            response(200, auth_response("access-two", "refresh-two")), response(200, Value::Null)]);
        let http = Http::new(Config::testing(&url)).unwrap();
        http.send_code("person@example.com").unwrap();
        let first = http.verify("person@example.com", "123456", 1000).unwrap();
        let next = http.refresh(&first, 2000).unwrap();
        assert_eq!(next.expires_at, 5600);
        assert_eq!(next.refresh_token, "refresh-two");
        http.logout(&next).unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests[0].path, "/auth/v1/otp");
        assert_eq!(requests[0].body["create_user"], true);
        assert!(requests[0].authorization.is_none());
        assert_eq!(requests[1].body["type"], "email");
        assert_eq!(requests[2].path, "/auth/v1/token?grant_type=refresh_token");
        assert_eq!(requests[2].body["refresh_token"], "refresh-one");
        assert_eq!(requests[3].path, "/auth/v1/logout?scope=local");
        assert_eq!(requests[3].authorization.as_deref(), Some("Bearer access-two"));
    }

    #[test]
    fn bad_refresh_requires_reauthentication_and_does_not_echo_provider_secrets() {
        let (url, server) = server(vec![response(400, json!({"code":"refresh_token_not_found","detail":"secret-from-provider"}))]);
        let error = Http::new(Config::testing(&url)).unwrap().refresh(&test_session(), 2000).err().unwrap();
        assert!(error.contains("登录已过期"));
        assert!(!error.contains("secret-from-provider"));
        server.join().unwrap();
    }

    #[test]
    fn redirects_and_oversized_responses_never_become_cloud_state() {
        let (url, server) = server(vec![
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{{}}", MAX_REPLY_BYTES + 1),
        ]);
        let http = Http::new(Config::testing(&url)).unwrap();
        assert!(http.read(&test_session(), -1).is_err());
        assert!(http.read(&test_session(), -1).err().unwrap().contains("过大"));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn cloud_rpc_preserves_operation_identity_across_a_repeated_request() {
        let mut state = sitzfleisch_core::State::new(1000);
        state.start_day("standard", 1000).unwrap();
        state.start_block_with_break("main", 50, vec![], 5).unwrap();
        let document = SyncDocument::capture(&mut state, None, &"a".repeat(32), &[]).unwrap();
        let reply = json!({"applied":true,"revision":2,"committed_revision":2,"document":document,"server_time":1000});
        let (url, server) = server(vec![response(200, reply.clone()), response(200, reply),
            response(200, json!({"revision":2,"document":document,"server_time":2200}))]);
        let http = Http::new(Config::testing(&url)).unwrap();
        for _ in 0..2 {
            let ack = http.write(&test_session(), 1, &"b".repeat(32), &"c".repeat(32), &document).unwrap();
            assert_eq!(ack.committed_revision, Some(2));
        }
        let remote = http.read(&test_session(), 1).unwrap().document.unwrap();
        let mut phone = sitzfleisch_core::State::new(2200);
        remote.apply(&mut phone, 2200).unwrap();
        assert_eq!(phone.day.as_ref().unwrap().timer.as_ref().unwrap().remaining_seconds(), 1800);
        let requests = server.join().unwrap();
        assert_eq!(requests[0].body, requests[1].body);
        assert_eq!(requests[0].path, "/rest/v1/rpc/sitz_sync_write");
        assert!(requests[0].body.get("user_id").is_none());
        assert!(requests[0].body["p_document"]["preferences"].get("blocked_hosts").is_none());
        assert_eq!(requests[2].path, "/rest/v1/rpc/sitz_sync_read");
    }
}
