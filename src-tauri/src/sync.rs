//! 跨设备同步外壳：网络串行、计时独立；写请求先落盘，再发送，确认只消费对应请求。

use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sitzfleisch_core::{self as core, sync::{merge, MergeChoice, SyncDocument}};
use tauri::{AppHandle, Manager};

use crate::sync_http::{Config, Http, Session, WriteReply};
use crate::sync_store::{new_id, AccountFile, PendingWrite, RemoteVersion, Store};
use crate::{Snapshot, Shared};

/// UI 操作绑定用户当时看到的对象；计时秒数变化不使按钮失效，另一端换格/暂停会使旧操作失效。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionTarget {
    pub revision: i64,
    pub account_id: Option<String>,
    pub day_id: Option<String>,
    pub timer_id: Option<String>,
    pub paused: bool,
    pub break_until: Option<i64>,
}

pub enum TargetCheck { Identity, Live, Preferences }

struct AccountTransition<'a>(&'a Shared);

impl<'a> AccountTransition<'a> {
    fn begin(shared: &'a Shared) -> Result<Self, String> {
        if shared.switching_account.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err("正在切换账号，请稍候。".into());
        }
        Ok(Self(shared))
    }
}

impl Drop for AccountTransition<'_> {
    fn drop(&mut self) { self.0.switching_account.store(false, std::sync::atomic::Ordering::SeqCst); }
}

fn save_before_switch(shared: &Shared) -> Result<(), String> {
    // begin 已阻止新业务操作；已持有 state 的旧操作会先完成，这次 save 会保存它的最终结果。
    crate::advance_current(shared);
    crate::save(shared);
    if shared.save_error.lock().unwrap().is_some() || shared.write_protected.lock().unwrap().is_some() {
        return Err("请先保存当前记录，再切换账号；尚未保存的内容仍留在本机。".into());
    }
    Ok(())
}

fn inherit_device_settings(current: &core::State, next: &mut core::State) {
    let shared = core::sync::SharedPreferences::capture(&next.preferences);
    next.preferences = current.preferences.clone();
    shared.apply(&mut next.preferences);
    let offset = next.last_tick.saturating_sub(current.last_tick);
    next.water_clock_checked_at = current.water_clock_checked_at.map(|at| at.saturating_add(offset));
    next.water_reminded_at = current.water_reminded_at.map(|at| at.saturating_add(offset));
}

fn rebase_local_day(state: &mut core::State, offset: i64) -> Result<(), String> {
    let shift = |at: &mut i64| -> Result<(), String> {
        *at = at.checked_add(offset).filter(|value| (0..=core::sync::MAX_UNIX_SECONDS).contains(value))
            .ok_or("本机计时时刻无法映射到云端，请先核对设备时钟。")?;
        Ok(())
    };
    shift(&mut state.last_tick)?;
    if let Some(day) = &mut state.day {
        shift(&mut day.started_at)?;
        if let Some(timer) = &mut day.timer { if timer.started_at != 0 { shift(&mut timer.started_at)?; } }
        for entry in &mut day.ledger {
            if entry.started_at != 0 { shift(&mut entry.started_at)?; }
            shift(&mut entry.ended_at)?;
        }
        for span in &mut day.pauses { shift(&mut span.started_at)?; if let Some(at) = &mut span.ended_at { shift(at)?; } }
        for span in &mut day.rests { shift(&mut span.started_at)?; shift(&mut span.ended_at)?; }
        if let Some(at) = &mut day.break_until { shift(at)?; }
    }
    // 历史记录保留原时刻；无法假定过去每一天的设备误差都与今天相同。
    state.water_clock_checked_at = state.water_clock_checked_at.map(|at| at.saturating_add(offset));
    state.water_reminded_at = state.water_reminded_at.map(|at| at.saturating_add(offset));
    Ok(())
}

#[derive(Clone, Serialize)]
pub struct MigrationSummary {
    local_days: usize,
    cloud_days: usize,
    local_active: bool,
    cloud_active: bool,
    cloud_empty: bool,
}

#[derive(Clone, Serialize)]
pub struct ConflictSummary {
    fields: Vec<String>,
    local_days: usize,
    remote_days: usize,
    local_active: bool,
    remote_active: bool,
}

#[derive(Clone, Serialize)]
pub struct Status {
    configured: bool,
    phase: &'static str,
    account_id: Option<String>,
    email: Option<String>,
    revision: i64,
    pending: bool,
    last_synced_at: Option<i64>,
    error: Option<String>,
    migration: Option<MigrationSummary>,
    conflict: Option<ConflictSummary>,
    recovery_path: Option<String>,
}

pub struct Runtime {
    pub config: Option<Config>,
    pub store: Option<Store>,
    pub account: Option<AccountFile>,
    session: Option<Session>,
    candidate: Option<Session>,
    import_remote: Option<RemoteVersion>,
    pub blocked: Option<String>,
    error: Option<String>,
    needs_login: bool,
    online: bool,
    last_synced_at: Option<i64>,
    recovery_path: Option<String>,
    pub clock_offset: i64,
    clock_anchor: Option<(u64, i64)>,
    worker: Option<thread::Thread>,
    last_code_sent: Option<u64>,
}

impl Runtime {
    pub fn disabled() -> Self {
        Self { config: None, store: None, account: None, session: None, candidate: None,
            import_remote: None, blocked: None, error: None, needs_login: false, online: false,
            last_synced_at: None, recovery_path: None, clock_offset: 0, clock_anchor: None, worker: None, last_code_sent: None }
    }

