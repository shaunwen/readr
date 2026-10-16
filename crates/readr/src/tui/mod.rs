mod data;
mod model;
mod view;

use std::time::{Duration, Instant};

use anyhow::Result;
use jiff::SignedDuration;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use crate::context::App;
use crate::{commands, shelf};

use self::data::Data;
use self::model::{Effect, Model};

const REFRESH: Duration = Duration::from_secs(5);

pub fn run(app: &App) -> Result<()> {
    let mut model = Model::new(Data::load(app)?);
    let mut terminal = ratatui::init();
    let result = event_loop(app, &mut terminal, &mut model);
    ratatui::restore();
    result
}

fn event_loop(app: &App, terminal: &mut ratatui::DefaultTerminal, model: &mut Model) -> Result<()> {
    let mut loaded_at = Instant::now();
    while !model.quit {
        terminal.draw(|frame| view::draw(frame, model))?;
        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && let Some(effect) = model.update(key)
            {
                model.message = Some(execute(app, effect));
                model.replace_data(Data::load(app)?);
                loaded_at = Instant::now();
            }
        } else if loaded_at.elapsed() >= REFRESH {
            model.replace_data(Data::load(app)?);
            loaded_at = Instant::now();
        }
    }
    Ok(())
}

fn execute(app: &App, effect: Effect) -> String {
    let result = match effect {
        Effect::Resume(book_id) => {
            book(app, book_id).and_then(|book| commands::open_book(app, &book))
        }
        Effect::SetStatus(book_id, status) => {
            book(app, book_id).and_then(|book| shelf::apply_status(app, &book, status))
        }
        Effect::SetGoal {
            book_id,
            goal,
            move_history,
        } => book(app, book_id)
            .and_then(|book| shelf::apply_goal(app, &book, goal.as_deref(), move_history)),
        Effect::AddSource(source_id) => match app.db.source(source_id) {
            Ok(Some(source)) => shelf::add(
                app,
                shelf::AddOptions {
                    query: &source.locator,
                    goal: None,
                    queue: false,
                },
            ),
            Ok(None) => Ok("that source is gone".into()),
            Err(error) => Err(error),
        },
        Effect::Link { source_id, book_id } => app
            .db
            .link_source(source_id, Some(book_id))
            .map(|()| "linked; its sessions now count".to_string()),
        Effect::Ignore(source_id) => app
            .db
            .ignore_source(source_id)
            .map(|()| "ignored".to_string()),
        Effect::EditSession {
            session_id,
            minutes,
        } => app
            .db
            .sessions_between(app.today(), app.today())
            .and_then(|sessions| {
                let source_id = sessions
                    .iter()
                    .find(|s| s.id == session_id)
                    .map(|s| s.source_id)
                    .ok_or_else(|| anyhow::anyhow!("session not found"))?;
                app.db
                    .edit_session(session_id, SignedDuration::from_mins(minutes), source_id)?;
                Ok(format!("session set to {minutes} min"))
            }),
        Effect::DeleteSession(session_id) => app
            .db
            .delete_session(session_id)
            .map(|()| "session deleted".to_string()),
    };
    let text = match result {
        Ok(message) => message,
        Err(error) => format!("{error:#}"),
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn book(app: &App, book_id: i64) -> Result<readr_core::db::Book> {
    app.db
        .book(book_id)?
        .ok_or_else(|| anyhow::anyhow!("that book is gone"))
}

#[cfg(test)]
mod tests;
