use jiff::civil::Time;
use jiff::{SignedDuration, Zoned};

use crate::config::Config;

pub const GOAL_REACHED: &str = "done";
pub const NIGHTLY: &str = "nightly";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Due {
    Remind { slot: Time },
    GoalReached,
    Nightly,
}

impl Due {
    pub fn kind(&self) -> String {
        match self {
            Self::Remind { slot } => format!("remind@{}", slot.strftime("%H:%M")),
            Self::GoalReached => GOAL_REACHED.into(),
            Self::Nightly => NIGHTLY.into(),
        }
    }
}

pub fn reminder_slots(config: &Config) -> Vec<Time> {
    let mut slots = Vec::new();
    let mut slot = config.remind.start;
    while slot < config.nightly.at {
        slots.push(slot);
        match slot.checked_add(config.remind.interval) {
            Ok(next) if next > slot => slot = next,
            _ => break,
        }
    }
    slots
}

pub fn due(
    config: &Config,
    now: &Zoned,
    total: SignedDuration,
    already: impl Fn(&str) -> bool,
) -> Option<Due> {
    let time = now.time();
    let candidate = if total >= config.daily_goal() {
        Due::GoalReached
    } else if time >= config.nightly.at {
        Due::Nightly
    } else {
        let slot = reminder_slots(config)
            .into_iter()
            .rfind(|slot| *slot <= time)?;
        if time.duration_since(slot) >= config.remind.interval {
            return None;
        }
        Due::Remind { slot }
    };
    (!already(&candidate.kind())).then_some(candidate)
}

#[cfg(test)]
mod tests {

    use super::*;

    fn config() -> Config {
        Config::parse("").unwrap()
    }

    fn at(time: &str) -> Zoned {
        format!("2026-10-16T{time}[Australia/Sydney]")
            .parse()
            .unwrap()
    }

    fn mins(minutes: i64) -> SignedDuration {
        SignedDuration::from_mins(minutes)
    }

    fn never(_: &str) -> bool {
        false
    }

    #[test]
    fn default_slots_run_every_two_hours_until_nine_pm() {
        // given
        let config = config();

        // when
        let slots: Vec<String> = reminder_slots(&config)
            .iter()
            .map(|t| t.strftime("%H:%M").to_string())
            .collect();

        // then
        assert_eq!(
            slots,
            ["09:00", "11:00", "13:00", "15:00", "17:00", "19:00"]
        );
    }

    #[test]
    fn reminder_is_due_in_the_current_slot_while_under_goal() {
        // given
        let now = at("11:20");

        // when
        let due = due(&config(), &now, mins(12), never);

        // then
        assert_eq!(
            due,
            Some(Due::Remind {
                slot: Time::constant(11, 0, 0, 0)
            })
        );
    }

    #[test]
    fn reminder_is_sent_once_per_slot() {
        // given
        let now = at("11:20");

        // when
        let due = due(&config(), &now, mins(12), |kind| kind == "remind@11:00");

        // then
        assert_eq!(due, None);
    }

    #[test]
    fn nothing_is_due_before_the_first_slot() {
        // given
        let now = at("07:30");

        // when
        let due = due(&config(), &now, mins(0), never);

        // then
        assert_eq!(due, None);
    }

    #[test]
    fn goal_reached_is_announced_once_and_stops_reminders() {
        // given
        let now = at("13:10");

        // when
        let first = due(&config(), &now, mins(20), never);
        let after = due(&config(), &now, mins(25), |kind| kind == GOAL_REACHED);

        // then
        assert_eq!(first, Some(Due::GoalReached));
        assert_eq!(after, None);
    }

    #[test]
    fn nightly_check_is_due_at_nine_pm_when_under_goal() {
        // given
        let now = at("21:00");

        // when
        let due = due(&config(), &now, mins(8), never);

        // then
        assert_eq!(due, Some(Due::Nightly));
    }

    #[test]
    fn nightly_check_after_wake_runs_once_the_same_day() {
        // given
        let now = at("23:40");

        // when
        let first = due(&config(), &now, mins(8), never);
        let second = due(&config(), &now, mins(8), |kind| kind == NIGHTLY);

        // then
        assert_eq!(first, Some(Due::Nightly));
        assert_eq!(second, None);
    }

    #[test]
    fn nightly_time_is_configurable() {
        // given
        let config = Config::parse("[nightly]\nat = \"22:30\"").unwrap();

        // when
        let before = due(&config, &at("21:30"), mins(8), never);
        let after = due(&config, &at("22:30"), mins(8), never);

        // then
        assert_eq!(
            before,
            Some(Due::Remind {
                slot: Time::constant(21, 0, 0, 0)
            })
        );
        assert_eq!(after, Some(Due::Nightly));
    }
}