    pub fn open(data_dir: &Path, state: &mut core::State) -> Self {
        let mut runtime = Self::disabled();
        let result = (|| -> Result<(), String> {
            runtime.config = Config::compiled()?;
            let Some(config) = &runtime.config else { return Ok(()); };
            let store = Store::open(data_dir, &config.url)?;
            if let Some(user) = store.active_user()? {
                let mut account = store.load(&user)?.ok_or("当前账号存档缺失，已停止自动覆盖。")?;
                let mut restored = account.state.clone();
                runtime.clock_offset = account.clock_offset;
                account.draft.apply(&mut restored, runtime.now())?;
                account.state = restored.clone();
                *state = restored;
                runtime.account = Some(account);
                runtime.needs_login = true;
            }
            runtime.store = Some(store);
            Ok(())
        })();
        if let Err(error) = result { runtime.blocked = Some(error); }
        runtime
    }

    pub fn active(&self) -> bool { self.account.is_some() }

    #[cfg(any(desktop, test))]
    pub fn action_target(&self, state: &core::State) -> ActionTarget {
        ActionTarget {
            revision: self.account.as_ref().map_or(0, |account| account.revision),
            account_id: self.account.as_ref().map(|account| account.user.id.clone()),
            day_id: state.day.as_ref().map(|day| day.sync_id.clone()),
            timer_id: state.day.as_ref().and_then(|day| day.timer.as_ref().map(|timer| timer.sync_id.clone())),
            paused: state.day.as_ref().is_some_and(core::Day::is_paused),
            break_until: state.day.as_ref().and_then(|day| day.break_until),
        }
    }

    pub fn check_target(&self, state: &core::State, expected: Option<&ActionTarget>, check: &TargetCheck) -> Result<(), String> {
        if expected.and_then(|target| target.account_id.as_deref()) != self.account.as_ref().map(|account| account.user.id.as_str()) {
            return Err("当前账号已改变，请重新打开这项操作。".into());
        }
        let Some(account) = &self.account else { return Ok(()); };
        let expected = expected.ok_or("共享内容已更新，请刷新后重试。")?;
        if !(0..=9_007_199_254_740_991).contains(&expected.revision)
            || expected.day_id.iter().chain(expected.timer_id.iter()).any(|id| id.is_empty() || id.len() > 128
                || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte)))
            || expected.break_until.is_some_and(|at| !(0..=core::sync::MAX_UNIX_SECONDS).contains(&at))
        { return Err("操作目标无效，请刷新后重试。".into()); }
        if matches!(check, TargetCheck::Preferences) && expected.revision != account.revision { return Err("另一台设备刚更新了记录，请核对当前内容后重试。".into()); }
        if matches!(check, TargetCheck::Live) && (expected.day_id.as_deref() != state.day.as_ref().map(|day| day.sync_id.as_str())
            || expected.timer_id.as_deref() != state.day.as_ref().and_then(|day| day.timer.as_ref().map(|timer| timer.sync_id.as_str()))
            || expected.paused != state.day.as_ref().is_some_and(core::Day::is_paused)
            || expected.break_until != state.day.as_ref().and_then(|day| day.break_until))
        { return Err("当前计时已经改变，请核对后重新操作。".into()); }
        Ok(())
    }

    pub fn now(&self) -> i64 {
        crate::sync_clock::anchored_time(self.clock_anchor, crate::sync_clock::continuous_seconds(),
            crate::now_unix().saturating_add(self.clock_offset))
    }

    fn calibrate(&mut self, server_time: i64) {
        self.clock_offset = server_time.saturating_sub(crate::now_unix());
        self.clock_anchor = crate::sync_clock::continuous_seconds().map(|ticks| (ticks, server_time));
        if let Some(account) = &mut self.account { account.clock_offset = self.clock_offset; }
    }

    fn reproject(&self, state: &mut core::State) -> Result<(), String> {
        if let Some(account) = &self.account { account.draft.apply(state, self.now())?; }
        Ok(())
    }

    pub fn wake(&self) { if let Some(worker) = &self.worker { worker.unpark(); } }

    pub fn status(&self, state: &core::State) -> Status {
        let pending = self.account.as_ref().is_some_and(|account| account.pending.is_some()
            || account.revision == 0 || !account.draft.equivalent(&account.base, self.now()));
        let conflict = self.account.as_ref().and_then(|account| account.conflict.as_ref().map(|remote| {
            let mut fields: Vec<String> = merge(&account.base, &account.draft, &remote.document, self.now(), None)
                .err().map(|error| error.fields).unwrap_or_default().into_iter()
                .map(|field| if field.starts_with("history") { "history".into() } else { field })
                .collect();
            fields.sort(); fields.dedup(); fields.truncate(8);
            ConflictSummary { fields, local_days: account.draft.history.len(), remote_days: remote.document.history.len(),
                local_active: account.draft.day.is_some(), remote_active: remote.document.day.is_some() }
        }));
        let phase = if self.blocked.is_some() { "error" }
            else if self.config.is_none() { "unconfigured" }
            else if self.candidate.is_some() { "awaiting_import" }
            else if self.account.is_none() { "signed_out" }
            else if self.needs_login { "reauth" }
            else if conflict.is_some() { "conflict" }
            else if !self.online { "offline" }
            else if pending { "pending" } else { "synced" };
        Status {
            configured: self.config.is_some(), phase,
            account_id: self.account.as_ref().map(|account| account.user.id.clone()),
            email: self.account.as_ref().map(|account| account.user.email.clone())
                .or_else(|| self.candidate.as_ref().map(|session| session.user.email.clone())),
            revision: self.account.as_ref().map_or(0, |account| account.revision), pending,
            last_synced_at: self.last_synced_at, error: self.blocked.clone().or_else(|| self.error.clone()), conflict,
            migration: self.candidate.as_ref().map(|_| MigrationSummary {
                local_days: state.history.len(), cloud_days: self.import_remote.as_ref().map_or(0, |remote| remote.document.history.len()),
                local_active: state.day.is_some(), cloud_active: self.import_remote.as_ref().is_some_and(|remote| remote.document.day.is_some()),
                cloud_empty: self.import_remote.is_none(),
            }), recovery_path: self.recovery_path.clone(),
        }
    }

    /// 在 state 锁内调用。此前的请求不可改写：服务端可能已经接受，只是确认还没到。
    pub fn changed(&mut self, state: &mut core::State, deleted_ids: &[String]) -> Result<(), String> {
        if let Some(error) = &self.blocked { return Err(error.clone()); }
        if let Some(account) = &mut self.account {
            let candidate = SyncDocument::capture(state, Some(&account.draft), &new_id()?, deleted_ids)?;
            // core 的单机60条裁剪不是用户删除，账号视图同样保留完整共享历史。
            state.history = candidate.history.clone();
            account.draft = candidate;
            account.state = state.clone();
        }
        self.wake();
        Ok(())
    }

    fn prepare_write(&mut self) -> Result<Option<PendingWrite>, String> {
        let now = self.now();
        let Some(account) = &mut self.account else { return Ok(None); };
        if account.conflict.is_some() { return Ok(None); }
        if account.pending.is_none() && (account.revision == 0 || !account.draft.equivalent(&account.base, now)) {
            account.pending = Some(PendingWrite { operation_id: new_id()?, expected_revision: account.revision,
                document: account.draft.clone() });
        }
        Ok(account.pending.clone())
    }

    fn accept_remote(&mut self, remote: RemoteVersion, state: &mut core::State) -> Result<bool, String> {
        let now = self.now().max(remote.server_time);
        let Some(account) = &mut self.account else { return Ok(false); };
        if remote.revision < account.revision { return Err("云端版本倒退，已保留本机记录。".into()); }
        match merge(&account.base, &account.draft, &remote.document, now, None) {
            Ok(merged) => {
                merged.apply(state, now)?;
                account.base = remote.document;
                account.revision = remote.revision;
                account.draft = merged;
                account.pending = None;
                account.conflict = None;
                self.error = None;
                Ok(true)
            }
            Err(_) => {
                account.pending = None;
                account.conflict = Some(remote);
                self.error = Some("本机和另一台设备修改了同一份内容，请选择要继续使用的版本。".into());
                Ok(false)
            }
        }
    }

    fn acknowledge(&mut self, sent: &PendingWrite, reply: WriteReply, state: &mut core::State) -> Result<bool, String> {
        let Some(account) = &mut self.account else { return Ok(false); };
        if account.pending.as_ref().map(|pending| &pending.operation_id) != Some(&sent.operation_id) {
            return Ok(false);
        }
        let document = reply.document.ok_or("提交确认缺少云端文档，待同步请求已保留。")?;
        if reply.applied {
            // 发送过程中可能又做了操作；共同基线前移到已提交的那一份，而非丢弃新草稿。
            account.base = sent.document.clone();
            account.revision = reply.committed_revision.ok_or("提交确认缺少版本。")?;
        }
        self.accept_remote(RemoteVersion { revision: reply.revision, document, server_time: reply.server_time }, state)
    }

    fn failed(&mut self, error: String) {
        if error.contains("登录已过期") { self.needs_login = true; }
        self.online = false;
        self.error = Some(error);
    }
}

