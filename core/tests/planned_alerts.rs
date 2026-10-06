use sitzfleisch_core::{from_json, to_json, AlertKind, PlannedAlert, State};

fn enabled_state() -> State {
    let mut state = State::new(1_000);
    state.preferences.water_reminder_enabled = true;
    state.preferences.stretch_reminder_enabled = true;
    state.preferences.stretch_reminder_minutes = 5;
    state.preferences.idle_reminder_enabled = true;
    state.preferences.idle_reminder_minutes = 5;
    state.preferences.break_minutes = 2;
    state
}

fn started() -> State {
    let mut state = enabled_state();
    state.start_day("standard", state.last_tick).unwrap();
    state
}

fn settle(state: &mut State, local_seconds: u32) {
    state.take_due_water_reminder(local_seconds);
    state.take_due_reminders();
    state.take_due_break(state.last_tick);
}

fn collect_actual(mut state: State, local_seconds: u32, horizon: i64) -> Vec<PlannedAlert> {
    let started_at = state.last_tick;
    let mut alerts = Vec::new();
    for elapsed in 1..=horizon {
        let now = started_at + elapsed;
        let running = state
            .day
            .as_ref()
            .is_some_and(|day| !day.is_paused() && day.timer.is_some());
        state.tick_wall_clock(now);
        let finished = running && state.day.as_ref().unwrap().timer.is_none();
        let pause_started_at = state
            .day
            .as_ref()
            .filter(|day| day.is_paused())
            .and_then(|day| day.pauses.last())
            .map(|pause| pause.started_at);
        let water = state
            .take_due_water_reminder((i64::from(local_seconds) + elapsed).rem_euclid(3_600) as u32);
        let (_, stretch, idle) = state.take_due_reminders();
        let rest = state.take_due_break(now);
        for (kind, due, pause) in [
            (AlertKind::BlockFinished, finished, None),
            (AlertKind::RestOver, rest, None),
            (AlertKind::Stretch, stretch, None),
            (AlertKind::Idle, idle, pause_started_at),
            (AlertKind::Water, water, pause_started_at),
        ] {
            if due {
                alerts.push(PlannedAlert {
                    kind,
                    at: now,
                    pause_started_at: pause,
                });
            }
        }
        if let Some(day) = &state.day {
            assert_eq!(day.suspend_seconds, 0);
            assert_eq!(
                day.seated_seconds + day.paused_seconds,
                now - day.started_at
            );
        }
    }
    // 参考结果来自实际 tick/take_due；这里只应用预排数量和范围的合同，不复制预测公式。
    let mut counts = [0; 5];
    alerts.retain(|alert| {
        if matches!(alert.kind, AlertKind::Idle | AlertKind::Water)
            && alert.at - started_at > 43_200
        {
            return false;
        }
        let limit = match alert.kind {
            AlertKind::Stretch | AlertKind::Idle => 72,
            AlertKind::Water => 24,
            _ => 1,
        };
        let count = &mut counts[alert.kind as usize];
        *count += 1;
        *count <= limit
    });
    alerts.sort_unstable_by_key(|alert| (alert.at, alert.kind));
    alerts
}

fn assert_aligned(state: &State, local_seconds: u32, horizon: i64) -> Vec<PlannedAlert> {
    assert_eq!(state.water_clock_checked_at, Some(state.last_tick));
    let before = to_json(state);
    let planned = state.planned_alerts(local_seconds, horizon);
    assert_eq!(to_json(state), before, "预测不能修改当前状态");
    let actual = collect_actual(state.clone(), local_seconds, horizon);
    assert_eq!(
        planned, actual,
        "预测与实际提醒必须逐项相等，误差 0 秒；状态：{state:?}"
    );
    planned
}

fn times(alerts: &[PlannedAlert], kind: AlertKind) -> Vec<i64> {
    alerts
        .iter()
        .filter(|alert| alert.kind == kind)
        .map(|alert| alert.at)
        .collect()
}

#[test]
fn running_with_rest_aligns_completion_rest_stretch_idle_and_water() {
    let mut state = started();
    state.start_block_with_break("main", 20, vec![], 2).unwrap();
    for local_seconds in [0, 815, 1_799, 3_599] {
        let mut initial = state.clone();
        settle(&mut initial, local_seconds);
        let alerts = assert_aligned(&initial, local_seconds, 10_800);
        assert_eq!(times(&alerts, AlertKind::BlockFinished), [2_200]);
        assert_eq!(times(&alerts, AlertKind::RestOver), [2_320]);
        assert_eq!(times(&alerts, AlertKind::Idle)[0], 2_620);
    }
}

