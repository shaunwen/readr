use std::fmt::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use jiff::{SignedDuration, Timestamp};
use readr_core::db::{Book, NewBook, Source};
use readr_core::find::{find_books, find_sources, main_title, same_title};
use readr_core::model::{BookStatus, Position, Reader};
use readr_core::readers::{Desktop, MacDesktop, file_title};
use readr_core::stats::format_minutes;

use crate::context::App;
use crate::status::truncate;

pub const ACTIVE_LIMIT: usize = 4;

pub struct ShelfRow {
    pub book: Book,
    pub goal_label: String,
    pub source: Option<Source>,
    pub position: Option<Position>,
    pub read: f64,
    pub time: SignedDuration,
    pub last_read: Option<Timestamp>,
}

impl ShelfRow {
    pub fn position_label(&self) -> String {
        self.position
            .as_ref()
            .map(Position::display)
            .unwrap_or_else(|| "-".into())
    }

    pub fn time_left(&self) -> Option<SignedDuration> {
        let at = self.position.as_ref()?.progress?;
        if self.read < 0.01 || self.time < SignedDuration::from_mins(10) {
            return None;
        }
        let seconds_per_book = self.time.as_secs_f64() / self.read;
        SignedDuration::try_from_secs_f64(seconds_per_book * (1.0 - at).max(0.0)).ok()
    }
}

pub fn rows(app: &App) -> Result<Vec<ShelfRow>> {
    let totals = app.db.book_totals()?;
    let last_read = app.db.last_read_by_book()?;
    let read = app.db.progress_read_by_book()?;
    let mut rows = Vec::new();
    for book in app.db.books()? {
        let sources = app.db.sources_for_book(book.id)?;
        let source = sources.iter().find(|s| s.reader != Reader::Manual).cloned();
        let position = source.as_ref().and_then(|s| s.last_position.clone());
        let goal_label = book
            .goal
            .as_deref()
            .map(|key| {
                app.config
                    .goal(key)
                    .map(|g| g.label.clone())
                    .unwrap_or_else(|| key.to_string())
            })
            .unwrap_or_else(|| "no goal".into());
        rows.push(ShelfRow {
            time: totals.get(&book.id).copied().unwrap_or_default(),
            last_read: last_read.get(&book.id).copied(),
            read: read.get(&book.id).copied().unwrap_or_default(),
            book,
            goal_label,
            source,
            position,
        });
    }
    let order = |row: &ShelfRow| {
        row.book
            .goal
            .as_deref()
            .and_then(|key| app.config.goals.iter().position(|g| g.key == key))
            .unwrap_or(usize::MAX)
    };
    rows.sort_by_key(|row| (order(row), row.book.priority, row.book.id));
    Ok(rows)
}

pub fn list(app: &App) -> Result<String> {
    let rows = rows(app)?;
    let mut out = String::new();
    let active: Vec<&ShelfRow> = rows
        .iter()
        .filter(|r| r.book.status == BookStatus::Active)
        .collect();
    if active.is_empty() {
        let _ = writeln!(
            out,
            "No active books. Add one with `readr shelf add <title>`."
        );
    }
    if !active.is_empty() {
        let _ = writeln!(
            out,
            "{:<22} {:<34} {:>10}  {:<9} {:<10} {:>6}",
            "goal", "book", "at", "reader", "last read", "read"
        );
    }
    for row in &active {
        let _ = writeln!(
            out,
            "{:<22} {:<34} {:>10}  {:<9} {:<10} {:>6}{}",
            truncate(&row.goal_label, 22),
            truncate(&row.book.title, 34),
            row.position_label(),
            row.source.as_ref().map(|s| s.reader.label()).unwrap_or("-"),
            relative_day(app, row.last_read),
            format_minutes(row.time),
            row.time_left()
                .map(|left| format!("  ~{} left", format_minutes(left)))
                .unwrap_or_default(),
        );
    }
    let queued: Vec<&str> = rows
        .iter()
        .filter(|r| r.book.status == BookStatus::Queued)
        .map(|r| r.book.title.as_str())
        .collect();
    if !queued.is_empty() {
        let _ = writeln!(out, "queue: {}", queued.join(", "));
    }
    let done = rows
        .iter()
        .filter(|r| r.book.status == BookStatus::Done)
        .count();
    if done > 0 {
        let _ = writeln!(out, "finished: {done}");
    }
    Ok(out)
}