fn notify_changed(app: &AppHandle, applied: bool) {
    if applied {
        #[cfg(target_os = "android")]
        crate::mobile::mark_dirty(app);
        #[cfg(desktop)]
        {
            let shared = app.state::<Shared>();
            let wanted = crate::blocking_wanted(&shared);
            let needs_apply = {
                let blocking = shared.blocking.lock().unwrap();
                wanted != blocking.active && !blocking.busy && blocking.error.is_none()
            };
            if !shared.isolated && needs_apply { crate::spawn_apply_blocking(app); }
        }
    }
    // 远端改变历史或当前账号时必须带完整历史，不能让界面一直沿用上个账号的记录。
    crate::broadcast_full(app);
}

fn session_for_request(app: &AppHandle, http: &Http) -> Result<Session, String> {
    let shared = app.state::<Shared>();
    let (session, scope) = {
        let runtime = shared.sync.lock().unwrap();
        let session = runtime.session.clone().ok_or("登录已过期，请重新登录；本机记录已保留。")?;
        let scope = runtime.store.as_ref().ok_or("同步存储未就绪。")?.scope.clone();
        (session, scope)
    };
    if session.expires_at > crate::now_unix().saturating_add(90) { return Ok(session); }
    let refreshed = http.refresh(&session, crate::now_unix())?;
    let encoded = serde_json::to_string(&refreshed).map_err(|_| "登录会话编码失败。")?;
    // 刷新令牌会轮换，成功后先存到安全存储，不能等下一次普通存档。
    crate::sync_credentials::store(app, &shared.data_dir, &scope, &encoded)?;
    shared.sync.lock().unwrap().session = Some(refreshed.clone());
    Ok(refreshed)
}