#[test]
fn running_without_rest_aligns_and_never_plans_a_rest_over() {
    let mut state = started();
    state.start_block_with_break("main", 10, vec![], 0).unwrap();
    settle(&mut state, 1_799);
    let alerts = assert_aligned(&state, 1_799, 7_200);
    assert!(times(&alerts, AlertKind::RestOver).is_empty());
    assert_eq!(times(&alerts, AlertKind::Idle)[0], 1_900);
}

#[test]
fn a_manually_paused_block_plans_only_water_with_its_pause_origin() {
    let mut state = started();
    state.start_block("main", 10, vec![]).unwrap();
    state.tick_wall_clock(1_060);
    state.toggle_pause(1_060).unwrap();
    settle(&mut state, 1_799);
    let alerts = assert_aligned(&state, 1_799, 7_200);
    assert!(!alerts.is_empty());
    assert!(alerts
        .iter()
        .all(|alert| alert.kind == AlertKind::Water && alert.pause_started_at == Some(1_060)));
}

#[test]
fn resting_aligns_the_deadline_post_rest_idle_and_pause_origin() {
    let mut state = started();
    state.start_block_with_break("main", 5, vec![], 2).unwrap();
    state.tick_wall_clock(1_330);
    settle(&mut state, 815);
    let alerts = assert_aligned(&state, 815, 7_200);
    assert_eq!(times(&alerts, AlertKind::RestOver), [1_420]);
    assert_eq!(times(&alerts, AlertKind::Idle)[0], 1_720);
    assert!(alerts
        .iter()
        .filter(|alert| matches!(alert.kind, AlertKind::Idle | AlertKind::Water))
        .all(|alert| alert.pause_started_at == Some(1_300)));
}

#[test]
fn a_long_pause_keeps_idle_and_water_for_the_shell_to_filter() {
    let mut state = started();
    state.tick_wall_clock(8_201);
    settle(&mut state, 1_799);
    let alerts = assert_aligned(&state, 1_799, 10_800);
    assert!(!times(&alerts, AlertKind::Idle).is_empty());
    assert!(!times(&alerts, AlertKind::Water).is_empty());
    assert!(alerts
        .iter()
        .all(|alert| alert.pause_started_at == Some(1_000)));
}

#[test]
fn water_uses_local_hour_and_half_hour_boundaries() {
    for (local_seconds, first_gap) in [(0, 1_800), (1_799, 1), (1_800, 1_800), (3_599, 1)] {
        let mut state = enabled_state();
        settle(&mut state, local_seconds);
        let alerts = assert_aligned(&state, local_seconds, 3_600);
        assert_eq!(
            times(&alerts, AlertKind::Water),
            [1_000 + first_gap, 2_800 + first_gap]
        );
        assert!(alerts
            .iter()
            .all(|alert| alert.kind == AlertKind::Water && alert.pause_started_at.is_none()));
    }
}

#[test]
fn five_minute_stretch_excludes_the_exact_block_endpoint() {
    let mut state = started();
    state.start_block_with_break("main", 10, vec![], 0).unwrap();
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 600);
    assert_eq!(
        alerts,
        [
            PlannedAlert {
                kind: AlertKind::Stretch,
                at: 1_300,
                pause_started_at: None
            },
            PlannedAlert {
                kind: AlertKind::BlockFinished,
                at: 1_600,
                pause_started_at: None
            },
        ]
    );
}

#[test]
fn idle_repeats_after_each_consumed_interval() {
    let mut state = started();
    state.preferences.water_reminder_enabled = false;
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 3_600);
    assert_eq!(
        times(&alerts, AlertKind::Idle),
        (1..=12)
            .map(|index| 1_000 + index * 300)
            .collect::<Vec<_>>()
    );
    assert!(alerts
        .iter()
        .all(|alert| alert.pause_started_at == Some(1_000)));
}

