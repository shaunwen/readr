use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

use super::{BookRef, PollContext, ReaderAdapter, ReaderEvent, file_title};
use crate::model::{Position, Reader};
use crate::session::ActivityKind;

const PROCESS: &str = "bookokrat";
const TITLE_PREFIX: &str = "bookokrat \u{2014} ";

pub fn default_root(home: &Path) -> PathBuf {
    home.join("Library/Application Support/bookokrat/libraries")
}

#[derive(Debug, Deserialize)]
struct BookmarksFile {
    #[serde(default)]
    books: HashMap<String, Bookmark>,
}

#[derive(Debug, Deserialize)]
struct Bookmark {
    last_read: Timestamp,
    chapter_index: Option<u32>,
    total_chapters: Option<u32>,
    pdf_page: Option<u32>,
    book_progress: Option<f64>,
    book_title: Option<String>,
    book_author: Option<String>,
    absolute_path: Option<String>,
}

impl Bookmark {
    fn position(&self) -> Position {
        let label = match (self.pdf_page, self.chapter_index, self.total_chapters) {
            (Some(page), _, Some(total)) => Some(format!("p.{}/{total}", page + 1)),
            (Some(page), _, None) => Some(format!("p.{}", page + 1)),
            (None, Some(chapter), Some(total)) => Some(format!("ch.{}/{total}", chapter + 1)),
            (None, Some(chapter), None) => Some(format!("ch.{}", chapter + 1)),
            (None, None, _) => None,
        };
        Position::new(self.book_progress, label)
    }
}

struct Known {
    book: BookRef,
    position: Position,
    last_read: Timestamp,
}

pub struct Bookokrat {
    root: PathBuf,
    stale_after: SignedDuration,
    file_mtimes: HashMap<PathBuf, SystemTime>,
    known: HashMap<String, Known>,
    primed: bool,
    was_running: bool,
}

impl Bookokrat {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            stale_after: SignedDuration::from_mins(10),
            file_mtimes: HashMap::new(),
            known: HashMap::new(),
            primed: false,
            was_running: false,
        }
    }

    fn bookmark_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("bookmarks.json"))
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        files
    }

    fn read_changed(&mut self, path: &Path, now: Timestamp) -> Result<Vec<ReaderEvent>> {
        let mtime = std::fs::metadata(path)?.modified()?;
        if self.file_mtimes.get(path) == Some(&mtime) {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(path)?;
        let file: BookmarksFile =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        self.file_mtimes.insert(path.to_path_buf(), mtime);

        let library = path.parent().unwrap_or(path);
        let mut events = Vec::new();
        for (name, bookmark) in file.books {
            let locator = bookmark
                .absolute_path
                .clone()
                .unwrap_or_else(|| library.join(&name).to_string_lossy().into_owned());
            if self
                .known
                .get(&locator)
                .is_some_and(|known| known.last_read == bookmark.last_read)
            {
                continue;
            }
            let book = BookRef {
                reader: Reader::Bookokrat,
                title: bookmark.book_title.clone().or_else(|| file_title(&name)),
                author: bookmark.book_author.clone(),
                collections: Vec::new(),
                locator: locator.clone(),
            };
            let position = bookmark.position();
            self.known.insert(
                locator,
                Known {
                    book: book.clone(),
                    position: position.clone(),
                    last_read: bookmark.last_read,
                },
            );
            let age = now.duration_since(bookmark.last_read);
            let fresh = age <= self.stale_after && age >= -SignedDuration::from_mins(1);
            if self.primed && fresh {
                events.push(ReaderEvent::Activity {
                    book,
                    at: bookmark.last_read.min(now),
                    kind: ActivityKind::PageTurn,
                    position: Some(position),
                });
            } else {
                events.push(ReaderEvent::Position {
                    book,
                    at: bookmark.last_read,
                    position,
                });
            }
        }
        Ok(events)
    }

    fn book_for_title(&self, title: &str) -> Option<&Known> {
        let stem = focused_book_stem(title)?;
        self.known
            .values()
            .filter(|known| {
                Path::new(&known.book.locator)
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy() == stem)
            })
            .max_by_key(|known| known.last_read)
    }
}

fn focused_book_stem(title: &str) -> Option<&str> {
    let start = title.find(TITLE_PREFIX)? + TITLE_PREFIX.len();
    let stem = title[start..].trim();
    (!stem.is_empty()).then_some(stem)
}