fn network_step(app: &AppHandle) -> Result<(), String> {
    let shared = app.state::<Shared>();
    let _network = shared.sync_network.lock().unwrap();
    let (config, should_work) = {
        let runtime = shared.sync.lock().unwrap();
        (runtime.config.clone(), runtime.account.is_some() && !runtime.needs_login && runtime.blocked.is_none())
    };
    if !should_work || shared.exiting.load(std::sync::atomic::Ordering::SeqCst) { return Ok(()); }
    if shared.write_protected.lock().unwrap().is_some() { return Err("存档处于保护模式，云同步已暂停。".into()); }
    let http = Http::new(config.ok_or("当前版本未配置云同步。")?)?;
    let session = session_for_request(app, &http)?;
    let pending = shared.sync.lock().unwrap().prepare_write()?;
    if pending.is_some() {
        crate::save(&shared);
        if shared.save_error.lock().unwrap().is_some() { return Err("请先重试保存；未保存的修改不会上传。".into()); }
    }
    let mut applied = false;
    if let Some(pending) = pending {
        let device = shared.sync.lock().unwrap().store.as_ref().ok_or("同步存储未就绪。")?.device_id.clone();
        let reply = http.write(&session, pending.expected_revision, &pending.operation_id, &device, &pending.document)?;
        let mut state = shared.state.lock().unwrap();
        let mut runtime = shared.sync.lock().unwrap();
        runtime.calibrate(reply.server_time);
        applied = runtime.acknowledge(&pending, reply, &mut state)?;
        runtime.reproject(&mut state)?;
        runtime.online = true;
        runtime.last_synced_at = Some(crate::now_unix());
    } else {
        let revision = {
            let runtime = shared.sync.lock().unwrap();
            let account = runtime.account.as_ref().ok_or("账号未就绪。")?;
            // 有冲突时继续拉最新版本，但不能推进共同基线或自动覆盖本机。
            account.conflict.as_ref().map_or(account.revision, |remote| remote.revision)
        };
        let reply = http.read(&session, revision)?;
        let mut state = shared.state.lock().unwrap();
        let mut runtime = shared.sync.lock().unwrap();
        runtime.calibrate(reply.server_time);
        if let Some(document) = reply.document {
            let remote = RemoteVersion { revision: reply.revision, document, server_time: reply.server_time };
            if runtime.account.as_ref().is_some_and(|account| account.conflict.is_some()) {
                runtime.account.as_mut().unwrap().conflict = Some(remote);
            } else { applied = runtime.accept_remote(remote, &mut state)?; }
        }
        runtime.reproject(&mut state)?;
        runtime.online = true;
        runtime.last_synced_at = Some(crate::now_unix());
        if runtime.account.as_ref().is_none_or(|account| account.conflict.is_none()) { runtime.error = None; }
    }
    crate::save(&shared);
    notify_changed(app, applied);
    Ok(())
}

pub fn start(app: &AppHandle) {
    let configured = app.state::<Shared>().sync.lock().unwrap().config.is_some();
    if !configured { return; }
    let handle = app.clone();
    thread::spawn(move || {
        let shared = handle.state::<Shared>();
        shared.sync.lock().unwrap().worker = Some(thread::current());
        let scope = {
            let runtime = shared.sync.lock().unwrap();
            runtime.account.as_ref().and_then(|_| runtime.store.as_ref().map(|store| store.scope.clone()))
        };
        if let Some(scope) = scope {
            let _network = shared.sync_network.lock().unwrap();
            let restored = crate::sync_credentials::load(&handle, &shared.data_dir, &scope)
                .and_then(|raw| raw.ok_or("登录凭据不可用，请重新登录；本机记录已保留。".into()))
                .and_then(|raw| serde_json::from_str::<Session>(&raw).map_err(|_| "登录凭据格式无效，请重新登录。".into()))
                .and_then(|session| session.validate().map(|()| session));
            let mut runtime = shared.sync.lock().unwrap();
            match restored {
                Ok(session) if runtime.account.as_ref().is_some_and(|account| account.user.id == session.user.id) => {
                    runtime.session = Some(session); runtime.needs_login = false;
                }
                Ok(_) => runtime.error = Some("登录凭据与账号存档不匹配，请重新登录。".into()),
                Err(error) => runtime.error = Some(error),
            }
        }
        let mut failures = 0u32;
        loop {
            if shared.exiting.load(std::sync::atomic::Ordering::SeqCst) { break; }
            #[cfg(target_os = "android")]
            if crate::mobile::stop_requested(&handle) { break; }
            match network_step(&handle) {
                Ok(()) => failures = 0,
                Err(error) => {
                    failures = (failures + 1).min(5);
                    shared.sync.lock().unwrap().failed(error);
                    crate::broadcast(&handle);
                }
            }
            let seconds = if failures > 0 { (3u64 << failures).min(60) } else { 3 };
            #[cfg(target_os = "android")]
            let seconds = if crate::mobile::is_suspended(&handle) { seconds.max(30) } else { seconds };
            thread::park_timeout(Duration::from_secs(seconds));
        }
    });
}

fn ready_config(shared: &Shared) -> Result<Config, String> {
    let runtime = shared.sync.lock().unwrap();
    if let Some(error) = &runtime.blocked { return Err(error.clone()); }
    runtime.config.clone().ok_or("此版本尚未配置云同步，请先完成云端部署。".into())
}

#[tauri::command]
pub async fn sync_send_code(email: String, app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        let config = ready_config(&shared)?;
        {
            let runtime = shared.sync.lock().unwrap();
            let now = crate::sync_clock::continuous_seconds().unwrap_or(crate::now_unix().max(0) as u64);
            if runtime.last_code_sent.is_some_and(|when| now.saturating_sub(when) < 60) {
                return Err("验证码已发送，请稍后再试。".into());
            }
            if runtime.account.is_some() && !runtime.needs_login { return Err("请先退出当前账号。".into()); }
        }
        Http::new(config)?.send_code(&email)?;
        shared.sync.lock().unwrap().last_code_sent = Some(crate::sync_clock::continuous_seconds().unwrap_or(crate::now_unix().max(0) as u64));
        Ok(())
    }).await.map_err(|_| "验证码请求未完成，请重试。".to_owned())?
}

