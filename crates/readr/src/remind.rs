use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use readr_core::schedule::{Due, due};
use readr_core::stats::{minutes, remaining};

use crate::context::App;
use crate::daemon::log;
use crate::next::next_up;
use crate::status::Today;
use crate::{email, notify};

const NIGHTLY_NOTICE: &str = "nightly-notice";
const EMAIL_RETRY: SignedDuration = SignedDuration::from_mins(10);

#[derive(Default)]
pub struct Reminders {
    last_email_attempt: Option<Timestamp>,
}

impl Reminders {
    pub fn tick(&mut self, app: &App) -> Result<()> {
        let now = app.now();
        let day = now.date();
        let today = Today::load(app)?;
        let already = |kind: &str| app.db.notified(day, kind).unwrap_or(false);
        let Some(due) = due(&app.config, &now, today.total, already) else {
            return Ok(());
        };
        let next = next_up(app)?.map(|next| next.describe());
        let left = minutes(remaining(today.total, today.goal));
        match &due {
            Due::Remind { .. } => {
                let (title, body) = remind_text(left, next.as_deref());
                send_notification(app, &title, &body);
                app.db.mark_notified(day, &due.kind(), now.timestamp())?;
            }
            Due::GoalReached => {
                let (title, body) = done_text(minutes(today.total), today.streak);
                send_notification(app, &title, &body);
                app.db.mark_notified(day, &due.kind(), now.timestamp())?;
            }
            Due::Nightly => {
                if !already(NIGHTLY_NOTICE) {
                    let (title, body) = remind_text(left, next.as_deref());
                    send_notification(app, &title, &body);
                    app.db.mark_notified(day, NIGHTLY_NOTICE, now.timestamp())?;
                }
                let Some(settings) = &app.config.nightly.email else {
                    app.db.mark_notified(day, &due.kind(), now.timestamp())?;
                    return Ok(());
                };
                let waited = self
                    .last_email_attempt
                    .is_none_or(|at| now.timestamp().duration_since(at) >= EMAIL_RETRY);
                if !waited {
                    return Ok(());
                }
                self.last_email_attempt = Some(now.timestamp());
                let (subject, body) = nightly_email(
                    minutes(today.total),
                    minutes(today.goal),
                    today.streak,
                    next.as_deref(),
                );
                match email::send(settings, &subject, &body) {
                    Ok(()) => {
                        app.db.mark_notified(day, &due.kind(), now.timestamp())?;
                        log(&format!("nightly email sent to {}", settings.to));
                    }
                    Err(error) => log(&format!("nightly email failed, retrying: {error:#}")),
                }
            }
        }
        Ok(())
    }
}

fn send_notification(app: &App, title: &str, body: &str) {
    match notify::notify(app.config.notify.channel, title, body) {
        Ok(delivery) => log(&format!("notified via {}: {title}", delivery.describe())),
        Err(error) => log(&format!("{error:#}")),
    }
}

pub fn remind_text(left: i64, next: Option<&str>) -> (String, String) {
    let title = format!("readr: {left} min to go today");
    let body = match next {
        Some(next) => format!("{next} · readr go"),
        None => "Open a book on your shelf · readr".into(),
    };
    (title, body)
}

pub fn done_text(total: i64, streak: u32) -> (String, String) {
    let days = if streak == 1 { "day" } else { "days" };
    (
        "readr: done for today".into(),
        format!("{total} min read · streak {streak} {days}"),
    )
}

pub fn nightly_email(total: i64, goal: i64, streak: u32, next: Option<&str>) -> (String, String) {
    let left = (goal - total).max(0);
    let subject = format!("readr: {left} min left today");
    let mut body = format!("You have read {total} of {goal} minutes today.\n");
    if streak > 0 {
        body.push_str(&format!(
            "Your {streak}-day streak ends tonight unless you read {left} more minutes.\n"
        ));
    }
    match next {
        Some(next) => body.push_str(&format!("\nPick up: {next}\nRun `readr go` to open it.\n")),
        None => body.push_str("\nAdd a book to your shelf with `readr shelf add`.\n"),
    }
    (subject, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_names_the_book_and_page_to_resume() {
        // given
        let next = Some("Walden p.9/300 (bookokrat)");

        // when
        let (title, body) = remind_text(8, next);

        // then
        assert_eq!(title, "readr: 8 min to go today");
        assert_eq!(body, "Walden p.9/300 (bookokrat) · readr go");
    }

    #[test]
    fn done_message_reports_the_streak() {
        // given
        let (total, streak) = (22, 1);

        // when
        let (title, body) = done_text(total, streak);

        // then
        assert_eq!(title, "readr: done for today");
        assert_eq!(body, "22 min read · streak 1 day");
    }

    #[test]
    fn nightly_email_warns_about_the_streak_at_risk() {
        // given
        let next = Some("Meditations 30% (Books)");

        // when
        let (subject, body) = nightly_email(12, 20, 6, next);

        // then
        assert_eq!(subject, "readr: 8 min left today");
        assert!(body.contains("You have read 12 of 20 minutes today."));
        assert!(body.contains("Your 6-day streak ends tonight unless you read 8 more minutes."));
        assert!(body.contains("Pick up: Meditations 30% (Books)"));
    }

    #[test]
    fn nightly_email_without_streak_or_shelf_still_helps() {
        // given
        let next = None;

        // when
        let (_, body) = nightly_email(0, 20, 0, next);

        // then
        assert!(!body.contains("streak"));
        assert!(body.contains("readr shelf add"));
    }
}
