//! 跨设备共享数据的纯逻辑。网络、账号和磁盘属于外壳；这里不产生任何副作用。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{ArchivedDay, CategoryDef, Day, Preferences, ProfileDef, State, TaskItem};

pub const SYNC_SCHEMA: u32 = 1;
pub const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HISTORY_DAYS: usize = 1000;
pub const MAX_DELETED_DAYS: usize = 10_000;
pub const MAX_UNIX_SECONDS: i64 = 253_402_300_799;
const MAX_DAY_CATEGORIES: usize = 128;
const MAX_LEDGER_ENTRIES: usize = 10_000;
const MAX_SPANS: usize = 20_000;

/// 白名单字段保证网站/应用屏蔽、声音、系统权限和提醒偏好不会搭便车上传。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedPreferences {
    pub categories: Vec<CategoryDef>,
    pub profiles: Vec<ProfileDef>,
    pub break_minutes: i64,
    pub uniform_block_minutes: i64,
    pub default_profile_id: String,
}

impl SharedPreferences {
    pub fn capture(preferences: &Preferences) -> Self {
        Self {
            categories: preferences.categories.clone(),
            profiles: preferences.profiles.clone(),
            break_minutes: preferences.break_minutes,
            uniform_block_minutes: preferences.uniform_block_minutes,
            default_profile_id: preferences.default_profile_id.clone(),
        }
    }

    pub fn apply(&self, preferences: &mut Preferences) {
        preferences.categories = self.categories.clone();
        preferences.profiles = self.profiles.clone();
        preferences.break_minutes = self.break_minutes;
        preferences.uniform_block_minutes = self.uniform_block_minutes;
        preferences.default_profile_id = self.default_profile_id.clone();
    }

    fn neutral_preferences(&self) -> Preferences {
        let mut preferences = Preferences::default();
        self.apply(&mut preferences);
        preferences
    }