#[tauri::command]
pub async fn sync_verify_code(email: String, code: String, app: AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        let http = Http::new(ready_config(&shared)?)?;
        let session = http.verify(&email, &code, crate::now_unix())?;
        let existing = {
            let runtime = shared.sync.lock().unwrap();
            if runtime.account.as_ref().is_some_and(|account| account.user.id != session.user.id) {
                return Err("请先退出当前账号，再登录另一个账号。".into());
            }
            runtime.store.as_ref().ok_or("同步存储未就绪。")?.load(&session.user)?
        };
        if existing.is_some() || shared.sync.lock().unwrap().account.is_some() {
            let _switch = AccountTransition::begin(&shared)?;
            save_before_switch(&shared)?;
            let scope = shared.sync.lock().unwrap().store.as_ref().ok_or("同步存储未就绪。")?.scope.clone();
            crate::sync_credentials::store(&app, &shared.data_dir, &scope,
                &serde_json::to_string(&session).map_err(|_| "登录会话编码失败。")?)?;
            let mut state = shared.state.lock().unwrap();
            let mut runtime = shared.sync.lock().unwrap();
            if runtime.account.is_none() {
                let mut account = existing.ok_or("账号缓存缺失。")?;
                account.user = session.user.clone();
                let mut restored = account.state.clone();
                runtime.clock_offset = account.clock_offset;
                runtime.clock_anchor = None;
                account.draft.apply(&mut restored, runtime.now())?;
                inherit_device_settings(&state, &mut restored);
                account.state = restored.clone();
                runtime.store.as_ref().unwrap().select(Some(&session.user))?;
                runtime.account = Some(account);
                *state = restored;
            }
            if let Some(account) = &mut runtime.account { account.user = session.user.clone(); }
            runtime.session = Some(session);
            runtime.candidate = None;
            runtime.import_remote = None;
            runtime.needs_login = false;
            runtime.error = None;
            runtime.wake();
        } else {
            let reply = http.read(&session, -1)?;
            let mut runtime = shared.sync.lock().unwrap();
            runtime.calibrate(reply.server_time);
            runtime.import_remote = reply.document.map(|document| RemoteVersion { revision: reply.revision, document, server_time: reply.server_time });
            runtime.candidate = Some(session);
            runtime.error = None;
        }
        crate::save(&shared);
        notify_changed(&app, true);
        Ok(crate::snapshot(&shared, true))
    }).await.map_err(|_| "登录处理未完成，请重试。".to_owned())?
}

#[tauri::command]
pub async fn sync_connect(strategy: String, app: AppHandle) -> Result<Snapshot, String> {
    if !matches!(strategy.as_str(), "merge_local" | "merge_remote" | "cloud_only") { return Err("请选择有效的迁移方式。".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        crate::save(&shared);
        if shared.save_error.lock().unwrap().is_some() || shared.write_protected.lock().unwrap().is_some() {
            return Err("请先修复本机存档保存问题，再迁移到账号。".into());
        }
        let (session, scope) = {
            let runtime = shared.sync.lock().unwrap();
            (runtime.candidate.clone().ok_or("请先验证邮箱。")?, runtime.store.as_ref().ok_or("同步存储未就绪。")?.scope.clone())
        };
        crate::sync_credentials::store(&app, &shared.data_dir, &scope,
            &serde_json::to_string(&session).map_err(|_| "登录会话编码失败。")?)?;
        let _switch = AccountTransition::begin(&shared)?;
        save_before_switch(&shared)?;
        {
            let mut state = shared.state.lock().unwrap();
            let mut runtime = shared.sync.lock().unwrap();
            let now = runtime.now();
            let mut empty = core::State::new(now);
            let empty = SyncDocument::capture(&mut empty, None, &new_id()?, &[])?;
            let (revision, remote) = runtime.import_remote.as_ref().map(|remote| (remote.revision, remote.document.clone())).unwrap_or((0, empty.clone()));
            let mut next_state = state.clone();
            let draft = if strategy == "cloud_only" { remote.clone() } else {
                rebase_local_day(&mut next_state, now.saturating_sub(crate::now_unix()))?;
                let local = SyncDocument::capture(&mut next_state, Some(&remote), &new_id()?, &[])?;
                merge(&empty, &local, &remote, now, Some(if strategy == "merge_local" { MergeChoice::Local } else { MergeChoice::Remote }))
                    .map_err(|_| "本机与云端数据无法安全合并，原数据已保留。")?
            };
            draft.apply(&mut next_state, now)?;
            let store = runtime.store.as_ref().ok_or("同步存储未就绪。")?;
            let backup = store.backup("migration", &json!({"local":&*state,"cloud":remote,"account_id":session.user.id}))?;
            let account = AccountFile { schema: 1, user: session.user.clone(), state: next_state.clone(), revision, clock_offset: now.saturating_sub(crate::now_unix()),
                base: remote, draft, pending: None, conflict: None };
            store.save(&account, &next_state)?;
            store.select(Some(&session.user))?;
            runtime.account = Some(account);
            runtime.session = Some(session);
            runtime.candidate = None;
            runtime.import_remote = None;
            runtime.needs_login = false;
            runtime.online = true;
            runtime.error = None;
            runtime.recovery_path = Some(backup.display().to_string());
            *state = next_state;
            runtime.wake();
        }
        notify_changed(&app, true);
        Ok(crate::snapshot(&shared, true))
    }).await.map_err(|_| "迁移处理未完成，请重试。".to_owned())?
}

#[tauri::command]
pub fn sync_retry(app: AppHandle) -> Snapshot {
    let shared = app.state::<Shared>();
    shared.sync.lock().unwrap().wake();
    crate::snapshot(&shared, true)
}

