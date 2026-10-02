use sitzfleisch_core::{from_json, to_json, Day, RestSpan, State, TaskItem};

fn started() -> State {
    let mut state = State::new(1_000);
    state.start_day("standard", 1_000).unwrap();
    state
}

fn assert_wall_clock_day(state: &State) -> &Day {
    let day = state.day.as_ref().unwrap();
    assert_eq!(day.suspend_seconds, 0);
    assert!(day.pauses.iter().all(|pause| !pause.auto));
    // 只对从未回拨过的序列检查这个不变量；回拨的独立测试不调用本函数。
    assert_eq!(
        day.seated_seconds + day.paused_seconds,
        state.last_tick - day.started_at
    );
    day
}

fn take_due(state: &mut State, local_offset: i64) {
    state.take_due_water_reminder((state.last_tick + local_offset).rem_euclid(3_600) as u32);
    state.take_due_reminders();
    state.take_due_break(state.last_tick);
}

fn walk_wall_clock(state: &mut State, target: i64, local_offset: i64) {
    while state.last_tick < target {
        state.tick_wall_clock(state.last_tick + 1);
        take_due(state, local_offset);
        if state.day.is_some() {
            assert_wall_clock_day(state);
        }
    }
}

fn assert_jump_equivalent(initial: &State, gap: i64, local_offset: i64) {
    let target = initial.last_tick + gap;
    let mut jumped = initial.clone();
    jumped.tick_wall_clock(target);
    take_due(&mut jumped, local_offset);
    let mut stepped = initial.clone();
    walk_wall_clock(&mut stepped, target, local_offset);
    if gap == 0 {
        take_due(&mut stepped, local_offset);
    }
    assert_eq!(jumped.last_tick, stepped.last_tick);
    let jumped_day = serde_json::to_value(&jumped.day).unwrap();
    let stepped_day = serde_json::to_value(&stepped.day).unwrap();
    // 序列化只为比较没有 PartialEq 的台账、暂停和计时格；逐字段报告相位或结算偏差。
    for field in [
        "seated_seconds",
        "paused_seconds",
        "ledger",
        "rests",
        "pauses",
        "timer",
        "break_until",
        "seated_since_relief",
        "paused_without_block",
        "categories",
        "suspend_seconds",
    ] {
        assert_eq!(
            jumped_day[field], stepped_day[field],
            "字段 {field}；gap={gap}；起始状态：{initial:?}"
        );
    }
    if jumped.day.is_some() {
        assert_wall_clock_day(&jumped);
    }
}

#[test]
fn a_large_wall_clock_gap_keeps_an_unfinished_block_running() {
    let mut state = started();
    state.start_block("main", 180, vec![]).unwrap();
    state.tick_wall_clock(1_030);
    state.tick_wall_clock(4_630);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.seated_seconds, 3_630);
    assert_eq!(day.paused_seconds, 0);
    assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 3_630);
    assert!(!day.is_paused());
}

#[test]
fn completion_during_a_gap_records_the_real_endpoint_and_rest_start() {
    let mut state = started();
    state.start_block_with_break("main", 3, vec![], 10).unwrap();
    state.tick_wall_clock(1_360);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.seated_seconds, 180);
    assert_eq!(day.paused_seconds, 180);
    assert_eq!(day.categories[0].accepted_seconds, 180);
    assert_eq!(day.ledger[0].ended_at, 1_180);
    assert_eq!(day.ledger[0].completion_note, None);
    assert_eq!(day.pauses.last().unwrap().started_at, 1_180);
    assert_eq!(
        day.rests,
        vec![RestSpan {
            started_at: 1_180,
            ended_at: 1_780
        }]
    );
    assert_eq!(day.break_until, Some(1_780));
    assert_eq!(day.paused_without_block, 0);
    assert!(day.timer.is_none());
}

#[test]
fn a_gap_longer_than_block_and_rest_counts_only_post_rest_idle() {
    let mut state = started();
    state.start_block_with_break("main", 3, vec![], 2).unwrap();
    state.tick_wall_clock(1_900);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.seated_seconds, 180);
    assert_eq!(day.paused_seconds, 720);
    assert_eq!(day.paused_without_block, 600);
    assert_eq!(
        day.rests,
        vec![RestSpan {
            started_at: 1_180,
            ended_at: 1_300
        }]
    );
    assert!(state.take_due_break(1_900));
    assert!(!state.take_due_break(1_900));
}

#[test]
fn a_gap_starting_in_rest_counts_idle_from_the_deadline() {
    let mut state = started();
    state.start_block_with_break("main", 3, vec![], 2).unwrap();
    state.tick_wall_clock(1_180);
    state.tick_wall_clock(1_250);
    assert_eq!(state.day.as_ref().unwrap().paused_without_block, 0);
    state.tick_wall_clock(1_300);
    assert_eq!(state.day.as_ref().unwrap().paused_without_block, 0);
    state.tick_wall_clock(1_350);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.paused_without_block, 50);
    assert_eq!(day.paused_seconds, 170);
}

