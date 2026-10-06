use sitzfleisch_core as core;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimePolicy {
    Heartbeat,
    WallClock,
}

pub fn advance_time(state: &mut core::State, now: i64, policy: TimePolicy) {
    match policy {
        TimePolicy::Heartbeat => state.tick(now),
        TimePolicy::WallClock => state.tick_wall_clock(now),
    }
}

pub fn resume_time(state: &mut core::State, now: i64, policy: TimePolicy) {
    match policy {
        TimePolicy::Heartbeat => state.resume_after_restart(now),
        TimePolicy::WallClock => state.resume_after_restart_wall_clock(now),
    }
}

pub fn show_due_banners(policy: TimePolicy, gap: i64) -> bool {
    policy == TimePolicy::Heartbeat || gap <= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> core::State {
        let mut state = core::State::new(1000);
        state.start_day("standard", 1000).unwrap();
        let category = state.day.as_ref().unwrap().categories[0].id.clone();
        state.start_block_with_break(&category, 3, Vec::new(), 3).unwrap();
        state
    }

    #[test]
    fn wall_clock_gap_settles_at_the_real_end_and_counts_rest_from_that_end() {
        let mut state = running();
        advance_time(&mut state, 1360, TimePolicy::WallClock);
        let day = state.day.unwrap();
        assert!(day.timer.is_none());
        assert_eq!(day.ledger[0].seconds, 180);
        assert_eq!(day.ledger[0].ended_at, 1180);
        assert_eq!(day.rests[0].started_at, 1180);
        assert_eq!(day.rests[0].ended_at, 1360);
        assert_eq!(day.suspend_seconds, 0);
        assert!(day.pauses.iter().all(|pause| !pause.auto));
    }

    #[test]
    fn wall_clock_cold_start_keeps_a_live_block_running_across_a_large_gap() {
        let mut state = running();
        resume_time(&mut state, 1135, TimePolicy::WallClock);
        let day = state.day.unwrap();
        assert_eq!(day.timer.unwrap().elapsed_seconds, 135);
        assert_eq!(day.suspend_seconds, 0);
        assert!(day.pauses.iter().all(|pause| !pause.auto));
    }

    #[test]
    fn desktop_policy_matches_original_tick_and_restart_byte_for_byte() {
        for restart in [false, true] {
            let original = running();
            let mut expected = original.clone();
            let mut actual = original;
            if restart {
                expected.resume_after_restart(1360);
                resume_time(&mut actual, 1360, TimePolicy::Heartbeat);
            } else {
                expected.tick(1360);
                advance_time(&mut actual, 1360, TimePolicy::Heartbeat);
            }
            assert_eq!(core::to_json(&actual), core::to_json(&expected));
        }
    }

    #[test]
    fn mobile_does_not_replay_expired_banners_but_desktop_keeps_its_behavior() {
        for gap in [-1, 0, 1, 2, 3, 600] {
            assert!(show_due_banners(TimePolicy::Heartbeat, gap));
            assert_eq!(show_due_banners(TimePolicy::WallClock, gap), gap <= 2);
        }
    }
}