#[tauri::command]
pub async fn sync_resolve(choice: String, app: AppHandle) -> Result<Snapshot, String> {
    let choice = match choice.as_str() { "local" => MergeChoice::Local, "remote" => MergeChoice::Remote, _ => return Err("请选择本机或云端版本。".into()) };
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        {
            let mut state = shared.state.lock().unwrap();
            let mut runtime = shared.sync.lock().unwrap();
            let now = runtime.now();
            let account = runtime.account.as_ref().ok_or("尚未登录。")?;
            let remote = account.conflict.clone().ok_or("当前没有需要处理的冲突。")?;
            let merged = merge(&account.base, &account.draft, &remote.document, now, Some(choice))
                .map_err(|_| "这两份记录仍无法安全合并，原数据已保留。")?;
            let backup = runtime.store.as_ref().ok_or("同步存储未就绪。")?.backup("conflict",
                &json!({"local":account,"local_live_state":&*state,"remote":remote}))?;
            let mut next_state = state.clone();
            merged.apply(&mut next_state, now)?;
            let mut next = account.clone();
            next.clock_offset = now.saturating_sub(crate::now_unix());
            next.base = remote.document;
            next.revision = remote.revision;
            next.draft = merged;
            next.pending = None;
            next.conflict = None;
            runtime.store.as_ref().unwrap().save(&next, &next_state)?;
            runtime.account = Some(next);
            runtime.error = None;
            runtime.recovery_path = Some(backup.display().to_string());
            *state = next_state;
            runtime.wake();
        }
        notify_changed(&app, true);
        Ok(crate::snapshot(&shared, true))
    }).await.map_err(|_| "冲突处理未完成，请重试。".to_owned())?
}

#[tauri::command]
pub async fn sync_sign_out(app: AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        crate::save(&shared);
        if shared.save_error.lock().unwrap().is_some() { return Err("请先保存本机记录，再退出登录。".into()); }
        let (scope, session, config) = {
            let runtime = shared.sync.lock().unwrap();
            (runtime.store.as_ref().ok_or("同步存储未就绪。")?.scope.clone(), runtime.session.clone(), runtime.config.clone())
        };
        // 离线也能在本机退出；即使远端撤销失败，本机也必须清除自己的凭据与自动登录标记。
        if let (Some(config), Some(session)) = (config, &session) { let _ = Http::new(config)?.logout(session); }
        let _switch = AccountTransition::begin(&shared)?;
        save_before_switch(&shared)?;
        crate::sync_credentials::delete(&app, &shared.data_dir, &scope)?;
        {
            let mut state = shared.state.lock().unwrap();
            let mut runtime = shared.sync.lock().unwrap();
            runtime.store.as_ref().unwrap().select(None)?;
            let (mut local, protection) = crate::load_initial(&shared.path);
            inherit_device_settings(&state, &mut local);
            runtime.account = None;
            runtime.session = None;
            runtime.candidate = None;
            runtime.import_remote = None;
            runtime.needs_login = false;
            runtime.online = false;
            runtime.clock_offset = 0;
            runtime.clock_anchor = None;
            runtime.last_synced_at = None;
            runtime.error = None;
            runtime.recovery_path = None;
            *state = local;
            *shared.write_protected.lock().unwrap() = protection;
        }
        crate::save(&shared);
        notify_changed(&app, true);
        Ok(crate::snapshot(&shared, true))
    }).await.map_err(|_| "退出登录未完成，请重试。".to_owned())?
}