#[test]
fn a_long_manual_pause_keeps_the_block_stopped() {
    let mut state = started();
    state.start_block("main", 20, vec![]).unwrap();
    state.tick_wall_clock(1_060);
    state.toggle_pause(1_060).unwrap();
    state.tick_wall_clock(11_860);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.seated_seconds, 60);
    assert_eq!(day.paused_seconds, 10_800);
    assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 60);
    assert_eq!(day.paused_without_block, 0);
    assert_eq!(day.seated_since_relief, 60);
    assert_eq!(state.take_due_reminders(), (false, false, false));
}

#[test]
fn a_long_pause_without_a_block_tracks_all_elapsed_seconds() {
    let mut state = started();
    state.tick_wall_clock(20_000);
    let day = assert_wall_clock_day(&state);
    assert_eq!(day.seated_seconds, 0);
    assert_eq!(day.paused_seconds, 19_000);
    assert_eq!(day.paused_without_block, 19_000);
}

#[test]
fn cold_restart_fills_the_gap_and_resets_only_the_water_observation_clock() {
    let mut state = started();
    state.preferences.water_reminder_enabled = true;
    state.start_block("main", 20, vec![]).unwrap();
    state.tick_wall_clock(1_060);
    let mut restored = from_json(&to_json(&state)).unwrap();
    restored.resume_after_restart_wall_clock(1_960);
    let day = assert_wall_clock_day(&restored);
    assert_eq!(day.seated_seconds, 960);
    assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 960);
    assert!(!day.is_paused());
    assert_eq!(restored.water_clock_checked_at, Some(1_960));
    assert_eq!(restored.water_reminded_at, state.water_reminded_at);
    assert!(!restored.take_due_water_reminder(0));
}

#[test]
fn cold_restart_settles_completion_and_preserves_a_manual_pause() {
    let mut state = started();
    state.start_block_with_break("main", 3, vec![], 2).unwrap();
    state.tick_wall_clock(1_060);
    let mut restored = from_json(&to_json(&state)).unwrap();
    restored.resume_after_restart_wall_clock(1_600);
    let day = assert_wall_clock_day(&restored);
    assert_eq!(day.seated_seconds, 180);
    assert_eq!(day.paused_seconds, 420);
    assert_eq!(day.ledger[0].ended_at, 1_180);
    assert_eq!(day.paused_without_block, 300);

    state.toggle_pause(1_060).unwrap();
    let mut restored = from_json(&to_json(&state)).unwrap();
    restored.resume_after_restart_wall_clock(1_600);
    let day = assert_wall_clock_day(&restored);
    assert_eq!(day.seated_seconds, 60);
    assert_eq!(day.paused_seconds, 540);
    assert_eq!(day.timer.as_ref().unwrap().elapsed_seconds, 60);
    assert_eq!(day.pauses.last().unwrap().started_at, 1_060);
}

#[test]
fn legacy_timers_keep_the_preference_rest_fallback() {
    let mut state = started();
    state.preferences.break_minutes = 3;
    state.start_block("main", 1, vec![]).unwrap();
    let mut value = serde_json::to_value(&state).unwrap();
    value["day"]["timer"]
        .as_object_mut()
        .unwrap()
        .remove("break_minutes");
    let mut restored = from_json(&value.to_string()).unwrap();
    restored.tick_wall_clock(1_900);
    let day = assert_wall_clock_day(&restored);
    assert_eq!(
        day.rests,
        vec![RestSpan {
            started_at: 1_060,
            ended_at: 1_240
        }]
    );
    assert_eq!(day.paused_without_block, 660);
}

#[test]
fn wall_clock_rollbacks_do_not_credit_time_or_open_an_auto_pause() {
    let mut state = started();
    state.start_block("main", 20, vec![]).unwrap();
    state.tick_wall_clock(1_300);
    let original = serde_json::to_value(&state.day).unwrap();
    state.tick_wall_clock(1_200);
    assert_eq!(state.last_tick, 1_200);
    assert_eq!(serde_json::to_value(&state.day).unwrap(), original);
    state.tick_wall_clock(1_200);
    assert_eq!(serde_json::to_value(&state.day).unwrap(), original);
    state.resume_after_restart_wall_clock(1_100);
    assert_eq!(state.last_tick, 1_100);
    assert_eq!(state.water_clock_checked_at, Some(1_100));
    assert_eq!(serde_json::to_value(&state.day).unwrap(), original);
}

