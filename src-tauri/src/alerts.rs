#![cfg_attr(not(mobile), allow(dead_code))]

use serde::{Deserialize, Serialize};
use sitzfleisch_core as core;

pub const MAX_APPLIED_ALARMS: usize = 170;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedAlarm {
    pub alert: core::PlannedAlert,
    pub id: i32,
    pub title: String,
    pub body: String,
    pub channel: String,
}

pub fn notification_id(kind: core::AlertKind, at: i64) -> i32 {
    let base = match kind {
        core::AlertKind::BlockFinished => 10000,
        core::AlertKind::RestOver => 11000,
        core::AlertKind::Stretch => 12000,
        core::AlertKind::Idle => 13000,
        core::AlertKind::Water => 14000,
    };
    base + at.div_euclid(60).rem_euclid(1000) as i32
}

pub fn alert_copy(kind: core::AlertKind, state: &core::State, alert: &core::PlannedAlert)
    -> (&'static str, String, &'static str) {
    match kind {
        core::AlertKind::BlockFinished => ("这一格走完了", "时间已计入。打开坐功，记录这段时间完成了什么。".into(), "timer"),
        core::AlertKind::RestOver => ("休息结束", "开下一格吧。".into(), "timer"),
        core::AlertKind::Stretch => ("起来活动一下", format!("已经连续在座 {} 分钟。", state.preferences.stretch_reminder_minutes), "body"),
        core::AlertKind::Idle => {
            let minutes = alert.pause_started_at.map(|start| alert.at.saturating_sub(start).max(0) / 60).unwrap_or(0);
            ("还没开格", format!("已经暂停 {minutes} 分钟了。"), "body")
        },
        core::AlertKind::Water => ("喝点水吧", "忙了一阵，喝几口水再继续。".into(), "water"),
    }
}

pub fn mobile_alert_allowed(state: &core::State, alert: &core::PlannedAlert) -> bool {
    if alert.kind == core::AlertKind::Water && state.day.is_none() { return false; }
    !(matches!(alert.kind, core::AlertKind::Water | core::AlertKind::Idle)
        && alert.pause_started_at.is_some_and(|start| alert.at.saturating_sub(start) >= 7200))
}

pub fn desired_alarms(state: &core::State, local_seconds: u32) -> Vec<AppliedAlarm> {
    state.planned_alerts(local_seconds, 12 * 3600).into_iter()
        .filter(|alert| mobile_alert_allowed(state, alert))
        .map(|alert| {
            let (title, body, channel) = alert_copy(alert.kind, state, &alert);
            AppliedAlarm { id: notification_id(alert.kind, alert.at), alert,
                title: title.into(), body, channel: channel.into() }
        }).collect()
}

pub struct AlarmDiff {
    pub cancel: Vec<AppliedAlarm>,
    pub schedule: Vec<AppliedAlarm>,
}

/// 已经触发和即将触发的 ID 只退出本地账本，不调用会撤掉系统横幅的 cancel。
pub fn diff(applied: &[AppliedAlarm], desired: &[AppliedAlarm], now: i64) -> AlarmDiff {
    let cutoff = now.saturating_add(1);
    AlarmDiff {
        cancel: applied.iter().filter(|item| item.alert.at > cutoff && !desired.contains(item)).cloned().collect(),
        schedule: desired.iter().filter(|item| item.alert.at > cutoff && !applied.contains(item)).cloned().collect(),
    }
}

pub fn schedule_date(at: i64) -> Result<time::OffsetDateTime, time::error::ComponentRange> {
    time::OffsetDateTime::from_unix_timestamp(at)
}

pub fn valid_applied_alarm(item: &AppliedAlarm) -> bool {
    let channel = match item.alert.kind {
        core::AlertKind::BlockFinished | core::AlertKind::RestOver => "timer",
        core::AlertKind::Stretch | core::AlertKind::Idle => "body",
        core::AlertKind::Water => "water",
    };
    item.id == notification_id(item.alert.kind, item.alert.at) && item.channel == channel
        && schedule_date(item.alert.at).is_ok()
        && item.alert.pause_started_at.is_none_or(|start| schedule_date(start).is_ok())
        && !item.title.is_empty() && item.title.len() <= 256 && item.body.len() <= 512
        && !item.title.contains('\0') && !item.body.contains('\0')
}

/// 用固定墙钟基准让系统自己走秒；模型不含每秒变化的剩余时长或托盘文字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusModel {
    pub visible: bool,
    pub title: String,
    pub text: String,
    pub chronometer_base_ms: Option<i64>,
    pub count_down: bool,
    /// 保存绝对截止时刻，投递前才换算为 timeoutAfter，避免每秒重发状态通知。
    pub timeout_at_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate<'a> {
    pub visible: bool,
    pub title: &'a str,
    pub text: &'a str,
    pub chronometer_base_ms: Option<i64>,
    pub count_down: bool,
    pub timeout_after_ms: Option<i64>,
}