    fn validate(&self) -> Result<(), String> {
        crate::validate_preferences(&self.neutral_preferences()).map_err(str::to_string)?;
        let mut profiles = BTreeSet::new();
        for category in &self.categories {
            user_id(&category.id, false)?;
            text(&category.name, 80, false, true)?;
            text(&category.short_name, 32, false, false)?;
            text(&category.icon, 64, false, false)?;
            text(&category.role, 64, false, false)?;
            text(&category.block_rationale, 1000, true, false)?;
        }
        for profile in &self.profiles {
            user_id(&profile.id, false)?;
            if !profiles.insert(&profile.id) {
                return Err("同步计划标识重复".into());
            }
            text(&profile.name, 80, false, true)?;
            text(&profile.subtitle, 1000, true, false)?;
            if profile.quotas.len() > 64 {
                return Err("同步计划中的项目过多".into());
            }
            for quota in &profile.quotas {
                user_id(&quota.category, false)?;
            }
        }
        user_id(&self.default_profile_id, true)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SyncDocument {
    pub schema: u32,
    pub preferences: SharedPreferences,
    pub last_tick: i64,
    pub day: Option<Day>,
    pub history: Vec<ArchivedDay>,
    pub deleted_day_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDocument {
    schema: u32,
    preferences: SharedPreferences,
    last_tick: i64,
    day: Option<Day>,
    history: Vec<ArchivedDay>,
    deleted_day_ids: Vec<String>,
}

impl<'de> Deserialize<'de> for SyncDocument {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // 本机旧存档允许兼容字段；网络协议必须另走严格入口，不能用 serde 的忽略未知字段吞掉新协议。
        let value = Value::deserialize(deserializer)?;
        strict_keys(&value).map_err(serde::de::Error::custom)?;
        let raw: RawDocument = serde_json::from_value(value)
            .map_err(|_| serde::de::Error::custom("同步数据字段类型无效"))?;
        let document = Self {
            schema: raw.schema,
            preferences: raw.preferences,
            last_tick: raw.last_tick,
            day: raw.day,
            history: raw.history,
            deleted_day_ids: raw.deleted_day_ids,
        };
        document.validate().map_err(serde::de::Error::custom)?;
        Ok(document)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeChoice {
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MergeConflict {
    pub fields: Vec<String>,
}

impl std::fmt::Display for MergeConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "共享数据需要处理冲突：{}",
            self.fields.join("、")
        )
    }
}

impl std::error::Error for MergeConflict {}

impl SyncDocument {
    pub fn from_json(raw: &str) -> Result<Self, String> {
        if raw.len() > MAX_DOCUMENT_BYTES {
            return Err("同步文档超过 2 MiB，原数据已保留".into());
        }
        serde_json::from_str(raw).map_err(|_| "同步数据格式或内容无效".into())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SYNC_SCHEMA {
            return Err("不支持的同步协议版本，请更新应用".into());
        }
        timestamp(self.last_tick)?;
        self.preferences.validate()?;
        if self.history.len() > MAX_HISTORY_DAYS || self.deleted_day_ids.len() > MAX_DELETED_DAYS {
            return Err("共享历史或删除标记达到容量上限，原数据已保留".into());
        }
        let mut ids = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        for id in &self.deleted_day_ids {
            sync_id(id)?;
            if !deleted.insert(id.as_str()) {
                return Err("同步删除标记重复".into());
            }
        }
        if let Some(day) = &self.day {
            validate_day(day, false, &mut ids)?;
            if deleted.contains(day.sync_id.as_str()) {
                return Err("已删除的学习日不能同时存在".into());
            }
        }
        for archived in &self.history {
            timestamp(archived.ended_at)?;
            if archived.ended_at < archived.day.started_at {
                return Err("学习日结束时刻早于开始时刻".into());
            }
            validate_day(&archived.day, true, &mut ids)?;
            if deleted.contains(archived.day.sync_id.as_str()) {
                return Err("已删除的学习日不能同时存在".into());
            }
        }
        if serialized_jsonb_len(self)? > MAX_DOCUMENT_BYTES {
            return Err("同步文档超过 2 MiB，原数据已保留".into());
        }
        Ok(())
    }

    /// 命名空间由外壳的安全随机源产生。先在副本上迁移，校验成功才把标识写回本机状态。
    pub fn capture(
        state: &mut State,
        previous: Option<&Self>,
        namespace: &str,
        deleted_day_ids: &[String],
    ) -> Result<Self, String> {
        if namespace.len() != 32
            || !namespace
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("同步标识命名空间无效".into());
        }
        timestamp(state.last_tick)?;
        if let Some(previous) = previous {
            previous.validate()?;
        }
        let mut migrated = state.clone();
        adopt_known_ids(&mut migrated, previous);
        assign_ids(&mut migrated, previous, namespace)?;
        let mut deleted: BTreeSet<String> = previous
            .into_iter()
            .flat_map(|document| document.deleted_day_ids.iter().cloned())
            .collect();
        for id in deleted_day_ids {
            sync_id(id)?;
            deleted.insert(id.clone());
        }
        let preferences = SharedPreferences::capture(&migrated.preferences);
        let mut history: BTreeMap<String, ArchivedDay> = previous
            .into_iter()
            .flat_map(|document| document.history.iter().cloned())
            .map(|archived| (archived.day.sync_id.clone(), archived))
            .collect();
        for archived in &migrated.history {
            history.insert(archived.day.sync_id.clone(), archived.clone());
        }
        history.retain(|id, _| !deleted.contains(id));
        let mut document = Self {
            schema: SYNC_SCHEMA,
            preferences,
            last_tick: migrated.last_tick,
            day: migrated.day.clone(),
            history: history.into_values().collect(),
            deleted_day_ids: deleted.into_iter().collect(),
        };
        document.normalize();
        document.validate()?;
        *state = migrated;
        Ok(document)
    }

    /// 只替换共享字段。设备偏好、喝水去重游标和当前设备已消费的提醒计数不从远端覆盖。
    pub fn apply(&self, state: &mut State, now: i64) -> Result<(), String> {
        self.validate()?;
        timestamp(now)?;
        let projected = self.projected(now)?;
        let mut updated = state.clone();
        self.preferences.apply(&mut updated.preferences);
        updated.last_tick = projected.last_tick;
        updated.day = projected.day;
        updated.history = projected.history;
        if let (Some(old), Some(new)) = (&state.day, &mut updated.day) {
            if !old.sync_id.is_empty() && old.sync_id == new.sync_id {
                new.seated_since_water = old.seated_since_water;
                new.seated_since_relief = old.seated_since_relief;
                new.paused_without_block = old.paused_without_block;
            }
        }
        *state = updated;
        Ok(())
    }

    /// 投影只推进到更晚的时刻，不让校时回拨将已记过的秒数再记一次。
    pub fn projected(&self, now: i64) -> Result<Self, String> {
        self.validate()?;
        timestamp(now)?;
        let target = now.max(self.last_tick);
        let mut state = State::new(self.last_tick);
        state.preferences = self.preferences.neutral_preferences();
        state.day = self.day.clone();
        state.tick_wall_clock(target);
        state.take_due_break(target);
        let mut projected = self.clone();
        projected.last_tick = target;
        projected.day = state.day;
        projected.normalize();
        projected.validate()?;
        Ok(projected)
    }

    pub fn equivalent(&self, other: &Self, now: i64) -> bool {
        let target = now.max(self.last_tick).max(other.last_tick);
        match (self.projected(target), other.projected(target)) {
            (Ok(left), Ok(right)) => same(&left, &right),
            _ => false,
        }
    }

    pub fn merge(
        base: &Self,
        local: &Self,
        remote: &Self,
        now: i64,
        choice: Option<MergeChoice>,
    ) -> Result<Self, MergeConflict> {
        merge(base, local, remote, now, choice)
    }

    fn normalize(&mut self) {
        let fallback = self.preferences.break_minutes;
        if let Some(day) = &mut self.day {
            normalize_day(day, fallback, self.last_tick);
        }
        for archived in &mut self.history {
            normalize_day(&mut archived.day, fallback, archived.ended_at);
        }
        self.history.sort_by(|a, b| {
            (a.day.started_at, &a.day.sync_id).cmp(&(b.day.started_at, &b.day.sync_id))
        });
        self.deleted_day_ids.sort();
        self.deleted_day_ids.dedup();
    }
}

/// 三方比较先把三个时间锚点投影到同一秒，自然走时不是用户冲突。
pub fn merge(
    base: &SyncDocument,
    local: &SyncDocument,
    remote: &SyncDocument,
    now: i64,
    choice: Option<MergeChoice>,
) -> Result<SyncDocument, MergeConflict> {
    let target = now
        .max(base.last_tick)
        .max(local.last_tick)
        .max(remote.last_tick);
    let project = |document: &SyncDocument| {
        document.projected(target).map_err(|reason| MergeConflict {
            fields: vec![reason],
        })
    };
    let base = project(base)?;
    let local = project(local)?;
    let remote = project(remote)?;
    let mut conflicts = Vec::new();
    let preferences = merge_preferences(
        &base.preferences,
        &local.preferences,
        &remote.preferences,
        choice,
        &mut conflicts,
    );
    let day = pick(
        &base.day,
        &local.day,
        &remote.day,
        "day",
        choice,
        &mut conflicts,
    );
    let base_history = history_map(&base);
    let local_history = history_map(&local);
    let remote_history = history_map(&remote);
    let ids: BTreeSet<&String> = base_history
        .keys()
        .chain(local_history.keys())
        .chain(remote_history.keys())
        .collect();
    let mut history = Vec::new();
    let mut deleted_day_ids = Vec::new();
    for id in ids {
        let before = base_history
            .get(id)
            .cloned()
            .unwrap_or(HistoryRecord::Absent);
        // 没有删除标记的缺项不是删除；旧版设备可能只缓存最近60天。
        let left = local_history.get(id).unwrap_or(&before);
        let right = remote_history.get(id).unwrap_or(&before);
        let picked = pick(
            &before,
            left,
            right,
            &format!("history:{id}"),
            choice,
            &mut conflicts,
        );
        match picked {
            HistoryRecord::Present(archived) => history.push(*archived),
            HistoryRecord::Deleted => deleted_day_ids.push(id.clone()),
            HistoryRecord::Absent => (),
        }
    }
    if !conflicts.is_empty() && choice.is_none() {
        return Err(MergeConflict { fields: conflicts });
    }
    if let Some(active) = &day {
        // 同一天一端收工、另一端继续时，已由 day 的冲突选择决定生命周期，不能两处各记一遍。
        history.retain(|archived| archived.day.sync_id != active.sync_id);
        deleted_day_ids.retain(|id| id != &active.sync_id);
    }
    let mut document = SyncDocument {
        schema: SYNC_SCHEMA,
        preferences,
        last_tick: target,
        day,
        history,
        deleted_day_ids,
    };
    document.normalize();
    document.validate().map_err(|reason| MergeConflict {
        fields: vec![reason],
    })?;
    Ok(document)
}

fn merge_preferences(
    base: &SharedPreferences,
    local: &SharedPreferences,
    remote: &SharedPreferences,
    choice: Option<MergeChoice>,
    conflicts: &mut Vec<String>,
) -> SharedPreferences {
    // 项目与计划有引用约束，作为一个编辑单元选择；时长偏好仍可独立合并。
    let plan = |preferences: &SharedPreferences| {
        (
            preferences.categories.clone(),
            preferences.profiles.clone(),
            preferences.default_profile_id.clone(),
        )
    };
    let (categories, profiles, default_profile_id) = pick(
        &plan(base),
        &plan(local),
        &plan(remote),
        "preferences.plan",
        choice,
        conflicts,
    );
    SharedPreferences {
        categories,
        profiles,
        default_profile_id,
        break_minutes: pick(
            &base.break_minutes,
            &local.break_minutes,
            &remote.break_minutes,
            "preferences.break_minutes",
            choice,
            conflicts,
        ),
        uniform_block_minutes: pick(
            &base.uniform_block_minutes,
            &local.uniform_block_minutes,
            &remote.uniform_block_minutes,
            "preferences.uniform_block_minutes",
            choice,
            conflicts,
        ),
    }
}

#[derive(Clone, Serialize)]
enum HistoryRecord {
    Present(Box<ArchivedDay>),
    Deleted,
    Absent,
}

fn history_map(document: &SyncDocument) -> BTreeMap<String, HistoryRecord> {
    let mut records: BTreeMap<_, _> = document
        .history
        .iter()
        .map(|archived| {
            (
                archived.day.sync_id.clone(),
                HistoryRecord::Present(Box::new(archived.clone())),
            )
        })
        .collect();
    for id in &document.deleted_day_ids {
        records.insert(id.clone(), HistoryRecord::Deleted);
    }
    records
}

fn pick<T: Clone + Serialize>(
    base: &T,
    local: &T,
    remote: &T,
    field: &str,
    choice: Option<MergeChoice>,
    conflicts: &mut Vec<String>,
) -> T {
    if same(local, remote) || same(remote, base) {
        local.clone()
    } else if same(local, base) {
        remote.clone()
    } else {
        conflicts.push(field.into());
        match choice {
            Some(MergeChoice::Local) => local.clone(),
            Some(MergeChoice::Remote) | None => remote.clone(),
        }
    }
}

fn same<T: Serialize>(left: &T, right: &T) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

/// 与 PostgreSQL `octet_length(document::text)` 对齐。本协议只有整数，JSONB 不涉及浮点规范化。
/// JSONB 在结构逗号、对象冒号后加空格；字符串内容里的逗号/冒号不能被误算。
pub fn serialized_jsonb_len(value: &impl Serialize) -> Result<usize, String> {
    let value = serde_json::to_value(value).map_err(|_| "同步文档无法序列化")?;
    jsonb_value_len(&value)
}

fn jsonb_value_len(value: &Value) -> Result<usize, String> {
    fn add(left: usize, right: usize) -> Result<usize, String> {
        left.checked_add(right)
            .ok_or_else(|| "同步文档长度超出允许范围".into())
    }
    fn string_len(value: &str) -> Result<usize, String> {
        // PostgreSQL JSONB 无法存储 U+0000；其他允许的转义字符仍按 JSON 字节数计算。
        if value.contains('\0') {
            return Err("同步文本不能含有空字符".into());
        }
        serde_json::to_vec(value)
            .map(|encoded| encoded.len())
            .map_err(|_| "同步文本无法序列化".into())
    }
    match value {
        Value::Null => Ok(4),
        Value::Bool(true) => Ok(4),
        Value::Bool(false) => Ok(5),
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(number.to_string().len()),
        Value::Number(_) => Err("同步文档不能包含非整数数值".into()),
        Value::String(value) => string_len(value),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_fold(2, |length, (index, item)| {
                let length = add(length, usize::from(index > 0) * 2)?;
                add(length, jsonb_value_len(item)?)
            }),
        Value::Object(object) => {
            object
                .iter()
                .enumerate()
                .try_fold(2, |length, (index, (key, item))| {
                    let length = add(length, usize::from(index > 0) * 2)?;
                    let length = add(length, string_len(key)?)?;
                    let length = add(length, 2)?;
                    add(length, jsonb_value_len(item)?)
                })
        }
    }
}

fn normalize_day(day: &mut Day, fallback_break: i64, now: i64) {
    day.seated_since_water = 0;
    day.seated_since_relief = 0;
    day.paused_without_block = 0;
    if day.break_until.is_some_and(|until| until <= now) {
        day.break_until = None;
    }
    if let Some(timer) = &mut day.timer {
        if timer.break_minutes == -1 {
            timer.break_minutes = fallback_break;
        }
        if timer.started_at == 0 {
            timer.started_at = now
                .saturating_sub(timer.elapsed_seconds)
                .max(day.started_at);
        }
    }
    for entry in &mut day.ledger {
        if entry.started_at == 0 {
            entry.started_at = entry
                .ended_at
                .saturating_sub(entry.seconds)
                .max(day.started_at);
        }
    }
}

fn adopt_known_ids(state: &mut State, previous: Option<&SyncDocument>) {
    let Some(previous) = previous else { return };
    for archived in &mut state.history {
        if archived.day.sync_id.is_empty() {
            if let Some(known) = previous.history.iter().find(|known| {
                known.ended_at == archived.ended_at
                    && same_without_ids(&known.day, &archived.day, archived.ended_at)
            }) {
                copy_ids(&mut archived.day, &known.day);
            }
        }
    }
    if let Some(day) = &mut state.day {
        if day.sync_id.is_empty() {
            if let Ok(projected) = previous.projected(state.last_tick) {
                if let Some(known) = &projected.day {
                    if same_without_ids(day, known, projected.last_tick) {
                        copy_ids(day, known);
                    }
                }
            }
        }
    }
}

fn same_without_ids(left: &Day, right: &Day, now: i64) -> bool {
    let strip = |day: &Day| {
        let mut stripped = day.clone();
        normalize_day(&mut stripped, 0, now);
        stripped.sync_id.clear();
        if let Some(timer) = &mut stripped.timer {
            timer.sync_id.clear();
        }
        for entry in &mut stripped.ledger {
            entry.sync_id.clear();
        }
        stripped
    };
    same(&strip(left), &strip(right))
}

fn copy_ids(day: &mut Day, known: &Day) {
    day.sync_id.clone_from(&known.sync_id);
    if let (Some(timer), Some(known_timer)) = (&mut day.timer, &known.timer) {
        timer.sync_id.clone_from(&known_timer.sync_id);
    }
    for (entry, known_entry) in day.ledger.iter_mut().zip(&known.ledger) {
        entry.sync_id.clone_from(&known_entry.sync_id);
    }
}

fn assign_ids(
    state: &mut State,
    previous: Option<&SyncDocument>,
    namespace: &str,
) -> Result<(), String> {
    if state.history.len() > MAX_HISTORY_DAYS {
        return Err("共享历史达到容量上限，原数据已保留".into());
    }
    let mut used = BTreeSet::new();
    for day in state
        .day
        .iter()
        .chain(state.history.iter().map(|archived| &archived.day))
    {
        if day.ledger.len() > MAX_LEDGER_ENTRIES {
            return Err("单个学习日的记录过多".into());
        }
        for id in std::iter::once(&day.sync_id)
            .chain(day.timer.iter().map(|timer| &timer.sync_id))
            .chain(day.ledger.iter().map(|entry| &entry.sync_id))
        {
            if !id.is_empty() && !used.insert(id.clone()) {
                return Err("同步记录标识重复".into());
            }
        }
    }
    if let Some(previous) = previous {
        used.extend(previous.deleted_day_ids.iter().cloned());
        for day in previous
            .day
            .iter()
            .chain(previous.history.iter().map(|archived| &archived.day))
        {
            used.insert(day.sync_id.clone());
            used.extend(day.timer.iter().map(|timer| timer.sync_id.clone()));
            used.extend(day.ledger.iter().map(|entry| entry.sync_id.clone()));
        }
    }
    let mut next = 0_u64;
    let mut assign = |id: &mut String, kind: &str| {
        if id.is_empty() {
            loop {
                let candidate = format!("{namespace}-{kind}-{next}");
                next += 1;
                if used.insert(candidate.clone()) {
                    *id = candidate;
                    break;
                }
            }
        }
    };
    for day in state
        .day
        .iter_mut()
        .chain(state.history.iter_mut().map(|archived| &mut archived.day))
    {
        assign(&mut day.sync_id, "d");
        if let Some(timer) = &mut day.timer {
            assign(&mut timer.sync_id, "b");
        }
        for entry in &mut day.ledger {
            assign(&mut entry.sync_id, "e");
        }
    }
    Ok(())
}

fn timestamp(value: i64) -> Result<(), String> {
    if !(0..=MAX_UNIX_SECONDS).contains(&value) {
        return Err("同步时间戳超出允许范围".into());
    }
    Ok(())
}

fn counter(value: i64) -> Result<(), String> {
    if !(0..=crate::MAX_TRACKED_SECONDS).contains(&value) {
        return Err("同步累计时长超出允许范围".into());
    }
    Ok(())
}

fn user_id(value: &str, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.is_empty())
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        return Err("同步项目或计划标识无效".into());
    }
    Ok(())
}