#[test]
fn two_second_gaps_preserve_stretch_and_idle_remainders() {
    let mut running = started();
    running.preferences.stretch_reminder_enabled = true;
    running.preferences.stretch_reminder_minutes = 5;
    running.start_block("main", 20, vec![]).unwrap();
    walk_wall_clock(&mut running, 1_299, 0);
    assert_jump_equivalent(&running, 2, 0);
    running.tick_wall_clock(1_301);
    assert_eq!(running.take_due_reminders(), (false, false, false));
    assert_eq!(running.day.as_ref().unwrap().seated_since_relief, 1);

    let mut idle = started();
    idle.preferences.idle_reminder_enabled = true;
    idle.preferences.idle_reminder_minutes = 5;
    walk_wall_clock(&mut idle, 1_299, 0);
    assert_jump_equivalent(&idle, 2, 0);
    idle.tick_wall_clock(1_301);
    assert_eq!(idle.take_due_reminders(), (false, false, false));
    assert_eq!(idle.day.as_ref().unwrap().paused_without_block, 1);
}

#[test]
fn large_gaps_do_not_replay_a_series_of_stretch_or_idle_reminders() {
    let mut running = started();
    running.preferences.stretch_reminder_enabled = true;
    running.preferences.stretch_reminder_minutes = 5;
    running.start_block("main", 180, vec![]).unwrap();
    running.tick_wall_clock(2_200);
    assert_eq!(running.take_due_reminders(), (false, true, false));
    assert_eq!(running.take_due_reminders(), (false, false, false));
    assert_eq!(running.day.as_ref().unwrap().seated_since_relief, 0);

    let mut idle = started();
    idle.preferences.idle_reminder_enabled = true;
    idle.preferences.idle_reminder_minutes = 5;
    idle.tick_wall_clock(2_201);
    assert_eq!(idle.take_due_reminders(), (false, false, false));
    assert_eq!(idle.day.as_ref().unwrap().paused_without_block, 1);
}

#[test]
fn finishing_exactly_at_a_stretch_boundary_keeps_its_counter_for_the_pause() {
    let mut state = started();
    state.preferences.stretch_reminder_enabled = true;
    state.preferences.stretch_reminder_minutes = 5;
    state.start_block_with_break("main", 5, vec![], 0).unwrap();
    assert_jump_equivalent(&state, 300, 0);
    state.tick_wall_clock(1_300);
    assert_eq!(state.take_due_reminders(), (false, false, false));
    assert_eq!(state.day.as_ref().unwrap().seated_since_relief, 300);
}

#[test]
fn wall_clock_and_heartbeat_keep_their_separate_suspend_and_idle_semantics() {
    let mut wall = started();
    wall.start_block_with_break("main", 1, vec![], 1).unwrap();
    let mut heartbeat = wall.clone();
    wall.tick_wall_clock(1_120);
    heartbeat.tick(1_120);
    assert_eq!(wall.day.as_ref().unwrap().paused_without_block, 0);
    assert_eq!(heartbeat.day.as_ref().unwrap().paused_without_block, 60);

    let mut wall = started();
    wall.start_block("main", 20, vec![]).unwrap();
    let mut heartbeat = wall.clone();
    wall.tick_wall_clock(1_121);
    heartbeat.tick(1_121);
    assert_eq!(wall.day.as_ref().unwrap().seated_seconds, 121);
    assert_eq!(heartbeat.day.as_ref().unwrap().seated_seconds, 0);
    assert_eq!(heartbeat.day.as_ref().unwrap().suspend_seconds, 121);
    assert!(heartbeat.day.as_ref().unwrap().pauses.last().unwrap().auto);
}

#[test]
fn jumps_match_second_by_second_across_timer_pause_rest_and_idle_states() {
    let mut running = started();
    running.preferences.stretch_reminder_enabled = true;
    running.preferences.stretch_reminder_minutes = 5;
    running.preferences.idle_reminder_enabled = true;
    running.preferences.idle_reminder_minutes = 5;
    running.preferences.water_reminder_enabled = true;
    let idle = running.clone();
    running
        .start_block_with_break("main", 10, vec![], 2)
        .unwrap();
    let mut no_rest = running.clone();
    no_rest
        .day
        .as_mut()
        .unwrap()
        .timer
        .as_mut()
        .unwrap()
        .break_minutes = 0;
    let mut paused = running.clone();
    walk_wall_clock(&mut paused, 1_179, 945);
    paused.toggle_pause(paused.last_tick).unwrap();
    let mut resting = running.clone();
    walk_wall_clock(&mut resting, 1_679, 945);
    let mut long_pause = resting.clone();
    walk_wall_clock(&mut long_pause, 4_320, 945);
    for initial in [
        State::new(1_000),
        idle,
        running,
        no_rest,
        paused,
        resting,
        long_pause,
    ] {
        for gap in [0, 1, 2, 3, 119, 120, 121, 299, 300, 301, 3_600, 43_200] {
            assert_jump_equivalent(&initial, gap, 945);
        }
    }
}