pub fn check(root: &Path) -> Result<usize> {
    let adapter = Bookokrat::new(root.to_path_buf());
    let files = adapter.bookmark_files();
    if files.is_empty() {
        anyhow::bail!("no bookmarks.json under {}", root.display());
    }
    let mut books = 0;
    for path in files {
        let text = std::fs::read_to_string(&path)?;
        let file: BookmarksFile =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        books += file.books.len();
    }
    Ok(books)
}

impl ReaderAdapter for Bookokrat {
    fn reader(&self) -> Reader {
        Reader::Bookokrat
    }

    fn poll(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
        let running = ctx.desktop.process_running(PROCESS);
        let mut events = Vec::new();
        for path in self.bookmark_files() {
            events.extend(self.read_changed(&path, ctx.now)?);
        }
        events.sort_by_key(|event| match event {
            ReaderEvent::Activity { at, .. }
            | ReaderEvent::Position { at, .. }
            | ReaderEvent::Closed { at, .. } => *at,
            ReaderEvent::Away { last_input_at, .. } => *last_input_at,
        });
        if running
            && ctx.window_focus
            && let Some(title) = ctx.desktop.focused_window_title()
            && let Some(known) = self.book_for_title(&title)
        {
            events.extend(ctx.reading_event(known.book.clone(), Some(known.position.clone())));
        }
        if self.was_running && !running {
            events.push(ReaderEvent::Closed {
                reader: Reader::Bookokrat,
                at: ctx.now,
            });
        }
        self.was_running = running;
        self.primed = true;
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::readers::fake::FakeDesktop;

    struct Fixture {
        dir: tempfile::TempDir,
        desktop: FakeDesktop,
    }

    impl Fixture {
        fn new() -> Self {
            let fixture = Self {
                dir: tempfile::tempdir().unwrap(),
                desktop: FakeDesktop::default(),
            };
            fixture.desktop.running.replace(vec![PROCESS.into()]);
            fixture
        }

        fn write(&self, library: &str, last_read: &str, page: u32) {
            let dir = self.dir.path().join(library);
            fs::create_dir_all(&dir).unwrap();
            let json = format!(
                r#"{{"books": {{"walden.pdf": {{
                    "chapter_href": "{page}", "node_index": 0, "last_read": "{last_read}",
                    "total_chapters": 300, "pdf_page": {page}, "book_progress": 0.05,
                    "book_title": "Walden", "book_author": "Henry David Thoreau",
                    "absolute_path": "/Books/communication/walden.pdf"
                }}}}}}"#
            );
            let path = dir.join("bookmarks.json");
            fs::write(&path, json).unwrap();
            let bump = SystemTime::now() + std::time::Duration::from_secs(u64::from(page));
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(bump)
                .unwrap();
        }

        fn poll(&self, adapter: &mut Bookokrat, now: &str) -> Vec<ReaderEvent> {
            let ctx = PollContext {
                now: now.parse().unwrap(),
                desktop: &self.desktop,
                away_after: SignedDuration::from_mins(15),
                window_focus: true,
                library_dirs: &[],
            };
            adapter.poll(&ctx).unwrap()
        }

        fn adapter(&self) -> Bookokrat {
            Bookokrat::new(self.dir.path().to_path_buf())
        }
    }

    #[test]
    fn first_poll_reports_positions_without_activity() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:01:00Z");

        // then
        let [ReaderEvent::Position { book, position, .. }] = events.as_slice() else {
            panic!("expected one position event, got {events:?}");
        };
        assert_eq!(book.locator, "/Books/communication/walden.pdf");
        assert_eq!(book.title.as_deref(), Some("Walden"));
        assert_eq!(position.label.as_deref(), Some("p.4/300"));
    }

    #[test]
    fn changed_last_read_is_a_page_turn() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:01:00Z");
        fixture.write("lib", "2026-10-16T09:02:10Z", 4);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:02:15Z");

        // then
        let [
            ReaderEvent::Activity {
                at, kind, position, ..
            },
        ] = events.as_slice()
        else {
            panic!("expected one activity event, got {events:?}");
        };
        assert_eq!(*at, "2026-10-16T09:02:10Z".parse::<Timestamp>().unwrap());
        assert_eq!(*kind, ActivityKind::PageTurn);
        assert_eq!(position.as_ref().unwrap().label.as_deref(), Some("p.5/300"));
    }

    #[test]
    fn unchanged_file_produces_no_events() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:01:00Z");

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:01:15Z");

        // then
        assert!(events.is_empty());
    }

    #[test]
    fn old_last_read_is_only_a_position_update() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:01:00Z");
        fixture.write("lib", "2026-10-16T08:00:00Z", 9);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:30:00Z");

        // then
        assert!(matches!(events.as_slice(), [ReaderEvent::Position { .. }]));
    }

    #[test]
    fn reader_exit_is_reported_once() {
        // given
        let fixture = Fixture::new();
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");
        fixture.desktop.running.replace(Vec::new());

        // when
        let first = fixture.poll(&mut adapter, "2026-10-16T09:00:15Z");
        let second = fixture.poll(&mut adapter, "2026-10-16T09:00:30Z");

        // then
        assert!(matches!(
            first.as_slice(),
            [ReaderEvent::Closed {
                reader: Reader::Bookokrat,
                ..
            }]
        ));
        assert!(second.is_empty());
    }

    fn focus(fixture: &Fixture, title: &str, idle_mins: i64) {
        fixture.desktop.trusted.replace(true);
        fixture.desktop.window_title.replace(Some(title.into()));
        fixture
            .desktop
            .idle
            .replace(Some(SignedDuration::from_mins(idle_mins)));
    }

    #[test]
    fn focused_bookokrat_window_counts_while_reading_a_long_page() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:05Z");
        focus(&fixture, "bookokrat \u{2014} walden", 8);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:08:00Z");

        // then
        let [
            ReaderEvent::Activity {
                book,
                kind,
                position,
                ..
            },
        ] = events.as_slice()
        else {
            panic!("expected one focused tick, got {events:?}");
        };
        assert_eq!(*kind, ActivityKind::Focused);
        assert_eq!(book.locator, "/Books/communication/walden.pdf");
        assert_eq!(position.as_ref().unwrap().label.as_deref(), Some("p.4/300"));
    }

    #[test]
    fn no_input_past_the_away_limit_reports_away() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:05Z");
        focus(&fixture, "bookokrat \u{2014} walden", 20);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:20:00Z");

        // then
        let [ReaderEvent::Away { last_input_at, .. }] = events.as_slice() else {
            panic!("expected away, got {events:?}");
        };
        assert_eq!(
            *last_input_at,
            "2026-10-16T09:00:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn other_focused_windows_are_not_reading() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:05Z");
        focus(&fixture, "nvim ~/notes", 1);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:03:00Z");

        // then
        assert!(events.is_empty());
    }

    #[test]
    fn window_focus_needs_accessibility_and_the_setting() {
        // given
        let fixture = Fixture::new();
        fixture.write("lib", "2026-10-16T09:00:00Z", 3);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:05Z");
        focus(&fixture, "bookokrat \u{2014} walden", 1);
        fixture.desktop.trusted.replace(false);

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:03:00Z");

        // then
        assert!(events.is_empty());
    }

    #[test]
    fn same_stem_in_two_formats_picks_the_most_recently_read() {
        // given
        let mut adapter = Bookokrat::new(PathBuf::from("/none"));
        for (locator, last_read) in [
            ("/Books/ulysses.pdf", "2026-10-10T09:00:00Z"),
            ("/Books/ulysses.epub", "2026-10-15T09:00:00Z"),
        ] {
            adapter.known.insert(
                locator.into(),
                Known {
                    book: BookRef {
                        reader: Reader::Bookokrat,
                        locator: locator.into(),
                        title: None,
                        author: None,
                        collections: Vec::new(),
                    },
                    position: Position::default(),
                    last_read: last_read.parse().unwrap(),
                },
            );
        }

        // when
        let known = adapter.book_for_title("bookokrat \u{2014} ulysses");

        // then
        assert_eq!(known.unwrap().book.locator, "/Books/ulysses.epub");
    }

    #[test]
    fn library_view_title_names_no_book() {
        // given
        let title = "bookokrat";

        // when
        let stem = focused_book_stem(title);

        // then
        assert_eq!(stem, None);
    }

    #[test]
    fn epub_position_uses_one_based_chapter() {
        // given
        let bookmark: Bookmark = serde_json::from_str(
            r#"{"last_read": "2026-10-14T20:00:00.123456Z", "chapter_index": 6,
                "total_chapters": 40, "book_progress": 0.1}"#,
        )
        .unwrap();

        // when
        let position = bookmark.position();

        // then
        assert_eq!(position.label.as_deref(), Some("ch.7/40"));
    }

    #[test]
    fn missing_root_is_not_an_error() {
        // given
        let fixture = Fixture::new();
        let mut adapter = Bookokrat::new(fixture.dir.path().join("missing"));

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert!(events.is_empty());
    }
}
