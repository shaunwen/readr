use std::fmt::Write;

use anyhow::{Context, Result, bail};
use jiff::{SignedDuration, Span, Timestamp};
use readr_core::db::Book;
use readr_core::find::find_sources;
use readr_core::model::Reader;
use readr_core::stats::{best_streak, format_minutes, minutes, streak};

use crate::context::App;
use crate::launch;
use crate::next::next_up;
use crate::shelf::pick_book;
use crate::status::truncate;

pub fn go(app: &App, query: Option<&str>) -> Result<String> {
    let book = match query {
        Some(query) => pick_book(app, query)?,
        None => {
            next_up(app)?
                .context("nothing on the shelf yet; add a book with `readr shelf add`")?
                .book
        }
    };
    open_book(app, &book)
}

pub fn open_book(app: &App, book: &Book) -> Result<String> {
    let source = app
        .db
        .sources_for_book(book.id)?
        .into_iter()
        .find(|s| s.reader != Reader::Manual)
        .with_context(|| format!("{} has no reader source yet", book.title))?;
    launch::open(&source, &book.title, app.config.launch.terminal)?;
    let position = source
        .last_position
        .as_ref()
        .map(|p| format!(" at {}", p.display()))
        .unwrap_or_default();
    Ok(format!(
        "opening {}{position} in {}\n",
        book.title,
        source.reader.label()
    ))
}

pub fn log(app: &App, duration: &str, query: &str, note: Option<&str>) -> Result<String> {
    let duration: SignedDuration = duration
        .parse()
        .with_context(|| format!("{duration:?} is not a duration like 25m or 1h10m"))?;
    if duration <= SignedDuration::ZERO || duration > SignedDuration::from_hours(12) {
        bail!("duration must be between 1 minute and 12 hours");
    }
    let book = pick_book(app, query)?;
    let now = Timestamp::now();
    app.db
        .add_manual_session(book.id, now - duration, app.today(), duration, note)?;
    let total = app.db.day_total(app.today(), app.config.count_off_shelf)?;
    Ok(format!(
        "logged {} for {} · today {}/{} min\n",
        format_minutes(duration),
        book.title,
        minutes(total),
        app.config.daily_goal_minutes
    ))
}

pub fn inbox(app: &App) -> Result<String> {
    let sources = app.db.inbox_sources()?;
    if sources.is_empty() {
        return Ok("inbox is empty\n".into());
    }
    let today = app.today();
    let sessions = app
        .db
        .sessions_between(today.saturating_sub(Span::new().days(3650)), today)?;
    let mut out = String::new();
    for source in &sources {
        let time: SignedDuration = sessions
            .iter()
            .filter(|s| s.source_id == source.id)
            .map(|s| s.active)
            .sum();
        let _ = writeln!(
            out,
            "{:<40} {:<9} {:>6}  {}",
            truncate(&source.display_title(), 40),
            source.reader.label(),
            format_minutes(time),
            source.locator
        );
    }
    let _ = writeln!(
        out,
        "\nadd one with `readr shelf add <title>`, link it with `readr inbox link <source> <book>`, or `readr inbox ignore <source>`"
    );
    Ok(out)
}

pub fn inbox_link(app: &App, source_query: &str, book_query: &str) -> Result<String> {
    let sources = app.db.inbox_sources()?;
    let found = find_sources(&sources, source_query);
    let [source] = found.as_slice() else {
        bail!(
            "{} inbox items match {source_query:?}; be more specific",
            found.len()
        );
    };
    let book = pick_book(app, book_query)?;
    app.db.link_source(source.id, Some(book.id))?;
    Ok(format!(
        "linked {} ({}) to {}; its sessions now count\n",
        source.display_title(),
        source.reader.label(),
        book.title
    ))
}

pub fn inbox_ignore(app: &App, source_query: &str) -> Result<String> {
    let sources = app.db.inbox_sources()?;
    let found = find_sources(&sources, source_query);
    let [source] = found.as_slice() else {
        bail!(
            "{} inbox items match {source_query:?}; be more specific",
            found.len()
        );
    };
    app.db.ignore_source(source.id)?;
    Ok(format!("ignoring {}\n", source.display_title()))
}

pub fn rename_goal(app: &App, old: &str, new: &str) -> Result<String> {
    let books = app.db.rename_goal(old, new)?;
    let mut out = format!("renamed goal {old} → {new} on {books} books and their sessions\n");
    if app.config.goal(new).is_none() {
        let _ = writeln!(
            out,
            "now change key = {old:?} to key = {new:?} in {}",
            app.paths.config_file.display()
        );
    }
    Ok(out)
}

pub fn stats(app: &App) -> Result<String> {
    let today = app.today();
    let goal = app.config.daily_goal();
    let year_ago = today.saturating_sub(Span::new().days(366));
    let totals = app
        .db
        .daily_totals(year_ago, today, app.config.count_off_shelf)?;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "streak {} · best {} · goal {} min/day\n",
        streak(&totals, today, goal),
        best_streak(&totals, goal),
        minutes(goal)
    );
    for offset in (0..7).rev() {
        let day = today.saturating_sub(Span::new().days(offset));
        let total = totals.get(&day).copied().unwrap_or_default();
        let filled = (minutes(total) as usize).min(40);
        let mark = if total >= goal { "✓" } else { " " };
        let _ = writeln!(
            out,
            "{} {mark} {:<40} {}",
            day.strftime("%a %d"),
            "█".repeat(filled),
            format_minutes(total)
        );
    }
    let monday =
        today.saturating_sub(Span::new().days(i64::from(today.weekday().to_monday_zero_offset())));
    let week = app.db.goal_totals(monday, today)?;
    let _ = writeln!(out, "\nthis week by goal");
    for goal in &app.config.goals {
        let read = week.get(&goal.key).copied().unwrap_or_default();
        let target = goal
            .weekly_minutes
            .map(|m| format!(" / {m}m"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {:<22} {}{target}",
            truncate(&goal.label, 22),
            format_minutes(read)
        );
    }
    Ok(out)
}