#[tauri::command]
pub async fn sync_cancel_sign_in(app: AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = app.state::<Shared>();
        let _network = shared.sync_network.lock().unwrap();
        {
            let mut runtime = shared.sync.lock().unwrap();
            runtime.candidate = None;
            runtime.import_remote = None;
            if !runtime.active() { runtime.clock_offset = 0; runtime.clock_anchor = None; }
        }
        Ok(crate::snapshot(&shared, true))
    }).await.map_err(|_| "取消登录未完成，请重试。".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(now: i64) -> SyncDocument {
        let mut state = core::State::new(now);
        SyncDocument::capture(&mut state, None, &"a".repeat(32), &[]).unwrap()
    }

    fn machine(base: SyncDocument) -> (Runtime, core::State) {
        let mut state = core::State::new(base.last_tick);
        base.apply(&mut state, base.last_tick).unwrap();
        let mut runtime = Runtime::disabled();
        runtime.clock_offset = base.last_tick - crate::now_unix();
        runtime.account = Some(AccountFile { schema: 1,
            user: crate::sync_http::AuthUser { id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(), email: "a@example.com".into() },
            state: state.clone(), revision: 1, clock_offset: runtime.clock_offset, base: base.clone(), draft: base, pending: None, conflict: None });
        (runtime, state)
    }

    #[test]
    fn acknowledgment_keeps_edits_made_while_request_was_in_flight() {
        let (mut runtime, mut state) = machine(document(1000));
        state.preferences.break_minutes = 5;
        runtime.changed(&mut state, &[]).unwrap();
        let first = runtime.prepare_write().unwrap().unwrap();
        state.preferences.uniform_block_minutes = 40;
        runtime.changed(&mut state, &[]).unwrap();
        assert_eq!(runtime.prepare_write().unwrap().unwrap().operation_id, first.operation_id);
        runtime.acknowledge(&first, WriteReply { applied: true, revision: 2, committed_revision: Some(2),
            document: Some(first.document.clone()), server_time: 1000 }, &mut state).unwrap();
        assert_eq!(state.preferences.break_minutes, 5);
        assert_eq!(state.preferences.uniform_block_minutes, 40);
        let second = runtime.prepare_write().unwrap().unwrap();
        assert_ne!(first.operation_id, second.operation_id);
        assert_eq!(second.expected_revision, 2);
    }

    #[test]
    fn both_devices_mutating_the_same_timer_require_a_choice() {
        let mut initial = core::State::new(1000);
        initial.start_day("standard", 1000).unwrap();
        initial.start_block_with_break("main", 30, vec![], 5).unwrap();
        let base = SyncDocument::capture(&mut initial, None, &"a".repeat(32), &[]).unwrap();
        let (mut runtime, mut local) = machine(base.clone());
        local.tick_wall_clock(1060);
        local.toggle_pause(1060).unwrap();
        runtime.changed(&mut local, &[]).unwrap();
        let sent = runtime.prepare_write().unwrap().unwrap();
        let mut other = initial.clone();
        other.tick_wall_clock(1060);
        other.extend_block(10).unwrap();
        let remote = SyncDocument::capture(&mut other, Some(&base), &"b".repeat(32), &[]).unwrap();
        runtime.acknowledge(&sent, WriteReply { applied: false, revision: 2, committed_revision: None,
            document: Some(remote), server_time: 1060 }, &mut local).unwrap();
        assert!(runtime.account.as_ref().unwrap().conflict.is_some());
        assert!(local.day.as_ref().unwrap().is_paused());
        assert!(runtime.prepare_write().unwrap().is_none());
    }

    #[test]
    fn a_phone_can_pause_resume_and_finish_the_same_mac_timer_once() {
        let base = document(1000);
        let (mut mac, mut computer) = machine(base.clone());
        let (mut phone, mut mobile) = machine(base);
        computer.start_day("standard", 1000).unwrap();
        computer.start_block_with_break("main", 50, vec![], 5).unwrap();
        mac.changed(&mut computer, &[]).unwrap();
        let started = mac.prepare_write().unwrap().unwrap();
        mac.acknowledge(&started, WriteReply { applied: true, revision: 2, committed_revision: Some(2),
            document: Some(started.document.clone()), server_time: 1000 }, &mut computer).unwrap();
        phone.calibrate(2200);
        phone.accept_remote(RemoteVersion { revision: 2, document: started.document, server_time: 2200 }, &mut mobile).unwrap();
        assert_eq!(mobile.day.as_ref().unwrap().timer.as_ref().unwrap().remaining_seconds(), 1800);
        mobile.toggle_pause(2200).unwrap();
        phone.changed(&mut mobile, &[]).unwrap();
        let paused = phone.prepare_write().unwrap().unwrap();
        phone.acknowledge(&paused, WriteReply { applied: true, revision: 3, committed_revision: Some(3),
            document: Some(paused.document.clone()), server_time: 2200 }, &mut mobile).unwrap();
        mac.calibrate(2800);
        mac.accept_remote(RemoteVersion { revision: 3, document: paused.document, server_time: 2800 }, &mut computer).unwrap();
        assert!(computer.day.as_ref().unwrap().is_paused());
        assert_eq!(computer.day.as_ref().unwrap().timer.as_ref().unwrap().elapsed_seconds, 1200);
        computer.toggle_pause(2800).unwrap();
        mac.changed(&mut computer, &[]).unwrap();
        let resumed = mac.prepare_write().unwrap().unwrap();
        mac.acknowledge(&resumed, WriteReply { applied: true, revision: 4, committed_revision: Some(4),
            document: Some(resumed.document.clone()), server_time: 2800 }, &mut computer).unwrap();
        phone.calibrate(3800);
        phone.accept_remote(RemoteVersion { revision: 4, document: resumed.document, server_time: 3800 }, &mut mobile).unwrap();
        mobile.finish_block(3800).unwrap();
        phone.changed(&mut mobile, &[]).unwrap();
        let finished = phone.prepare_write().unwrap().unwrap();
        phone.acknowledge(&finished, WriteReply { applied: true, revision: 5, committed_revision: Some(5),
            document: Some(finished.document.clone()), server_time: 3800 }, &mut mobile).unwrap();
        mac.calibrate(3800);
        mac.accept_remote(RemoteVersion { revision: 5, document: finished.document, server_time: 3800 }, &mut computer).unwrap();
        for state in [&computer, &mobile] {
            let day = state.day.as_ref().unwrap();
            assert_eq!(day.ledger.len(), 1);
            assert_eq!(day.ledger[0].seconds, 2200);
            assert_eq!(day.net_seconds(), 2200);
        }
        assert_eq!(computer.day.unwrap().ledger[0].sync_id, mobile.day.unwrap().ledger[0].sync_id);
    }

    #[test]
    fn calibration_rebuilds_provisional_ticks_instead_of_recounting_them() {
        let mut initial = core::State::new(1000);
        initial.start_day("standard", 1000).unwrap();
        initial.start_block_with_break("main", 2, vec![], 5).unwrap();
        let base = SyncDocument::capture(&mut initial, None, &"a".repeat(32), &[]).unwrap();
        let (mut runtime, mut state) = machine(base);
        state.tick_wall_clock(1600);
        assert!(state.day.as_ref().unwrap().timer.is_none());
        runtime.calibrate(1060);
        runtime.reproject(&mut state).unwrap();
        assert_eq!(state.day.as_ref().unwrap().timer.as_ref().unwrap().elapsed_seconds, 60);
        assert!(state.day.as_ref().unwrap().ledger.is_empty());
        runtime.calibrate(1050);
        runtime.reproject(&mut state).unwrap();
        runtime.calibrate(1060);
        runtime.reproject(&mut state).unwrap();
        assert_eq!(state.day.as_ref().unwrap().timer.as_ref().unwrap().elapsed_seconds, 60);
        assert!(runtime.prepare_write().unwrap().is_none(), "校时与自然走时不能产生新的业务修改");
    }

    #[test]
    fn pending_operations_and_clock_calibration_survive_restart_without_crossing_accounts() {
        let directory = std::env::temp_dir().join(format!("sitz-sync-restart-{}", new_id().unwrap()));
        std::fs::create_dir(&directory).unwrap();
        let store = Store::open(&directory, "https://synthetic.example.com").unwrap();
        let (mut runtime, mut state) = machine(document(1000));
        state.preferences.break_minutes = 7;
        runtime.changed(&mut state, &[]).unwrap();
        let sent = runtime.prepare_write().unwrap().unwrap();
        state.preferences.uniform_block_minutes = 45;
        runtime.changed(&mut state, &[]).unwrap();
        runtime.calibrate(1000);
        let first = runtime.account.as_ref().unwrap().clone();
        store.save(&first, &state).unwrap();
        let mut second = first.clone();
        second.user.id = "ffffffff-bbbb-cccc-dddd-eeeeeeeeeeee".into();
        second.user.email = "b@example.com".into();
        second.draft.preferences.break_minutes = 12;
        second.pending = None;
        store.save(&second, &state).unwrap();
        let restored = store.load(&first.user).unwrap().unwrap();
        assert_eq!(restored.clock_offset, first.clock_offset);
        assert_eq!(restored.pending.as_ref().unwrap().operation_id, sent.operation_id);
        assert_eq!(restored.draft.preferences.uniform_block_minutes, 45);
        assert_eq!(store.load(&second.user).unwrap().unwrap().draft.preferences.break_minutes, 12);
        runtime.account = Some(restored);
        runtime.acknowledge(&sent, WriteReply { applied: true, revision: 2, committed_revision: Some(2),
            document: Some(sent.document.clone()), server_time: 1000 }, &mut state).unwrap();
        assert_eq!(state.preferences.uniform_block_minutes, 45);
        assert!(runtime.prepare_write().unwrap().is_some());
        let bytes = std::fs::read_to_string(store.account_path(&first.user.id).unwrap()).unwrap();
        assert!(!bytes.contains("access_token") && !bytes.contains("refresh_token"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn switching_accounts_refuses_unsaved_changes_and_keeps_device_preferences() {
        let fixture = crate::tests::TestShared::new(core::State::new(1000));
        let shared = &fixture.0;
        crate::save(shared);
        let before = std::fs::read(&shared.path).unwrap();
        std::fs::create_dir(shared.path.with_extension("json.tmp")).unwrap();
        shared.state.lock().unwrap().preferences.break_minutes = 7;
        {
            let _guard = AccountTransition::begin(shared).unwrap();
            assert!(save_before_switch(shared).is_err());
            assert!(shared.switching_account.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(!shared.switching_account.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(shared.state.lock().unwrap().preferences.break_minutes, 7);
        assert_eq!(std::fs::read(&shared.path).unwrap(), before);
        let mut source = core::State::new(1000);
        source.preferences.blocked_hosts = vec!["example.com".into()];
        source.preferences.sound_enabled = false;
        let mut destination = core::State::new(1000);
        destination.preferences.break_minutes = 9;
        inherit_device_settings(&source, &mut destination);
        assert_eq!(destination.preferences.break_minutes, 9);
        assert_eq!(destination.preferences.blocked_hosts, ["example.com"]);
        assert!(!destination.preferences.sound_enabled);
    }

    #[test]
    fn old_confirmation_cannot_mutate_a_new_timer_but_own_ack_does_not_invalidate_it() {
        let mut initial = core::State::new(1000);
        initial.start_day("standard", 1000).unwrap();
        initial.start_block_with_break("main", 30, vec![], 5).unwrap();
        let base = SyncDocument::capture(&mut initial, None, &"a".repeat(32), &[]).unwrap();
        let (mut runtime, mut state) = machine(base);
        let day = state.day.as_ref().unwrap();
        let expected = ActionTarget { revision: 1, account_id: Some(runtime.account.as_ref().unwrap().user.id.clone()), day_id: Some(day.sync_id.clone()),
            timer_id: Some(day.timer.as_ref().unwrap().sync_id.clone()), paused: false, break_until: None };
        assert!(runtime.check_target(&state, Some(&expected), &TargetCheck::Live).is_ok());
        runtime.account.as_mut().unwrap().revision = 2;
        assert!(runtime.check_target(&state, Some(&expected), &TargetCheck::Live).is_ok(), "自己的上传确认不改变操作对象");
        assert!(runtime.check_target(&state, Some(&expected), &TargetCheck::Preferences).is_err(), "整份偏好编辑必须检查基线");
        state.day.as_mut().unwrap().timer.as_mut().unwrap().sync_id = "different-timer".into();
        assert!(runtime.check_target(&state, Some(&expected), &TargetCheck::Live).is_err());
        assert!(runtime.check_target(&state, None, &TargetCheck::Live).is_err());
        runtime.account = None;
        assert!(runtime.check_target(&state, Some(&expected), &TargetCheck::Identity).is_err(), "旧账号操作不能落到退出后的单机数据");
    }

    #[test]
    fn importing_an_active_local_day_translates_anchors_without_changing_earned_seconds() {
        let mut local = core::State::new(1300);
        local.start_day("standard", 1300).unwrap();
        local.start_block_with_break("main", 20, vec![], 5).unwrap();
        local.tick_wall_clock(1360);
        let original = core::to_json(&local);
        let mut imported = local.clone();
        rebase_local_day(&mut imported, -300).unwrap();
        let document = SyncDocument::capture(&mut imported, None, &"a".repeat(32), &[]).unwrap();
        let mut phone = core::State::new(1060);
        document.apply(&mut phone, 1060).unwrap();
        assert_eq!(phone.day.as_ref().unwrap().started_at, 1000);
        assert_eq!(phone.day.as_ref().unwrap().timer.as_ref().unwrap().elapsed_seconds, 60);
        assert_eq!(core::to_json(&local), original, "迁移只操作副本，原单机数据不被校时改写");
    }
}
