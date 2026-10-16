use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use readr_core::db::Source;
use readr_core::find::matches;
use readr_core::model::BookStatus;

use super::data::Data;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Today,
    Shelf,
    Stats,
    Inbox,
}

impl Screen {
    pub const ALL: [Screen; 4] = [Self::Today, Self::Shelf, Self::Stats, Self::Inbox];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    Add {
        query: String,
        selected: usize,
    },
    Goal {
        book_id: i64,
        selected: usize,
        move_history: bool,
    },
    Link {
        source_id: i64,
        selected: usize,
    },
    EditMinutes {
        session_id: i64,
        input: String,
    },
    ConfirmDelete {
        session_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Resume(i64),
    SetStatus(i64, BookStatus),
    SetGoal {
        book_id: i64,
        goal: Option<String>,
        move_history: bool,
    },
    AddSource(i64),
    Link {
        source_id: i64,
        book_id: i64,
    },
    Ignore(i64),
    EditSession {
        session_id: i64,
        minutes: i64,
    },
    DeleteSession(i64),
}

pub struct Model {
    pub data: Data,
    pub screen: Screen,
    pub mode: Mode,
    pub today_selected: usize,
    pub shelf_selected: usize,
    pub inbox_selected: usize,
    pub message: Option<String>,
    pub quit: bool,
}

impl Model {
    pub fn new(data: Data) -> Self {
        Self {
            data,
            screen: Screen::Today,
            mode: Mode::Normal,
            today_selected: 0,
            shelf_selected: 0,
            inbox_selected: 0,
            message: None,
            quit: false,
        }
    }

    pub fn replace_data(&mut self, data: Data) {
        self.data = data;
        self.today_selected = clamp(self.today_selected, self.data.sessions.len());
        self.shelf_selected = clamp(self.shelf_selected, self.data.shelf.len());
        self.inbox_selected = clamp(self.inbox_selected, self.data.inbox.len());
    }

    pub fn add_matches(&self, query: &str) -> Vec<&Source> {
        if query.trim().is_empty() {
            return self.data.candidates.iter().take(50).collect();
        }
        self.data
            .candidates
            .iter()
            .filter(|s| {
                let text = format!(
                    "{} {} {}",
                    s.display_title(),
                    s.author.as_deref().unwrap_or(""),
                    s.locator
                );
                matches(query, &text)
            })
            .take(50)
            .collect()
    }

    pub fn goal_options(&self) -> Vec<(Option<String>, String)> {
        let mut options: Vec<(Option<String>, String)> = self
            .data
            .goals
            .iter()
            .map(|(key, label)| (Some(key.clone()), label.clone()))
            .collect();
        options.push((None, "no goal".into()));
        options
    }

    pub fn update(&mut self, key: KeyEvent) -> Option<Effect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return None;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match mode {
            Mode::Normal => self.normal(key),
            Mode::Help => None,
            Mode::Add { query, selected } => self.add(key, query, selected),
            Mode::Goal {
                book_id,
                selected,
                move_history,
            } => self.goal(key, book_id, selected, move_history),
            Mode::Link {
                source_id,
                selected,
            } => self.link(key, source_id, selected),
            Mode::EditMinutes { session_id, input } => self.edit(key, session_id, input),
            Mode::ConfirmDelete { session_id } => match key.code {
                KeyCode::Char('y') => Some(Effect::DeleteSession(session_id)),
                _ => None,
            },
        }
    }