pub fn relative_day(app: &App, at: Option<Timestamp>) -> String {
    let Some(at) = at else {
        return "never".into();
    };
    let day = app.local(at).date();
    let today = app.today();
    let days = (today - day).get_days();
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=13 => format!("{days}d ago"),
        _ => day.strftime("%d %b").to_string(),
    }
}

pub struct AddOptions<'a> {
    pub query: &'a str,
    pub goal: Option<&'a str>,
    pub queue: bool,
}

pub fn add(app: &App, options: AddOptions<'_>) -> Result<String> {
    let all = app.db.all_sources()?;
    let mut matches: Vec<Source> = find_sources(&all, options.query)
        .into_iter()
        .cloned()
        .collect();
    if matches.is_empty() {
        let path = Path::new(options.query);
        if !path.is_file() {
            bail!(
                "no book matches {:?}; open it once in bookokrat, Preview or Books, or pass a file path",
                options.query
            );
        }
        let canonical = path.canonicalize()?;
        let locator = canonical.to_string_lossy();
        let title = MacDesktop
            .document_title(&canonical)
            .or_else(|| file_title(&locator));
        matches.push(
            app.db
                .upsert_source(Reader::Bookokrat, &locator, title.as_deref(), None)?,
        );
    }
    let first_title = matches[0].display_title();
    if !matches
        .iter()
        .all(|s| same_title(&s.display_title(), &first_title))
    {
        let mut message = format!(
            "{:?} matches several books, be more specific:\n",
            options.query
        );
        for source in matches.iter().take(10) {
            let _ = writeln!(
                message,
                "  {}  ({}, {})",
                source.display_title(),
                source.reader.label(),
                source.locator
            );
        }
        bail!(message.trim_end().to_string());
    }
    let unlinked: Vec<&Source> = matches.iter().filter(|s| s.book_id.is_none()).collect();
    if unlinked.is_empty() {
        let book_id = matches[0].book_id.expect("linked source");
        let book = app.db.book(book_id)?.context("book of linked source")?;
        bail!("{} is already on the shelf ({})", book.title, book.status);
    }

    let title = matches
        .iter()
        .filter_map(|s| s.title.as_deref())
        .map(main_title)
        .max_by_key(|t| t.len().min(60))
        .unwrap_or_else(|| main_title(&first_title));
    let author = matches.iter().find_map(|s| s.author.clone());
    let (goal, goal_note) = match options.goal {
        Some("none") => (None, String::new()),
        Some(key) => {
            if app.config.goal(key).is_none() {
                let known: Vec<&str> = app.config.goals.iter().map(|g| g.key.as_str()).collect();
                bail!("unknown goal {key:?}; goals are: {}", known.join(", "));
            }
            (Some(key.to_string()), String::new())
        }
        None => match matches
            .iter()
            .find_map(|s| app.config.suggest_goal(&s.locator, &s.collections))
        {
            Some(goal) => (
                Some(goal.key.clone()),
                " (from its folder or collection)".to_string(),
            ),
            None => (None, " (pass --goal to set one)".to_string()),
        },
    };
    let status = if options.queue {
        BookStatus::Queued
    } else {
        BookStatus::Active
    };
    let book_id = app.db.add_book(
        &NewBook {
            title: title.clone(),
            author,
            goal: goal.clone(),
            status,
        },
        Timestamp::now(),
    )?;
    for source in &unlinked {
        app.db.link_source(source.id, Some(book_id))?;
    }
    let readers: Vec<&str> = unlinked.iter().map(|s| s.reader.label()).collect();
    let mut out = format!(
        "added {title} · {} · goal {}{goal_note} · {}\n",
        status,
        goal.as_deref().unwrap_or("none"),
        readers.join(", ")
    );
    let active = app
        .db
        .books()?
        .iter()
        .filter(|b| b.status == BookStatus::Active)
        .count();
    if status == BookStatus::Active && active > ACTIVE_LIMIT {
        let _ = writeln!(
            out,
            "note: {active} active books; finishing or queueing some keeps the shelf focused"
        );
    }
    Ok(out)
}

