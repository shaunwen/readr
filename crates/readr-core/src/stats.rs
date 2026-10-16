use std::collections::BTreeMap;

use jiff::SignedDuration;
use jiff::civil::Date;

pub fn streak(totals: &BTreeMap<Date, SignedDuration>, today: Date, goal: SignedDuration) -> u32 {
    let met = |day: Date| totals.get(&day).is_some_and(|total| *total >= goal);
    let mut day = if met(today) {
        today
    } else {
        match today.yesterday() {
            Ok(yesterday) => yesterday,
            Err(_) => return 0,
        }
    };
    let mut count = 0;
    while met(day) {
        count += 1;
        match day.yesterday() {
            Ok(previous) => day = previous,
            Err(_) => break,
        }
    }
    count
}

pub fn best_streak(totals: &BTreeMap<Date, SignedDuration>, goal: SignedDuration) -> u32 {
    let mut best = 0;
    let mut current = 0;
    let mut previous: Option<Date> = None;
    for (day, total) in totals {
        if *total < goal {
            current = 0;
            previous = None;
            continue;
        }
        let consecutive = previous.is_some_and(|p| p.tomorrow().ok() == Some(*day));
        current = if consecutive { current + 1 } else { 1 };
        best = best.max(current);
        previous = Some(*day);
    }
    best
}

pub fn minutes(duration: SignedDuration) -> i64 {
    duration.as_secs() / 60
}

pub fn remaining(total: SignedDuration, goal: SignedDuration) -> SignedDuration {
    (goal - total).max(SignedDuration::ZERO)
}

pub fn format_minutes(duration: SignedDuration) -> String {
    let total = minutes(duration);
    if total >= 60 {
        format!("{}h{:02}m", total / 60, total % 60)
    } else {
        format!("{total}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(days: &[(i8, i64)]) -> BTreeMap<Date, SignedDuration> {
        days.iter()
            .map(|(day, mins)| {
                (
                    Date::constant(2026, 10, *day),
                    SignedDuration::from_mins(*mins),
                )
            })
            .collect()
    }

    fn goal() -> SignedDuration {
        SignedDuration::from_mins(20)
    }

    #[test]
    fn streak_includes_today_once_goal_is_met() {
        // given
        let totals = totals(&[(14, 25), (15, 20), (16, 21)]);

        // when
        let streak = streak(&totals, Date::constant(2026, 10, 16), goal());

        // then
        assert_eq!(streak, 3);
    }

    #[test]
    fn unfinished_today_keeps_yesterdays_streak_alive() {
        // given
        let totals = totals(&[(14, 25), (15, 20), (16, 5)]);

        // when
        let streak = streak(&totals, Date::constant(2026, 10, 16), goal());

        // then
        assert_eq!(streak, 2);
    }

    #[test]
    fn a_missed_day_breaks_the_streak() {
        // given
        let totals = totals(&[(13, 30), (14, 10), (15, 20)]);

        // when
        let streak = streak(&totals, Date::constant(2026, 10, 16), goal());

        // then
        assert_eq!(streak, 1);
    }

    #[test]
    fn best_streak_finds_the_longest_run() {
        // given
        let totals = totals(&[(1, 20), (2, 20), (3, 20), (5, 20), (6, 3), (7, 40)]);

        // when
        let best = best_streak(&totals, goal());

        // then
        assert_eq!(best, 3);
    }

    #[test]
    fn remaining_never_goes_negative() {
        // given
        let total = SignedDuration::from_mins(31);

        // when
        let left = remaining(total, goal());

        // then
        assert_eq!(left, SignedDuration::ZERO);
    }

    #[test]
    fn long_durations_show_hours() {
        // given
        let duration = SignedDuration::from_mins(95);

        // when
        let shown = format_minutes(duration);

        // then
        assert_eq!(shown, "1h35m");
    }
}