pub(crate) fn sync_id(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if !(34..=128).contains(&bytes.len())
        || bytes[32] != b'-'
        || !bytes[..32]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        || !bytes[33..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err("同步记录标识无效".into());
    }
    Ok(())
}

fn text(value: &str, maximum: usize, multiline: bool, required: bool) -> Result<(), String> {
    if value.len() > maximum * 4
        || value.chars().count() > maximum
        || (required && value.trim().is_empty())
        || value.chars().any(|character| {
            character.is_control() && !(multiline && matches!(character, '\n' | '\r' | '\t'))
        })
    {
        return Err("同步文本为空、过长或含不支持的控制字符".into());
    }
    Ok(())
}

fn tasks(tasks: &[TaskItem]) -> Result<(), String> {
    if tasks.len() > 50 {
        return Err("同步任务数量过多".into());
    }
    for task in tasks {
        text(&task.text, 500, true, true)?;
    }
    Ok(())
}

fn validate_day<'a>(
    day: &'a Day,
    archived: bool,
    ids: &mut BTreeSet<&'a str>,
) -> Result<(), String> {
    sync_id(&day.sync_id)?;
    if !ids.insert(&day.sync_id) {
        return Err("同步记录标识重复".into());
    }
    timestamp(day.started_at)?;
    user_id(&day.profile_id, true)?;
    text(&day.profile_name, 80, false, true)?;
    if day.categories.is_empty()
        || day.categories.len() > MAX_DAY_CATEGORIES
        || day.ledger.len() > MAX_LEDGER_ENTRIES
        || day.pauses.len() > MAX_SPANS
        || day.rests.len() > MAX_SPANS
    {
        return Err("同步学习日的项目、记录或时间段数量无效".into());
    }
    for value in [
        day.seated_seconds,
        day.paused_seconds,
        day.suspend_seconds,
        day.cups,
    ] {
        counter(value)?;
    }
    if day.suspend_seconds > day.paused_seconds {
        return Err("同步挂起时长不能超过暂停时长".into());
    }
    if day.seated_since_water != 0 || day.seated_since_relief != 0 || day.paused_without_block != 0
    {
        return Err("同步文档不能携带设备提醒计数".into());
    }
    let mut categories = BTreeSet::new();
    for category in &day.categories {
        user_id(&category.id, false)?;
        text(&category.name, 80, false, true)?;
        if !categories.insert(category.id.as_str()) {
            return Err("同步学习日的项目重复".into());
        }
        if !(0..=1440).contains(&category.quota_minutes) {
            return Err("同步配额超出允许范围".into());
        }
        counter(category.accepted_seconds)?;
    }
    if let Some(timer) = &day.timer {
        if archived {
            return Err("已归档学习日不能仍在计时".into());
        }
        sync_id(&timer.sync_id)?;
        if !ids.insert(&timer.sync_id) {
            return Err("同步记录标识重复".into());
        }
        timestamp(timer.started_at)?;
        user_id(&timer.category, false)?;
        if !categories.contains(timer.category.as_str()) {
            return Err("同步计时引用了不存在的项目".into());
        }
        if !(1..=21_600).contains(&timer.total_seconds)
            || !(0..=timer.total_seconds).contains(&timer.elapsed_seconds)
            || !(0..=120).contains(&timer.break_minutes)
        {
            return Err("同步计时时长无效".into());
        }
        tasks(&timer.tasks)?;
    }
    let mut totals: BTreeMap<&str, i64> = BTreeMap::new();
    for entry in &day.ledger {
        sync_id(&entry.sync_id)?;
        if !ids.insert(&entry.sync_id) {
            return Err("同步记录标识重复".into());
        }
        user_id(&entry.category, false)?;
        timestamp(entry.started_at)?;
        timestamp(entry.ended_at)?;
        if !categories.contains(entry.category.as_str()) {
            return Err("同步记录引用了不存在的项目".into());
        }
        if !(1..=21_600).contains(&entry.seconds) || entry.ended_at < entry.started_at {
            return Err("同步专注记录时长无效".into());
        }
        tasks(&entry.tasks)?;
        if let Some(note) = &entry.completion_note {
            text(note, 2000, true, false)?;
        }
        if entry.accepted {
            *totals.entry(&entry.category).or_default() += entry.seconds;
        }
    }
    if day.categories.iter().any(|category| {
        category.accepted_seconds != totals.get(category.id.as_str()).copied().unwrap_or(0)
    }) {
        return Err("同步进度与专注记录不一致".into());
    }
    let mut open = false;
    for (index, pause) in day.pauses.iter().enumerate() {
        timestamp(pause.started_at)?;
        if let Some(until) = pause.ended_at {
            timestamp(until)?;
            if until < pause.started_at {
                return Err("同步暂停时间段无效".into());
            }
        } else {
            if archived || open || index + 1 != day.pauses.len() {
                return Err("同步未结束暂停段无效".into());
            }
            open = true;
        }
    }
    for rest in &day.rests {
        timestamp(rest.started_at)?;
        timestamp(rest.ended_at)?;
        if rest.ended_at < rest.started_at {
            return Err("同步休息时间段无效".into());
        }
    }
    if let Some(until) = day.break_until {
        timestamp(until)?;
        if archived || day.timer.is_some() || !open {
            return Err("同步休息状态无效".into());
        }
    }
    if !archived && day.timer.is_none() && !open {
        return Err("未计时的学习日必须处于暂停状态".into());
    }
    Ok(())
}

