use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Result, bail};
use jiff::Timestamp;
use readr_core::db::{Book, Source};
use readr_core::find::{find_books, find_sources, main_title, matches, same_title};

use crate::context::App;
use crate::shelf::relative_day;

pub struct ForgetOptions<'a> {
    pub query: &'a str,
    pub keep_history: bool,
    pub yes: bool,
}

struct Target {
    title: String,
    book: Option<Book>,
    sources: Vec<Source>,
}

pub fn forget(app: &App, options: ForgetOptions<'_>) -> Result<String> {
    let target = resolve(app, options.query)?;
    if let Some(book) = &target.book
        && !options.yes
    {
        confirm(&format!(
            "{} is on your shelf ({}). Take it off and forget it?",
            book.title, book.status
        ))?;
    }
    let ids: Vec<i64> = target.sources.iter().map(|s| s.id).collect();
    let deleted = app
        .db
        .forget(&ids, &target.title, options.keep_history, Timestamp::now())?;
    if let Some(book) = &target.book {
        app.db.delete_book(book.id)?;
    }

    let mut readers: Vec<&str> = target.sources.iter().map(|s| s.reader.label()).collect();
    readers.sort_unstable();
    readers.dedup();
    let mut out = format!("forgot {}", target.title);
    if !readers.is_empty() {
        let _ = write!(out, " ({})", readers.join(", "));
    }
    if options.keep_history {
        out.push_str(": past sessions kept");
    } else {
        let _ = write!(
            out,
            ": {deleted} session{} deleted",
            if deleted == 1 { "" } else { "s" }
        );
    }
    if target.book.is_some() {
        out.push_str(", removed from the shelf");
    }
    let _ = writeln!(
        out,
        "\nreadr won't track it again until `readr track \"{}\"`",
        target.title
    );
    Ok(out)
}

pub fn track(app: &App, query: &str) -> Result<String> {
    let titles = app.db.untracked_titles()?;
    let found: Vec<&String> = titles
        .iter()
        .map(|(title, _)| title)
        .filter(|title| title.eq_ignore_ascii_case(query.trim()) || matches(query, title))
        .collect();
    let title = match found.as_slice() {
        [title] => (*title).clone(),
        [] => bail!("no forgotten book matches {query:?}; see `readr forget --list`"),
        many => bail!(
            "{query:?} matches several forgotten books: {}",
            many.iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let copies = app.db.track_again(&title)?;
    Ok(format!(
        "tracking {title} again ({copies} cop{} known); past sessions stay deleted\nput it back on the shelf with `readr shelf add \"{title}\"`\n",
        if copies == 1 { "y" } else { "ies" }
    ))
}

pub fn list(app: &App) -> Result<String> {
    let titles = app.db.untracked_titles()?;
    if titles.is_empty() {
        return Ok("no forgotten books\n".into());
    }
    let mut out = String::new();
    for (title, at) in titles {
        let _ = writeln!(out, "{title:<40} forgotten {}", relative_day(app, Some(at)));
    }
    Ok(out)
}

fn resolve(app: &App, query: &str) -> Result<Target> {
    let books = app.db.books()?;
    let all = app.db.all_sources()?;
    let found_books = find_books(&books, query);
    let (title, book) = match found_books.as_slice() {
        [book] => (book.title.clone(), Some((*book).clone())),
        [] => {
            let tracked: Vec<Source> = all.iter().filter(|s| !s.untracked).cloned().collect();
            let found = find_sources(&tracked, query);
            let Some(first) = found.first() else {
                bail!("no book matches {query:?}");
            };
            let first_title = first.display_title();
            if !found
                .iter()
                .all(|s| same_title(&s.display_title(), &first_title))
            {
                let titles: Vec<String> =
                    found.iter().take(10).map(|s| s.display_title()).collect();
                bail!(
                    "{query:?} matches several books, be more specific: {}",
                    titles.join(", ")
                );
            }
            let book = match found.iter().find_map(|s| s.book_id) {
                Some(id) => app.db.book(id)?,
                None => None,
            };
            let title = book
                .as_ref()
                .map(|b| b.title.clone())
                .unwrap_or_else(|| main_title(&first_title));
            (title, book)
        }
        many => bail!(
            "{query:?} matches several books on the shelf: {}",
            many.iter()
                .map(|b| b.title.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let sources = all
        .into_iter()
        .filter(|s| {
            book.as_ref().is_some_and(|b| s.book_id == Some(b.id))
                || same_title(&s.display_title(), &title)
        })
        .collect();
    Ok(Target {
        title,
        book,
        sources,
    })
}

fn confirm(question: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("{question} Run again with --yes to confirm.");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("nothing changed")
    }
}
