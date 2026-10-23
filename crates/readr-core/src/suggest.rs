use std::collections::HashMap;

use jiff::{SignedDuration, Timestamp};

use crate::config::Config;
use crate::db::Book;
use crate::model::BookStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion<'a> {
    pub book: &'a Book,
    pub reason: String,
}

pub fn suggest<'a>(
    config: &Config,
    books: &'a [Book],
    last_read: &HashMap<i64, Timestamp>,
    week_by_goal: &HashMap<String, SignedDuration>,
) -> Option<Suggestion<'a>> {
    let active: Vec<&Book> = books
        .iter()
        .filter(|book| book.status == BookStatus::Active)
        .collect();
    let goals_with_books: Vec<_> = config
        .goals
        .iter()
        .filter(|goal| {
            active
                .iter()
                .any(|b| b.goal.as_deref() == Some(goal.key.as_str()))
        })
        .collect();

    let default_share = SignedDuration::from_mins(
        i64::from(config.daily_goal_minutes) * 7 / goals_with_books.len().max(1) as i64,
    );
    let behind = goals_with_books.iter().min_by(|a, b| {
        let ratio = |goal: &&&crate::config::Goal| {
            let read = week_by_goal.get(&goal.key).copied().unwrap_or_default();
            let target = goal
                .weekly_minutes
                .map(|m| SignedDuration::from_mins(i64::from(m)))
                .unwrap_or(default_share)
                .max(SignedDuration::from_mins(1));
            read.as_secs_f64() / target.as_secs_f64()
        };
        ratio(a).total_cmp(&ratio(b))
    });

    let (pool, reason): (Vec<&Book>, String) = match behind {
        Some(goal) => {
            let read = week_by_goal.get(&goal.key).copied().unwrap_or_default();
            let pool = active
                .iter()
                .copied()
                .filter(|b| b.goal.as_deref() == Some(goal.key.as_str()))
                .collect();
            let reason = match goal.weekly_minutes {
                Some(target) => format!(
                    "{} · {}/{target} min this week",
                    goal.label,
                    read.as_secs() / 60
                ),
                None => format!("{} · least read this week", goal.label),
            };
            (pool, reason)
        }
        None => (active.clone(), "on your shelf".into()),
    };

    pool.into_iter()
        .max_by(|a, b| {
            let key = |book: &Book| (last_read.get(&book.id).copied(), -book.priority, -book.id);
            key(a).cmp(&key(b))
        })
        .map(|book| Suggestion { book, reason })
}

#[cfg(test)]
mod tests {

    use super::*;

    fn config(text: &str) -> Config {
        Config::parse(text).unwrap()
    }

    fn goals() -> &'static str {
        r#"
        [[goal]]
        key = "health"
        label = "Health"

        [[goal]]
        key = "communication"
        label = "Communication"
        "#
    }

    fn book(id: i64, goal: &str, status: BookStatus) -> Book {
        Book {
            id,
            title: format!("book {id}"),
            author: None,
            goal: Some(goal.into()),
            status,
            priority: 0,
            added_at: Timestamp::UNIX_EPOCH,
            finished_at: None,
        }
    }

    fn week(pairs: &[(&str, i64)]) -> HashMap<String, SignedDuration> {
        pairs
            .iter()
            .map(|(goal, mins)| (goal.to_string(), SignedDuration::from_mins(*mins)))
            .collect()
    }

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn suggests_the_goal_read_least_this_week() {
        // given
        let books = vec![
            book(1, "health", BookStatus::Active),
            book(2, "communication", BookStatus::Active),
        ];

        // when
        let suggestion = suggest(
            &config(goals()),
            &books,
            &HashMap::new(),
            &week(&[("health", 60), ("communication", 10)]),
        )
        .unwrap();

        // then
        assert_eq!(suggestion.book.id, 2);
        assert_eq!(suggestion.reason, "Communication · least read this week");
    }

    #[test]
    fn weekly_targets_decide_which_goal_is_behind() {
        // given
        let text = r#"
        [[goal]]
        key = "health"
        label = "Health"
        weekly_minutes = 120

        [[goal]]
        key = "communication"
        label = "Communication"
        weekly_minutes = 20
        "#;
        let books = vec![
            book(1, "health", BookStatus::Active),
            book(2, "communication", BookStatus::Active),
        ];

        // when
        let suggestion = suggest(
            &config(text),
            &books,
            &HashMap::new(),
            &week(&[("health", 50), ("communication", 15)]),
        )
        .unwrap();

        // then
        assert_eq!(suggestion.book.id, 1);
        assert_eq!(suggestion.reason, "Health · 50/120 min this week");
    }

    #[test]
    fn within_a_goal_the_most_recently_read_book_wins() {
        // given
        let books = vec![
            book(1, "health", BookStatus::Active),
            book(2, "health", BookStatus::Active),
        ];
        let last_read = HashMap::from([
            (1, ts("2026-10-10T09:00:00Z")),
            (2, ts("2026-10-15T09:00:00Z")),
        ]);

        // when
        let suggestion = suggest(&config(goals()), &books, &last_read, &HashMap::new()).unwrap();

        // then
        assert_eq!(suggestion.book.id, 2);
    }

    #[test]
    fn queued_and_finished_books_are_not_suggested() {
        // given
        let books = vec![
            book(1, "health", BookStatus::Queued),
            book(2, "health", BookStatus::Done),
        ];

        // when
        let suggestion = suggest(&config(goals()), &books, &HashMap::new(), &HashMap::new());

        // then
        assert_eq!(suggestion, None);
    }

    #[test]
    fn books_without_a_configured_goal_are_still_suggested() {
        // given
        let books = vec![book(1, "travel", BookStatus::Active)];

        // when
        let suggestion =
            suggest(&config(goals()), &books, &HashMap::new(), &HashMap::new()).unwrap();

        // then
        assert_eq!(suggestion.book.id, 1);
        assert_eq!(suggestion.reason, "on your shelf");
    }
}
