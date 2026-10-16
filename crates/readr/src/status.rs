use std::fmt::Write;

use anyhow::Result;
use jiff::SignedDuration;
use readr_core::db::SessionView;
use readr_core::model::Position;
use readr_core::stats::{format_minutes, minutes, streak};

use crate::context::App;
use crate::next::NextUp;

pub struct Today {
    pub total: SignedDuration,
    pub goal: SignedDuration,
    pub streak: u32,
    pub sessions: Vec<SessionView>,
}

impl Today {
    pub fn load(app: &App) -> Result<Self> {
        let today = app.today();
        let from = today.saturating_sub(jiff::Span::new().days(400));
        let totals = app
            .db
            .daily_totals(from, today, app.config.count_off_shelf)?;
        let goal = app.config.daily_goal();
        Ok(Self {
            total: totals.get(&today).copied().unwrap_or_default(),
            goal,
            streak: streak(&totals, today, goal),
            sessions: app.db.sessions_between(today, today)?,
        })
    }

    pub fn done(&self) -> bool {
        self.total >= self.goal
    }
}

pub fn short(today: &Today) -> String {
    format!("{}/{}m", minutes(today.total), minutes(today.goal))
}

pub fn long(app: &App, today: &Today, next: Option<&NextUp>) -> String {
    let mut out = String::new();
    let mark = if today.done() { "  done" } else { "" };
    let _ = writeln!(
        out,
        "Today {}/{} min · streak {}{mark}",
        minutes(today.total),
        minutes(today.goal),
        today.streak
    );
    if today.sessions.is_empty() {
        let _ = writeln!(out, "  no reading yet");
    }
    for session in &today.sessions {
        let started = app.local(session.started_at);
        let reading = if session.open { "  (reading)" } else { "" };
        let off_shelf = if session.book_id.is_none() {
            "  (off shelf)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {}  {:<28} {:<9} {:>5}  {}{reading}{off_shelf}",
            started.strftime("%H:%M"),
            truncate(&session.title, 28),
            session.reader.label(),
            format_minutes(session.active),
            movement(&session.start_position, &session.end_position),
        );
    }
    if let Some(next) = next {
        let _ = writeln!(out, "Up next  {}  ({})", next.describe(), next.reason);
    }
    out
}

fn movement(start: &Option<Position>, end: &Option<Position>) -> String {
    match (start, end) {
        (Some(start), Some(end)) if start != end => {
            format!("{} → {}", start.display(), end.display())
        }
        (_, Some(end)) => end.display(),
        (Some(start), None) => start.display(),
        (None, None) => String::new(),
    }
}

pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