#[test]
fn water_gets_the_future_pause_origin_at_and_after_block_completion() {
    let mut state = started();
    state.start_block_with_break("main", 40, vec![], 3).unwrap();
    settle(&mut state, 1_799);
    let alerts = assert_aligned(&state, 1_799, 5_400);
    let water: Vec<_> = alerts
        .iter()
        .filter(|alert| alert.kind == AlertKind::Water)
        .collect();
    assert_eq!(
        water
            .iter()
            .map(|alert| alert.pause_started_at)
            .collect::<Vec<_>>(),
        [None, None, Some(3_400)]
    );

    let mut state = started();
    state.start_block_with_break("main", 30, vec![], 2).unwrap();
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 1_800);
    let simultaneous: Vec<_> = alerts.iter().filter(|alert| alert.at == 2_800).collect();
    assert_eq!(
        simultaneous
            .iter()
            .map(|alert| alert.kind)
            .collect::<Vec<_>>(),
        [AlertKind::BlockFinished, AlertKind::Water]
    );
    assert_eq!(simultaneous[1].pause_started_at, Some(2_800));
}

#[test]
fn reminder_switches_do_not_disable_block_or_rest_end_alerts() {
    for mask in 0..8 {
        let mut state = started();
        state.preferences.stretch_reminder_enabled = mask & 1 != 0;
        state.preferences.idle_reminder_enabled = mask & 2 != 0;
        state.preferences.water_reminder_enabled = mask & 4 != 0;
        state.start_block("main", 10, vec![]).unwrap();
        settle(&mut state, 0);
        let alerts = assert_aligned(&state, 0, 3_600);
        assert_eq!(times(&alerts, AlertKind::BlockFinished), [1_600]);
        assert_eq!(times(&alerts, AlertKind::RestOver), [1_720]);
        assert_eq!(times(&alerts, AlertKind::Stretch).is_empty(), mask & 1 == 0);
        assert_eq!(times(&alerts, AlertKind::Idle).is_empty(), mask & 2 == 0);
        assert_eq!(times(&alerts, AlertKind::Water).is_empty(), mask & 4 == 0);
    }
}

#[test]
fn horizons_are_positive_and_include_the_exact_last_second() {
    let mut state = started();
    state.start_block_with_break("main", 1, vec![], 1).unwrap();
    settle(&mut state, 1_740);
    for horizon in [0, -1, i64::MIN] {
        assert!(state.planned_alerts(1_740, horizon).is_empty());
    }
    assert!(state.planned_alerts(1_740, 59).is_empty());
    let alerts = assert_aligned(&state, 1_740, 60);
    assert_eq!(
        alerts,
        [
            PlannedAlert {
                kind: AlertKind::BlockFinished,
                at: 1_060,
                pause_started_at: None
            },
            PlannedAlert {
                kind: AlertKind::Water,
                at: 1_060,
                pause_started_at: Some(1_060)
            },
        ]
    );
    let alerts = assert_aligned(&state, 1_740, 120);
    assert_eq!(times(&alerts, AlertKind::RestOver), [1_120]);
}

#[test]
fn periodic_counts_and_twelve_hour_limits_bound_large_horizons() {
    let mut state = started();
    state.start_block_with_break("main", 1, vec![], 0).unwrap();
    state.tick_wall_clock(1_060);
    state
        .start_block_with_break("main", 180, vec![], 0)
        .unwrap();
    state.extend_block(180).unwrap();
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 86_400);
    assert_eq!(times(&alerts, AlertKind::Stretch).len(), 72);
    assert_eq!(times(&alerts, AlertKind::Idle).len(), 72);
    assert_eq!(times(&alerts, AlertKind::Water).len(), 24);
    assert!(alerts
        .iter()
        .filter(|alert| matches!(alert.kind, AlertKind::Idle | AlertKind::Water))
        .all(|alert| alert.at <= state.last_tick + 43_200));
    assert_eq!(state.planned_alerts(0, i64::MAX), alerts);
}

#[test]
fn legacy_break_fallback_matches_actual_wall_clock_completion() {
    let mut state = started();
    state.preferences.break_minutes = 3;
    state.start_block("main", 1, vec![]).unwrap();
    let mut value = serde_json::to_value(&state).unwrap();
    value["day"]["timer"]
        .as_object_mut()
        .unwrap()
        .remove("break_minutes");
    let mut state = from_json(&value.to_string()).unwrap();
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 3_600);
    assert_eq!(times(&alerts, AlertKind::RestOver), [1_240]);
    assert_eq!(times(&alerts, AlertKind::Idle)[0], 1_540);
}