pub fn pick_book(app: &App, query: &str) -> Result<Book> {
    let books = app.db.books()?;
    let found = find_books(&books, query);
    match found.as_slice() {
        [book] => Ok((*book).clone()),
        [] => bail!("no book on the shelf matches {query:?}"),
        many => {
            let titles: Vec<&str> = many.iter().map(|b| b.title.as_str()).collect();
            bail!("{query:?} matches several books: {}", titles.join(", "))
        }
    }
}

pub fn set_status(app: &App, query: &str, status: BookStatus) -> Result<String> {
    apply_status(app, &pick_book(app, query)?, status)
}

pub fn apply_status(app: &App, book: &Book, status: BookStatus) -> Result<String> {
    app.db.set_book_status(book.id, status, Timestamp::now())?;
    let mut out = format!("{} is now {status}\n", book.title);
    if matches!(status, BookStatus::Done | BookStatus::Dropped)
        && let Some(next) = promote_next_queued(app, book.goal.as_deref())?
    {
        let _ = writeln!(out, "{} moved from the queue to active", next.title);
    }
    Ok(out)
}

fn promote_next_queued(app: &App, goal: Option<&str>) -> Result<Option<Book>> {
    let books = app.db.books()?;
    let queued: Vec<&Book> = books
        .iter()
        .filter(|b| b.status == BookStatus::Queued)
        .collect();
    let next = queued
        .iter()
        .find(|b| b.goal.as_deref() == goal)
        .or_else(|| queued.first())
        .copied()
        .cloned();
    if let Some(book) = &next {
        app.db
            .set_book_status(book.id, BookStatus::Active, Timestamp::now())?;
    }
    Ok(next)
}

pub fn set_goal(app: &App, query: &str, goal: &str, move_history: bool) -> Result<String> {
    let goal = match goal {
        "none" => None,
        key if app.config.goal(key).is_some() => Some(key),
        key => bail!("unknown goal {key:?}"),
    };
    apply_goal(app, &pick_book(app, query)?, goal, move_history)
}

pub fn apply_goal(
    app: &App,
    book: &Book,
    goal: Option<&str>,
    move_history: bool,
) -> Result<String> {
    app.db.set_book_goal(book.id, goal, move_history)?;
    let history = if move_history {
        "past sessions moved too"
    } else {
        "past sessions keep their goal"
    };
    Ok(format!(
        "{} → {} ({history})\n",
        book.title,
        goal.unwrap_or("no goal")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: f64, read: f64, minutes: i64) -> ShelfRow {
        ShelfRow {
            book: Book {
                id: 1,
                title: "Walden".into(),
                author: None,
                goal: None,
                status: BookStatus::Active,
                priority: 0,
                added_at: Timestamp::UNIX_EPOCH,
                finished_at: None,
            },
            goal_label: "Learning".into(),
            source: None,
            position: Some(Position::new(Some(at), Some("p.150/300".into()))),
            read,
            time: SignedDuration::from_mins(minutes),
            last_read: None,
        }
    }

    #[test]
    fn time_left_uses_pages_actually_read_not_the_saved_place() {
        // given
        let shelf_row = row(0.5, 0.1, 60);

        // when
        let left = shelf_row.time_left();

        // then
        assert_eq!(left, Some(SignedDuration::from_mins(300)));
    }

    #[test]
    fn a_jump_without_tracked_reading_gives_no_estimate() {
        // given
        let shelf_row = row(0.63, 0.0, 30);

        // when
        let left = shelf_row.time_left();

        // then
        assert_eq!(left, None);
    }

    #[test]
    fn position_is_shown_as_a_place_not_a_percentage_read() {
        // given
        let shelf_row = row(0.5, 0.0, 0);

        // when
        let label = shelf_row.position_label();

        // then
        assert_eq!(label, "p.150/300");
    }
}
