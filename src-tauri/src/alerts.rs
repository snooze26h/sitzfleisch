#![cfg_attr(not(mobile), allow(dead_code))]

use serde::Serialize;
use sitzfleisch_core as core;

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