impl StatusModel {
    pub fn update(&self, now_ms: i64) -> StatusUpdate<'_> {
        StatusUpdate {
            visible: self.visible,
            title: &self.title,
            text: &self.text,
            chronometer_base_ms: self.chronometer_base_ms,
            count_down: self.count_down,
            timeout_after_ms: self.timeout_at_ms.map(|at| at.saturating_sub(now_ms).max(1)),
        }
    }
}

fn millis(seconds: i64) -> i64 {
    seconds.saturating_mul(1000).max(0)
}

pub fn status_model(state: &core::State) -> StatusModel {
    let mut model = StatusModel {
        visible: state.day.is_some(),
        title: "坐功".into(),
        text: String::new(),
        chronometer_base_ms: None,
        count_down: false,
        timeout_at_ms: None,
    };
    let Some(day) = &state.day else { return model };
    let name_of = |id: &str| day.categories.iter().find(|category| category.id == id)
        .map(|category| category.name.as_str()).unwrap_or(id).to_owned();
    if let Some(timer) = &day.timer {
        let name = name_of(&timer.category);
        if day.is_paused() {
            model.text = format!("{name} · 已暂停");
            model.chronometer_base_ms = day.pauses.last().map(|pause| millis(pause.started_at));
        } else {
            let end = millis(state.last_tick.saturating_add(timer.remaining_seconds().max(1)));
            model.text = name;
            model.chronometer_base_ms = Some(end);
            model.count_down = true;
            model.timeout_at_ms = Some(end);
        }
    } else if let Some(until) = day.break_until.filter(|until| *until > state.last_tick) {
        model.text = "休息中".into();
        model.chronometer_base_ms = Some(millis(until));
        model.count_down = true;
        model.timeout_at_ms = Some(millis(until));
    } else {
        model.text = match core::suggest(day, &state.preferences, state.last_tick) {
            Some(next) => format!("下一格 {}", name_of(&next.category)),
            None => "今日达成".into(),
        };
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    fn studying() -> core::State {
        let mut state = core::State::new(1_700_000_000);
        let profile = state.preferences.profiles[0].id.clone();
        state.start_day(&profile, state.last_tick).unwrap();
        state
    }

    fn start_block(state: &mut core::State) {
        let category = state.day.as_ref().unwrap().categories[0].id.clone();
        state.start_block_with_break(&category, 1, Vec::new(), 1).unwrap();
    }

    fn alarm(kind: core::AlertKind, at: i64) -> AppliedAlarm {
        let state = studying();
        let alert = core::PlannedAlert { kind, at, pause_started_at: None };
        let (title, body, channel) = alert_copy(kind, &state, &alert);
        AppliedAlarm { id: notification_id(kind, at), alert, title: title.into(), body, channel: channel.into() }
    }

    #[test]
    fn diff_preserves_fired_and_imminent_notifications_and_only_schedules_the_future() {
        let fired = alarm(core::AlertKind::Water, 100);
        let imminent = alarm(core::AlertKind::Idle, 101);
        let removed = alarm(core::AlertKind::Stretch, 105);
        let retained = alarm(core::AlertKind::BlockFinished, 106);
        let added = alarm(core::AlertKind::RestOver, 107);
        let delta = diff(&[fired.clone(), imminent.clone(), removed.clone(), retained.clone()],
            &[fired, imminent, retained, added.clone()], 100);
        assert_eq!(delta.cancel, vec![removed]);
        assert_eq!(delta.schedule, vec![added]);
    }

    #[test]
    fn ids_are_kind_scoped_and_roll_with_minutes_instead_of_reusing_recent_notifications() {
        for (kind, base) in [(core::AlertKind::BlockFinished,10000), (core::AlertKind::RestOver,11000),
            (core::AlertKind::Stretch,12000), (core::AlertKind::Idle,13000), (core::AlertKind::Water,14000)] {
            assert_eq!(notification_id(kind, 60 * 1234), base + 234);
            assert_ne!(notification_id(kind, 60 * 1234), notification_id(kind, 60 * 1235));
        }
    }

    #[test]
    fn mobile_water_requires_a_day_and_long_pauses_filter_water_and_idle_at_the_exact_boundary() {
        let mut state = core::State::new(100);
        let mut alert = core::PlannedAlert { kind: core::AlertKind::Water, at: 1000, pause_started_at: None };
        assert!(!mobile_alert_allowed(&state, &alert));
        state.start_day("standard", 100).unwrap();
        assert!(mobile_alert_allowed(&state, &alert));
        for kind in [core::AlertKind::Water, core::AlertKind::Idle] {
            alert.kind = kind;
            alert.pause_started_at = Some(100);
            alert.at = 7299;
            assert!(mobile_alert_allowed(&state, &alert));
            alert.at = 7300;
            assert!(!mobile_alert_allowed(&state, &alert));
        }
    }

    #[test]
    fn schedule_serialization_uses_utc_and_milliseconds_without_fractional_drift() {
        let date = schedule_date(1_700_000_000).unwrap();
        assert_eq!(date.nanosecond(), 0);
        assert_eq!(date.offset(), time::UtcOffset::UTC);
        let payload = serde_json::to_value(tauri_plugin_notification::Schedule::At {
            date, repeating: false, allow_while_idle: true,
        }).unwrap();
        // 插件格式器固定输出九位小数；全零在 Android 的毫秒解析器里仍是 0，不能有纳秒余量。
        assert_eq!(payload["at"]["date"], "2023-11-14T22:13:20.000000000Z");
        assert_eq!(payload["at"]["allowWhileIdle"], true);
    }

    #[test]
    fn persisted_alarm_identity_and_channel_are_checked_before_native_calls() {
        let mut item = alarm(core::AlertKind::Water, 1_700_000_000);
        assert!(valid_applied_alarm(&item));
        item.id = 9000;
        assert!(!valid_applied_alarm(&item));
        item.id = notification_id(item.alert.kind, item.alert.at);
        item.channel = "other".into();
        assert!(!valid_applied_alarm(&item));
    }

    #[test]
    fn no_study_day_cancels_status_and_next_block_has_no_chronometer() {
        let hidden = status_model(&core::State::new(1_700_000_000));
        assert!(!hidden.visible);
        assert_eq!(hidden.chronometer_base_ms, None);
        let state = studying();
        let next = status_model(&state);
        assert!(next.visible && next.text.starts_with("下一格 "));
        assert_eq!(next.chronometer_base_ms, None);
        assert_eq!(next.timeout_at_ms, None);
    }

    #[test]
    fn running_model_stays_equal_across_ticks_but_changes_when_extended() {
        let mut state = studying();
        start_block(&mut state);
        let before = status_model(&state);
        assert!(before.count_down);
        assert_eq!(before.chronometer_base_ms, Some((state.last_tick + 60) * 1000));
        assert_eq!(before.timeout_at_ms, before.chronometer_base_ms);
        state.tick(state.last_tick + 1);
        assert_eq!(status_model(&state), before);
        state.extend_block(1).unwrap();
        assert_ne!(status_model(&state), before);
    }

    #[test]
    fn pause_counts_up_from_pause_start_and_resume_replaces_the_deadline() {
        let mut state = studying();
        start_block(&mut state);
        state.tick(state.last_tick + 5);
        state.toggle_pause(state.last_tick).unwrap();
        let paused = status_model(&state);
        assert!(!paused.count_down && paused.text.ends_with("已暂停"));
        assert_eq!(paused.chronometer_base_ms, Some(state.last_tick * 1000));
        assert_eq!(paused.timeout_at_ms, None);
        state.tick(state.last_tick + 7);
        assert_eq!(status_model(&state), paused);
        state.toggle_pause(state.last_tick).unwrap();
        let resumed = status_model(&state);
        assert!(resumed.count_down);
        assert_eq!(resumed.chronometer_base_ms, Some((state.last_tick + 55) * 1000));
    }

    #[test]
    fn rest_counts_down_and_expiration_returns_to_next_block() {
        let mut state = studying();
        start_block(&mut state);
        state.tick(state.last_tick + 60);
        let resting = status_model(&state);
        assert_eq!(resting.text, "休息中");
        assert!(resting.count_down);
        assert_eq!(resting.timeout_at_ms, Some((state.last_tick + 60) * 1000));
        state.tick(state.last_tick + 1);
        assert_eq!(status_model(&state), resting);
        state.tick(state.last_tick + 59);
        assert!(status_model(&state).text.starts_with("下一格 "));
    }

    #[test]
    fn timeout_is_converted_only_at_delivery_and_never_becomes_zero() {
        let mut state = studying();
        start_block(&mut state);
        let model = status_model(&state);
        let first = serde_json::to_value(model.update(state.last_tick * 1000)).unwrap();
        assert_eq!(first["timeoutAfterMs"], 60_000);
        assert_eq!(first["chronometerBaseMs"], (state.last_tick + 60) * 1000);
        let later = serde_json::to_value(model.update((state.last_tick + 61) * 1000)).unwrap();
        assert_eq!(later["timeoutAfterMs"], 1);
    }
}