struct SeededRng(u64);

impl SeededRng {
    fn below(&mut self, limit: u64) -> u64 {
        // 固定种子的 xorshift64，仅用于覆盖状态组合，不引入随机数依赖。
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % limit
    }
}

#[test]
fn fixed_seed_generated_states_match_second_by_second_advancement() {
    let mut rng = SeededRng(0x5172_2026_1002);
    for case in 0..128 {
        let mut state = started();
        state.preferences.stretch_reminder_enabled = rng.below(2) == 1;
        state.preferences.idle_reminder_enabled = rng.below(2) == 1;
        state.preferences.water_reminder_enabled = rng.below(2) == 1;
        state.preferences.stretch_reminder_minutes = [5, 6, 15, 50, 240][rng.below(5) as usize];
        state.preferences.idle_reminder_minutes = [5, 6, 15, 50, 240][rng.below(5) as usize];
        state.preferences.break_minutes = [0, 1, 2, 10, 120][rng.below(5) as usize];
        let offset = rng.below(3_600) as i64;
        for _ in 0..12 {
            match rng.below(6) {
                0 if state.day.as_ref().unwrap().timer.is_none() => {
                    let minutes = rng.below(20) as i64 + 1;
                    let category = ["main", "reading", "writing", "browse"][rng.below(4) as usize];
                    state
                        .start_block(
                            category,
                            minutes,
                            vec![TaskItem {
                                text: "记录任务".into(),
                                done: false,
                            }],
                        )
                        .unwrap();
                }
                1 if state.day.as_ref().unwrap().timer.is_some() => {
                    state.toggle_pause(state.last_tick).unwrap();
                }
                2 if state.day.as_ref().unwrap().timer.is_some() => {
                    state.finish_block(state.last_tick).unwrap();
                }
                3 if state.day.as_ref().unwrap().timer.is_some() => {
                    state.abandon_block(state.last_tick).unwrap();
                }
                4 if state.day.as_ref().unwrap().break_until.is_some() => {
                    state.end_break().unwrap();
                }
                _ => {
                    state = from_json(&to_json(&state)).unwrap();
                }
            }
            let target = state.last_tick + rng.below(601) as i64;
            walk_wall_clock(&mut state, target, offset);
        }
        let gap = if case % 8 == 0 {
            (case / 8) % 4
        } else {
            rng.below(10_801) as i64
        };
        assert_jump_equivalent(&state, gap, offset);
    }
}

#[test]
fn a_quick_new_block_with_an_already_due_stretch_counter_is_jump_equivalent() {
    let mut state = started();
    state.preferences.stretch_reminder_enabled = true;
    state.preferences.stretch_reminder_minutes = 5;
    state.start_block_with_break("main", 5, vec![], 0).unwrap();
    walk_wall_clock(&mut state, 1_300, 0);
    state.start_block_with_break("main", 5, vec![], 0).unwrap();
    assert_jump_equivalent(&state, 3, 0);
}

#[test]
fn enabling_reminders_with_overdue_counters_is_jump_equivalent() {
    let mut running = started();
    running.preferences.stretch_reminder_minutes = 5;
    running.start_block("main", 20, vec![]).unwrap();
    running.tick_wall_clock(1_600);
    let mut prefs = running.preferences.clone();
    prefs.stretch_reminder_enabled = true;
    running.update_preferences(prefs).unwrap();

    let mut idle = started();
    idle.preferences.idle_reminder_minutes = 5;
    idle.tick_wall_clock(1_600);
    let mut prefs = idle.preferences.clone();
    prefs.idle_reminder_enabled = true;
    idle.update_preferences(prefs).unwrap();

    for initial in [running, idle] {
        for gap in [1, 2, 3, 1_200] {
            assert_jump_equivalent(&initial, gap, 0);
        }
    }
}

#[test]
fn an_overdue_stretch_counter_is_not_consumed_if_the_block_finishes_next_second() {
    let mut state = started();
    state.preferences.stretch_reminder_minutes = 5;
    state.start_block("main", 20, vec![]).unwrap();
    state.tick_wall_clock(2_199);
    let mut prefs = state.preferences.clone();
    prefs.stretch_reminder_enabled = true;
    state.update_preferences(prefs).unwrap();
    assert_jump_equivalent(&state, 3, 0);
}

#[test]
fn a_legacy_missing_pause_span_is_repaired_before_counting_post_rest_idle() {
    let mut state = started();
    state.start_block_with_break("main", 1, vec![], 1).unwrap();
    walk_wall_clock(&mut state, 1_060, 0);
    state.day.as_mut().unwrap().pauses.clear();
    for gap in [1, 2, 3, 120] {
        assert_jump_equivalent(&state, gap, 0);
    }
}