fn object_keys(value: &Value, expected: &[&str]) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("同步对象类型无效".into());
    };
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("同步文档含未知或缺失字段".into());
    }
    Ok(())
}

fn array(value: &Value) -> Result<&Vec<Value>, String> {
    value.as_array().ok_or_else(|| "同步列表类型无效".into())
}

fn strict_keys(value: &Value) -> Result<(), String> {
    object_keys(
        value,
        &[
            "schema",
            "preferences",
            "last_tick",
            "day",
            "history",
            "deleted_day_ids",
        ],
    )?;
    let preferences = &value["preferences"];
    object_keys(
        preferences,
        &[
            "categories",
            "profiles",
            "break_minutes",
            "uniform_block_minutes",
            "default_profile_id",
        ],
    )?;
    for category in array(&preferences["categories"])? {
        object_keys(
            category,
            &[
                "id",
                "name",
                "short_name",
                "default_block_minutes",
                "icon",
                "role",
                "block_rationale",
            ],
        )?;
    }
    for profile in array(&preferences["profiles"])? {
        object_keys(profile, &["id", "name", "subtitle", "quotas"])?;
        for quota in array(&profile["quotas"])? {
            object_keys(quota, &["category", "minutes"])?;
        }
    }
    if !value["day"].is_null() {
        strict_day_keys(&value["day"])?;
    }
    for archived in array(&value["history"])? {
        object_keys(archived, &["day", "ended_at"])?;
        strict_day_keys(&archived["day"])?;
    }
    Ok(())
}