#[test]
fn rest_with_a_retained_idle_counter_uses_its_remaining_interval() {
    let mut state = started();
    state.start_block_with_break("main", 1, vec![], 2).unwrap();
    state.tick_wall_clock(1_060);
    state.day.as_mut().unwrap().paused_without_block = 42;
    settle(&mut state, 0);
    let alerts = assert_aligned(&state, 0, 3_600);
    assert_eq!(times(&alerts, AlertKind::Idle)[0], 1_438);
}

#[test]
fn future_water_records_follow_the_same_rollback_guard_as_actual_ticks() {
    for ahead in [1_799, 1_800, 1_801, 1_802, 86_400] {
        for local_seconds in [0, 1_799, 3_599] {
            let mut state = enabled_state();
            // 模拟恢复后的未来提醒记录；观察时钟已对齐，下一秒的防回拨规则负责修正。
            state.water_reminded_at = Some(state.last_tick + ahead);
            assert_aligned(&state, local_seconds, 7_200);
        }
    }
}

#[test]
fn invalid_local_clock_seconds_skip_only_water() {
    let mut state = started();
    state.start_block("main", 10, vec![]).unwrap();
    settle(&mut state, 0);
    let expected: Vec<_> = state
        .planned_alerts(0, 3_600)
        .into_iter()
        .filter(|alert| alert.kind != AlertKind::Water)
        .collect();
    for local_seconds in [3_600, u32::MAX] {
        assert_eq!(state.planned_alerts(local_seconds, 3_600), expected);
    }
}

#[test]
fn extreme_timestamps_and_loaded_numbers_cannot_overflow_or_expand_the_queue() {
    for now in [i64::MIN, i64::MAX - 3, i64::MAX] {
        let mut state = enabled_state();
        state.last_tick = now;
        for horizon in [1, 43_200, i64::MAX] {
            let alerts = state.planned_alerts(1_799, horizon);
            assert!(alerts.len() <= 170);
            assert!(alerts.iter().all(|alert| alert.at > now
                && i128::from(alert.at) <= i128::from(now) + i128::from(horizon)));
        }
    }
    let mut state = started();
    state.start_block("main", 10, vec![]).unwrap();
    let timer = state.day.as_mut().unwrap().timer.as_mut().unwrap();
    timer.total_seconds = i64::MAX;
    timer.elapsed_seconds = i64::MIN;
    timer.break_minutes = i64::MAX;
    state.day.as_mut().unwrap().seated_since_relief = i64::MAX;
    state.preferences.idle_reminder_minutes = i64::MAX;
    assert!(state.planned_alerts(0, i64::MAX).len() <= 170);
    state.preferences.stretch_reminder_minutes = 0;
    state.preferences.idle_reminder_minutes = -1;
    assert!(state
        .planned_alerts(0, 3_600)
        .iter()
        .all(|alert| !matches!(alert.kind, AlertKind::Stretch | AlertKind::Idle)));
}

fn random_below(seed: &mut u64, limit: u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed % limit
}

#[test]
fn fixed_seed_state_and_local_clock_combinations_align_to_zero_seconds() {
    let mut seed = 0x5172_2026_1003;
    for case in 0..96 {
        let mut state = enabled_state();
        state.preferences.water_reminder_enabled = random_below(&mut seed, 2) == 1;
        state.preferences.stretch_reminder_enabled = random_below(&mut seed, 2) == 1;
        state.preferences.idle_reminder_enabled = random_below(&mut seed, 2) == 1;
        state.preferences.stretch_reminder_minutes =
            [5, 6, 15, 50, 240][random_below(&mut seed, 5) as usize];
        state.preferences.idle_reminder_minutes =
            [5, 6, 15, 50, 240][random_below(&mut seed, 5) as usize];
        state.preferences.break_minutes = [0, 1, 2, 10][random_below(&mut seed, 4) as usize];
        if case % 8 != 0 {
            state.start_day("standard", state.last_tick).unwrap();
            if case % 6 != 0 {
                state
                    .start_block("main", random_below(&mut seed, 60) as i64 + 1, vec![])
                    .unwrap();
            }
            state.tick_wall_clock(state.last_tick + random_below(&mut seed, 7_201) as i64);
            if case % 3 == 0 && state.day.as_ref().unwrap().timer.is_some() {
                state.toggle_pause(state.last_tick).unwrap();
            }
        }
        let local_seconds = random_below(&mut seed, 3_600) as u32;
        settle(&mut state, local_seconds);
        assert_aligned(&state, local_seconds, 10_800);
    }
}
