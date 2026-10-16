use anyhow::Result;
use jiff::Span;
use readr_core::db::{Book, Source};
use readr_core::suggest::suggest;

use crate::context::App;

pub struct NextUp {
    pub book: Book,
    pub source: Option<Source>,
    pub reason: String,
}

impl NextUp {
    pub fn describe(&self) -> String {
        let position = self
            .source
            .as_ref()
            .and_then(|s| s.last_position.as_ref())
            .map(|p| format!(" {}", p.display()))
            .unwrap_or_default();
        let reader = self
            .source
            .as_ref()
            .map(|s| format!(" ({})", s.reader.label()))
            .unwrap_or_default();
        format!("{}{position}{reader}", self.book.title)
    }
}

pub fn next_up(app: &App) -> Result<Option<NextUp>> {
    let books = app.db.books()?;
    let today = app.today();
    let monday =
        today.saturating_sub(Span::new().days(i64::from(today.weekday().to_monday_zero_offset())));
    let week = app.db.goal_totals(monday, today)?;
    let last_read = app.db.last_read_by_book()?;
    let Some(suggestion) = suggest(&app.config, &books, &last_read, &week) else {
        return Ok(None);
    };
    let source = app
        .db
        .sources_for_book(suggestion.book.id)?
        .into_iter()
        .find(|source| source.reader != readr_core::model::Reader::Manual);
    Ok(Some(NextUp {
        book: suggestion.book.clone(),
        source,
        reason: suggestion.reason,
    }))
}