fn strict_day_keys(day: &Value) -> Result<(), String> {
    object_keys(
        day,
        &[
            "sync_id",
            "profile_name",
            "profile_id",
            "started_at",
            "categories",
            "timer",
            "ledger",
            "seated_seconds",
            "paused_seconds",
            "suspend_seconds",
            "cups",
            "seated_since_water",
            "seated_since_relief",
            "paused_without_block",
            "break_until",
            "pauses",
            "rests",
        ],
    )?;
    for category in array(&day["categories"])? {
        object_keys(
            category,
            &["id", "name", "quota_minutes", "accepted_seconds"],
        )?;
    }
    if !day["timer"].is_null() {
        object_keys(
            &day["timer"],
            &[
                "sync_id",
                "category",
                "total_seconds",
                "elapsed_seconds",
                "tasks",
                "started_at",
                "break_minutes",
            ],
        )?;
        strict_task_keys(&day["timer"]["tasks"])?;
    }
    for entry in array(&day["ledger"])? {
        object_keys(
            entry,
            &[
                "sync_id",
                "category",
                "seconds",
                "accepted",
                "tasks",
                "started_at",
                "ended_at",
                "completion_note",
            ],
        )?;
        strict_task_keys(&entry["tasks"])?;
    }
    for pause in array(&day["pauses"])? {
        object_keys(pause, &["started_at", "ended_at", "auto"])?;
    }
    for rest in array(&day["rests"])? {
        object_keys(rest, &["started_at", "ended_at"])?;
    }
    Ok(())
}

fn strict_task_keys(value: &Value) -> Result<(), String> {
    for task in array(value)? {
        object_keys(task, &["text", "done"])?;
    }
    Ok(())
}
