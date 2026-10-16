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
}

pub struct PollContext<'a> {
    pub now: Timestamp,
    pub desktop: &'a dyn Desktop,
    pub idle_limit: SignedDuration,
    pub library_dirs: &'a [std::path::PathBuf],
}

impl PollContext<'_> {
    pub fn user_is_active(&self) -> bool {
        self.desktop
            .idle()
            .is_some_and(|idle| idle < self.idle_limit)
    }

    pub fn in_library(&self, path: &Path) -> bool {
        self.library_dirs.iter().any(|dir| path.starts_with(dir))
    }
}

pub trait ReaderAdapter {
    fn reader(&self) -> Reader;
    fn poll(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>>;
}

fn file_stem(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
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
    }
}
