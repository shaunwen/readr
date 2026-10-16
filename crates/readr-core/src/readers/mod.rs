pub mod apple_books;
mod ax;
pub mod bookokrat;
pub mod desktop;
pub mod preview;

use std::path::Path;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};

use crate::model::{Position, Reader};
use crate::session::ActivityKind;

pub use desktop::{Desktop, MacDesktop};

#[derive(Debug, Clone, PartialEq)]
pub struct BookRef {
    pub reader: Reader,
    pub locator: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub collections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReaderEvent {
    Activity {
        book: BookRef,
        at: Timestamp,
        kind: ActivityKind,
        position: Option<Position>,
    },
    Position {
        book: BookRef,
        at: Timestamp,
        position: Position,
    },
    Closed {
        reader: Reader,
        at: Timestamp,
    },
    Away {
        book: BookRef,
        last_input_at: Timestamp,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Active,
    Away { last_input_at: Timestamp },
    Unknown,
}

pub struct PollContext<'a> {
    pub now: Timestamp,
    pub desktop: &'a dyn Desktop,
    pub away_after: SignedDuration,
    pub window_focus: bool,
    pub library_dirs: &'a [std::path::PathBuf],
}

impl PollContext<'_> {
    pub fn presence(&self) -> Presence {
        match self.desktop.idle() {
            Some(idle) if idle < self.away_after => Presence::Active,
            Some(idle) => Presence::Away {
                last_input_at: self.now - idle,
            },
            None => Presence::Unknown,
        }
    }

    pub fn reading_event(&self, book: BookRef, position: Option<Position>) -> Option<ReaderEvent> {
        match self.presence() {
            Presence::Active => Some(ReaderEvent::Activity {
                book,
                at: self.now,
                kind: ActivityKind::Focused,
                position,
            }),
            Presence::Away { last_input_at } => Some(ReaderEvent::Away {
                book,
                last_input_at,
            }),
            Presence::Unknown => None,
        }
    }

    pub fn in_library(&self, path: &Path) -> bool {
        self.library_dirs.iter().any(|dir| path.starts_with(dir))
    }
}

pub trait ReaderAdapter {
    fn reader(&self) -> Reader;
    fn poll(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>>;
}

const MINOR_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "by", "for", "in", "of", "on", "or", "the", "to", "vs", "with",
];

pub fn file_title(path: &str) -> Option<String> {
    let stem = Path::new(path).file_stem()?.to_string_lossy().into_owned();
    if stem.contains(' ') {
        return Some(stem);
    }
    let words: Vec<String> = stem
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .enumerate()
        .map(|(index, word)| {
            if index > 0 && MINOR_WORDS.contains(&word) {
                return word.to_string();
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use jiff::SignedDuration;

    use super::Desktop;

    #[derive(Default)]
    pub struct FakeDesktop {
        pub front: RefCell<Option<String>>,
        pub idle: RefCell<Option<SignedDuration>>,
        pub running: RefCell<Vec<String>>,
        pub preview_document: RefCell<Option<PathBuf>>,
        pub pages: RefCell<HashMap<PathBuf, u32>>,
        pub titles: RefCell<HashMap<PathBuf, String>>,
        pub window_title: RefCell<Option<String>>,
        pub trusted: RefCell<bool>,
    }

    impl FakeDesktop {
        pub fn active_in(app: &str) -> Self {
            let desktop = Self::default();
            desktop.front.replace(Some(app.into()));
            desktop.idle.replace(Some(SignedDuration::from_secs(5)));
            desktop
        }
    }

    impl Desktop for FakeDesktop {
        fn frontmost_bundle_id(&self) -> Option<String> {
            self.front.borrow().clone()
        }

        fn idle(&self) -> Option<SignedDuration> {
            *self.idle.borrow()
        }

        fn process_running(&self, name: &str) -> bool {
            self.running.borrow().iter().any(|p| p == name)
        }

        fn preview_front_document(&self) -> Option<PathBuf> {
            self.preview_document.borrow().clone()
        }

        fn page_count(&self, path: &Path) -> Option<u32> {
            self.pages.borrow().get(path).copied()
        }

        fn document_title(&self, path: &Path) -> Option<String> {
            self.titles.borrow().get(path).cloned()
        }

        fn accessibility_trusted(&self) -> bool {
            *self.trusted.borrow()
        }

        fn focused_window_title(&self) -> Option<String> {
            if !*self.trusted.borrow() {
                return None;
            }
            self.window_title.borrow().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphenated_file_names_become_titles() {
        // given
        let path = "/Books/classics/war-and-peace.pdf";

        // when
        let title = file_title(path);

        // then
        assert_eq!(title.as_deref(), Some("War and Peace"));
    }

    #[test]
    fn minor_words_stay_lowercase_except_first() {
        // given
        let path = "/Books/a-philosophy-of-software-design.pdf";

        // when
        let title = file_title(path);

        // then
        assert_eq!(title.as_deref(), Some("A Philosophy of Software Design"));
    }

    #[test]
    fn file_names_with_spaces_are_kept() {
        // given
        let path = "/Books/Pride and Prejudice, Annotated.epub";

        // when
        let title = file_title(path);

        // then
        assert_eq!(title.as_deref(), Some("Pride and Prejudice, Annotated"));
    }
}