    fn normal(&mut self, key: KeyEvent) -> Option<Effect> {
        self.message = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.screen = self.screen.next(),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.screen = self.screen.previous()
            }
            KeyCode::Char(c @ '1'..='4') => {
                self.screen = Screen::ALL[c as usize - '1' as usize];
            }
            KeyCode::Char('t') => self.screen = Screen::Today,
            KeyCode::Char('s') => self.screen = Screen::Shelf,
            KeyCode::Char('i') if self.screen != Screen::Inbox => self.screen = Screen::Inbox,
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('a') if self.screen != Screen::Inbox => {
                self.screen = Screen::Shelf;
                self.mode = Mode::Add {
                    query: String::new(),
                    selected: 0,
                };
            }
            _ => return self.screen_key(key),
        }
        None
    }

    fn screen_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match self.screen {
            Screen::Today => match key.code {
                KeyCode::Enter => self.data.next.as_ref().map(|n| Effect::Resume(n.book_id)),
                KeyCode::Char('e') => {
                    let session = self.selected_finished_session()?;
                    self.mode = Mode::EditMinutes {
                        session_id: session.0,
                        input: session.1.to_string(),
                    };
                    None
                }
                KeyCode::Char('x') => {
                    let (session_id, _) = self.selected_finished_session()?;
                    self.mode = Mode::ConfirmDelete { session_id };
                    None
                }
                _ => None,
            },
            Screen::Shelf => {
                let row = self.data.shelf.get(self.shelf_selected)?;
                let book_id = row.book.id;
                let status = row.book.status;
                match key.code {
                    KeyCode::Enter => Some(Effect::Resume(book_id)),
                    KeyCode::Char('f') => Some(Effect::SetStatus(book_id, BookStatus::Done)),
                    KeyCode::Char('d') => Some(Effect::SetStatus(book_id, BookStatus::Dropped)),
                    KeyCode::Char('u') => Some(Effect::SetStatus(
                        book_id,
                        if status == BookStatus::Active {
                            BookStatus::Queued
                        } else {
                            BookStatus::Active
                        },
                    )),
                    KeyCode::Char(c @ ('g' | 'G')) => {
                        let current = row.book.goal.clone();
                        let selected = self
                            .goal_options()
                            .iter()
                            .position(|(key, _)| *key == current)
                            .unwrap_or(0);
                        self.mode = Mode::Goal {
                            book_id,
                            selected,
                            move_history: c == 'G',
                        };
                        None
                    }
                    _ => None,
                }
            }
            Screen::Inbox => {
                let (source, _) = self.data.inbox.get(self.inbox_selected)?;
                let source_id = source.id;
                match key.code {
                    KeyCode::Char('a') | KeyCode::Enter => Some(Effect::AddSource(source_id)),
                    KeyCode::Char('i') => Some(Effect::Ignore(source_id)),
                    KeyCode::Char('L') | KeyCode::Char('n') => {
                        if self.data.shelf.is_empty() {
                            self.message = Some("the shelf is empty; add the book instead".into());
                        } else {
                            self.mode = Mode::Link {
                                source_id,
                                selected: 0,
                            };
                        }
                        None
                    }
                    _ => None,
                }
            }
            Screen::Stats => None,
        }
    }

    fn selected_finished_session(&mut self) -> Option<(i64, i64)> {
        let session = self.data.sessions.get(self.today_selected)?;
        if session.open {
            self.message = Some("this session is still running".into());
            return None;
        }
        Some((session.id, session.active.as_secs() / 60))
    }

    fn move_selection(&mut self, delta: isize) {
        let (selected, len) = match self.screen {
            Screen::Today => (&mut self.today_selected, self.data.sessions.len()),
            Screen::Shelf => (&mut self.shelf_selected, self.data.shelf.len()),
            Screen::Inbox => (&mut self.inbox_selected, self.data.inbox.len()),
            Screen::Stats => return,
        };
        *selected = step(*selected, delta, len);
    }

    fn add(&mut self, key: KeyEvent, mut query: String, selected: usize) -> Option<Effect> {
        let count = self.add_matches(&query).len();
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter => {
                return self
                    .add_matches(&query)
                    .get(selected)
                    .map(|source| Effect::AddSource(source.id));
            }
            KeyCode::Down => {
                self.mode = Mode::Add {
                    query,
                    selected: step(selected, 1, count),
                };
                return None;
            }
            KeyCode::Up => {
                self.mode = Mode::Add {
                    query,
                    selected: step(selected, -1, count),
                };
                return None;
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(c) => query.push(c),
            _ => {}
        }
        self.mode = Mode::Add { query, selected: 0 };
        None
    }

    fn goal(
        &mut self,
        key: KeyEvent,
        book_id: i64,
        selected: usize,
        move_history: bool,
    ) -> Option<Effect> {
        let options = self.goal_options();
        let selected = match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter => {
                let goal = options.get(selected).and_then(|(key, _)| key.clone());
                return Some(Effect::SetGoal {
                    book_id,
                    goal,
                    move_history,
                });
            }
            KeyCode::Char('j') | KeyCode::Down => step(selected, 1, options.len()),
            KeyCode::Char('k') | KeyCode::Up => step(selected, -1, options.len()),
            _ => selected,
        };
        self.mode = Mode::Goal {
            book_id,
            selected,
            move_history,
        };
        None
    }

    fn link(&mut self, key: KeyEvent, source_id: i64, selected: usize) -> Option<Effect> {
        let len = self.data.shelf.len();
        let selected = match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter => {
                return self.data.shelf.get(selected).map(|row| Effect::Link {
                    source_id,
                    book_id: row.book.id,
                });
            }
            KeyCode::Char('j') | KeyCode::Down => step(selected, 1, len),
            KeyCode::Char('k') | KeyCode::Up => step(selected, -1, len),
            _ => selected,
        };
        self.mode = Mode::Link {
            source_id,
            selected,
        };
        None
    }

    fn edit(&mut self, key: KeyEvent, session_id: i64, mut input: String) -> Option<Effect> {
        match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter => {
                return match input.trim().parse::<i64>() {
                    Ok(minutes) if (0..=720).contains(&minutes) => Some(Effect::EditSession {
                        session_id,
                        minutes,
                    }),
                    _ => {
                        self.message = Some("enter minutes between 0 and 720".into());
                        None
                    }
                };
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 3 => input.push(c),
            _ => {}
        }
        self.mode = Mode::EditMinutes { session_id, input };
        None
    }
}

fn step(selected: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (selected as isize + delta).clamp(0, len as isize - 1) as usize
}

fn clamp(selected: usize, len: usize) -> usize {
    selected.min(len.saturating_sub(1))
}
