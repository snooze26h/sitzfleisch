use crate::{State, WATER_REMINDER_MINUTES};
use serde::{Deserialize, Serialize};

const MAX_PERIODIC_ALERTS: usize = 72;
const MAX_WATER_ALERTS: usize = 24;
const REMINDER_HORIZON_SECONDS: i128 = 12 * 3_600;

/// 同一秒的提醒按这个顺序排列，供外壳稳定比较预排结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertKind {
    BlockFinished,
    RestOver,
    Stretch,
    Idle,
    Water,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedAlert {
    pub kind: AlertKind,
    pub at: i64,
    /// 闲置与喝水提醒发生在暂停中时，供外壳计算正文时长和长暂停过滤。
    pub pause_started_at: Option<i64>,
}

impl State {
    /// 心跳取完本轮 take_due_* 后调用；last_tick 和 water_clock_checked_at 应已对齐。
    /// 非正 horizon 不预排；本地钟面秒数不在 0..3600 时只跳过喝水提醒。
    /// 不改变状态，也不处理移动端专有的文案、权限或长暂停过滤。
    pub fn planned_alerts(&self, local_seconds_in_hour: u32, horizon: i64) -> Vec<PlannedAlert> {
        if horizon <= 0 {
            return Vec::new();
        }
        // 用宽位整数算未来时刻，损坏存档或过大的 horizon 不能溢出，也不能造出截断的闹钟。
        let now = i128::from(self.last_tick);
        let until = now + i128::from(horizon);
        let reminder_until = until.min(now + REMINDER_HORIZON_SECONDS);
        let day = self.day.as_ref();
        let running_timer = day
            .filter(|day| !day.is_paused())
            .and_then(|day| day.timer.as_ref());
        let block_end = running_timer.map(|timer| {
            now + (i128::from(timer.total_seconds) - i128::from(timer.elapsed_seconds)).max(1)
        });
        let rest = running_timer.map_or(0, |timer| {
            let minutes = if timer.break_minutes >= 0 {
                timer.break_minutes
            } else {
                self.preferences.break_minutes.max(0)
            };
            i128::from(minutes) * 60
        });
        let pause_started_at = day
            .and_then(|day| day.pauses.last())
            .map(|pause| pause.started_at);
        let mut alerts = Vec::new();
        let mut push = |kind, at, pause_started_at| {
            if now < at && at <= until {
                if let Ok(at) = i64::try_from(at) {
                    alerts.push(PlannedAlert {
                        kind,
                        at,
                        pause_started_at,
                    });
                }
            }
        };

        if let Some(end) = block_end {
            push(AlertKind::BlockFinished, end, None);
            if rest > 0 {
                push(AlertKind::RestOver, end + rest, None);
            }
        } else if let Some(until) = day
            .filter(|day| day.timer.is_none())
            .and_then(|day| day.break_until)
        {
            push(AlertKind::RestOver, i128::from(until), None);
        }

        if self.preferences.stretch_reminder_enabled
            && self.preferences.stretch_reminder_minutes > 0
        {
            if let (Some(day), Some(end)) = (day, block_end) {
                let interval = i128::from(self.preferences.stretch_reminder_minutes) * 60;
                let mut at = now + (interval - i128::from(day.seated_since_relief)).max(1);
                for _ in 0..MAX_PERIODIC_ALERTS {
                    // 格在这个终点先进入暂停，所以终点同秒不再触发起身提醒。
                    if at >= end || at > until {
                        break;
                    }
                    push(AlertKind::Stretch, at, None);
                    at += interval;
                }
            }
        }

        if self.preferences.idle_reminder_enabled && self.preferences.idle_reminder_minutes > 0 {
            if let Some(day) = day.filter(|day| block_end.is_some() || day.timer.is_none()) {
                let interval = i128::from(self.preferences.idle_reminder_minutes) * 60;
                let (mut at, pause_start) = if let Some(end) = block_end {
                    (end + rest + interval, i64::try_from(end).ok())
                } else if let Some(until) = day.break_until.filter(|until| i128::from(*until) > now)
                {
                    (
                        i128::from(until) + interval - i128::from(day.paused_without_block),
                        pause_started_at,
                    )
                } else {
                    (
                        now + (interval - i128::from(day.paused_without_block)).max(1),
                        pause_started_at,
                    )
                };
                if at <= now {
                    at += ((now - at) / interval + 1) * interval;
                }
                for _ in 0..MAX_PERIODIC_ALERTS {
                    if at > reminder_until {
                        break;
                    }
                    push(AlertKind::Idle, at, pause_start);
                    at += interval;
                }
            }
        }

        if self.preferences.water_reminder_enabled && local_seconds_in_hour < 3_600 {
            let interval = i128::from(WATER_REMINDER_MINUTES) * 60;
            let first_tick = now + 1;
            let reminded_at = self.water_reminded_at.map(i128::from).map(|at| {
                if at > first_tick + interval {
                    first_tick
                } else {
                    at
                }
            });
            let mut at = now - i128::from(local_seconds_in_hour) % interval + interval;
            for _ in 0..MAX_WATER_ALERTS {
                if at > reminder_until {
                    break;
                }
                if reminded_at.is_none_or(|last| at > last) {
                    let pause_start = if let Some(end) = block_end {
                        if at >= end {
                            i64::try_from(end).ok()
                        } else {
                            None
                        }
                    } else {
                        day.filter(|day| day.is_paused()).and(pause_started_at)
                    };
                    push(AlertKind::Water, at, pause_start);
                }
                at += interval;
            }
        }

        alerts.sort_unstable_by_key(|alert| (alert.at, alert.kind));
        alerts
    }
}
