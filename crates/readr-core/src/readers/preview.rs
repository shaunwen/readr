use std::collections::HashMap;
use std::io::Cursor;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use jiff::{SignedDuration, Timestamp};
use plist::Value;

use super::desktop::PREVIEW_BUNDLE;
use super::{BookRef, PollContext, ReaderAdapter, ReaderEvent, file_title};
use crate::model::{Position, Reader};

pub fn default_view_state(home: &Path) -> PathBuf {
    home.join(
        "Library/Containers/com.apple.Preview/Data/Library/Preferences/com.apple.Preview.ViewState.plist",
    )
}

#[derive(Debug, Clone, PartialEq)]
struct ViewStateEntry {
    inode: u64,
    stamp: Timestamp,
    page_index: Option<u32>,
}

pub struct Preview {
    view_state: PathBuf,
    view_state_mtime: Option<SystemTime>,
    stamps: HashMap<u64, Timestamp>,
    page_index: HashMap<PathBuf, u32>,
    pages: HashMap<PathBuf, Option<u32>>,
    inodes: HashMap<u64, PathBuf>,
    indexed_at: Option<Timestamp>,
    titles: HashMap<PathBuf, Option<String>>,
}

impl Preview {
    pub fn new(view_state: PathBuf) -> Self {
        Self {
            view_state,
            view_state_mtime: None,
            stamps: HashMap::new(),
            page_index: HashMap::new(),
            pages: HashMap::new(),
            inodes: HashMap::new(),
            indexed_at: None,
            titles: HashMap::new(),
        }
    }

    fn book(&mut self, path: &Path, ctx: &PollContext<'_>) -> BookRef {
        let locator = path.to_string_lossy().into_owned();
        let title = self
            .titles
            .entry(path.to_path_buf())
            .or_insert_with(|| ctx.desktop.document_title(path))
            .clone()
            .or_else(|| file_title(&locator));
        BookRef {
            reader: Reader::Preview,
            title,
            author: None,
            collections: Vec::new(),
            locator,
        }
    }

    fn position(&mut self, path: &Path, ctx: &PollContext<'_>) -> Option<Position> {
        let index = *self.page_index.get(path)?;
        let pages = *self
            .pages
            .entry(path.to_path_buf())
            .or_insert_with(|| ctx.desktop.page_count(path));
        let page = index + 1;
        Some(match pages {
            Some(pages) if pages > 0 => Position::new(
                Some(f64::from(page) / f64::from(pages)),
                Some(format!("p.{page}/{pages}")),
            ),
            _ => Position::new(None, Some(format!("p.{page}"))),
        })
    }

    fn path_for_inode(&mut self, inode: u64, ctx: &PollContext<'_>) -> Option<PathBuf> {
        if let Some(path) = self.inodes.get(&inode).filter(|path| path.exists()) {
            return Some(path.clone());
        }
        let recently = self
            .indexed_at
            .is_some_and(|at| ctx.now.duration_since(at) < SignedDuration::from_mins(5));
        if !recently {
            self.inodes = index_pdfs(ctx.library_dirs);
            self.indexed_at = Some(ctx.now);
        }
        self.inodes.get(&inode).cloned()
    }

    fn position_events(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
        let Ok(metadata) = std::fs::metadata(&self.view_state) else {
            return Ok(Vec::new());
        };
        let mtime = metadata.modified()?;
        if self.view_state_mtime == Some(mtime) {
            return Ok(Vec::new());
        }
        let entries = read_view_state(&self.view_state)?;
        self.view_state_mtime = Some(mtime);

        let mut events = Vec::new();
        for entry in entries {
            if self.stamps.get(&entry.inode) == Some(&entry.stamp) {
                continue;
            }
            self.stamps.insert(entry.inode, entry.stamp);
            let Some(page_index) = entry.page_index else {
                continue;
            };
            let Some(path) = self.path_for_inode(entry.inode, ctx) else {
                continue;
            };
            self.page_index.insert(path.clone(), page_index);
            if let Some(position) = self.position(&path, ctx) {
                events.push(ReaderEvent::Position {
                    book: self.book(&path, ctx),
                    at: entry.stamp,
                    position,
                });
            }
        }
        events.sort_by_key(|event| match event {
            ReaderEvent::Position { at, .. } => *at,
            _ => Timestamp::MIN,
        });
        Ok(events)
    }
}

pub fn check(view_state: &Path) -> Result<usize> {
    Ok(read_view_state(view_state)?.len())
}

impl ReaderAdapter for Preview {
    fn reader(&self) -> Reader {
        Reader::Preview
    }

    fn poll(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
        let mut events = self.position_events(ctx)?;
        let front = ctx.desktop.frontmost_bundle_id();
        if front.as_deref() == Some(PREVIEW_BUNDLE)
            && let Some(path) = ctx.desktop.preview_front_document()
            && ctx.in_library(&path)
        {
            let position = self.position(&path, ctx);
            let book = self.book(&path, ctx);
            events.extend(ctx.reading_event(book, position));
        }
        Ok(events)
    }
}

fn read_view_state(path: &Path) -> Result<Vec<ViewStateEntry>> {
    let value = Value::from_file(path).with_context(|| format!("reading {}", path.display()))?;
    let Some(root) = value.as_dictionary() else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (key, entry) in root {
        let Some(inode) = key.rsplit('.').next().and_then(|i| i.parse().ok()) else {
            continue;
        };
        let Some(entry) = entry.as_dictionary() else {
            continue;
        };
        let Some(stamp) = entry
            .get("Timestamp")
            .and_then(Value::as_date)
            .and_then(|date| Timestamp::try_from(SystemTime::from(date)).ok())
        else {
            continue;
        };
        let page_index = entry
            .get("Data")
            .and_then(Value::as_data)
            .and_then(|data| Value::from_reader(Cursor::new(data)).ok())
            .and_then(|inner| {
                inner
                    .as_dictionary()?
                    .get("UI_Additions_Windowed_Properties")?
                    .as_dictionary()?
                    .get("elementIndex")?
                    .as_unsigned_integer()
            })
            .and_then(|index| u32::try_from(index).ok());
        entries.push(ViewStateEntry {
            inode,
            stamp,
            page_index,
        });
    }
    Ok(entries)
}

fn index_pdfs(dirs: &[PathBuf]) -> HashMap<u64, PathBuf> {
    let mut index = HashMap::new();
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file()
                && is_pdf(&path)
                && let Ok(metadata) = entry.metadata()
            {
                index.insert(metadata.ino(), path);
            }
        }
    }
    index
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use plist::Dictionary;

    use super::*;
    use crate::readers::fake::FakeDesktop;
    use crate::session::ActivityKind;

    struct Fixture {
        dir: tempfile::TempDir,
        desktop: FakeDesktop,
        library: Vec<PathBuf>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let library = vec![dir.path().join("Books")];
            fs::create_dir_all(&library[0]).unwrap();
            Self {
                dir,
                desktop: FakeDesktop::active_in(PREVIEW_BUNDLE),
                library,
            }
        }

        fn book(&self, relative: &str, pages: u32) -> PathBuf {
            let path = self.library[0].join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"%PDF").unwrap();
            self.desktop.pages.borrow_mut().insert(path.clone(), pages);
            path
        }

        fn view_state_path(&self) -> PathBuf {
            self.dir.path().join("ViewState.plist")
        }

        fn write_view_state(&self, entries: &[(&Path, &str, u64)]) {
            let mut root = Dictionary::new();
            for (path, stamp, index) in entries {
                let mut windowed = Dictionary::new();
                windowed.insert("elementIndex".into(), Value::Integer((*index).into()));
                windowed.insert("pageViewMode".into(), Value::Integer(0.into()));
                let mut inner = Dictionary::new();
                inner.insert(
                    "UI_Additions_Windowed_Properties".into(),
                    Value::Dictionary(windowed),
                );
                let mut data = Vec::new();
                Value::Dictionary(inner)
                    .to_writer_binary(&mut data)
                    .unwrap();
                let stamp: Timestamp = stamp.parse().unwrap();
                let mut entry = Dictionary::new();
                entry.insert("Data".into(), Value::Data(data));
                entry.insert(
                    "Timestamp".into(),
                    Value::Date(SystemTime::from(stamp).into()),
                );
                let inode = fs::metadata(path).unwrap().ino();
                root.insert(
                    format!("8C1E2F3A-0000-4000-8000-000000000001.{inode}"),
                    Value::Dictionary(entry),
                );
            }
            Value::Dictionary(root)
                .to_file_binary(self.view_state_path())
                .unwrap();
        }

        fn poll(&self, adapter: &mut Preview, now: &str) -> Vec<ReaderEvent> {
            let ctx = PollContext {
                now: now.parse().unwrap(),
                desktop: &self.desktop,
                away_after: SignedDuration::from_mins(15),
                window_focus: true,
                library_dirs: &self.library,
            };
            adapter.poll(&ctx).unwrap()
        }

        fn adapter(&self) -> Preview {
            Preview::new(self.view_state_path())
        }
    }

    fn positions(events: &[ReaderEvent]) -> Vec<(String, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                ReaderEvent::Position { book, position, .. } => {
                    Some((book.title.clone().unwrap(), position.label.clone().unwrap()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn saved_view_state_gives_one_based_page_and_progress() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("classics/war-and-peace.pdf", 500);
        fixture.write_view_state(&[(&book, "2026-10-15T20:00:00Z", 41)]);
        fixture.desktop.front.replace(None);
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert_eq!(
            positions(&events),
            vec![("War And Peace".to_string(), "p.42/500".to_string())]
        );
        let ReaderEvent::Position { position, .. } = &events[0] else {
            unreachable!()
        };
        assert!((position.progress.unwrap() - 42.0 / 500.0).abs() < 1e-9);
    }

    #[test]
    fn only_changed_entries_are_reported_again() {
        // given
        let fixture = Fixture::new();
        let body = fixture.book("war-and-peace.pdf", 500);
        let explanation = fixture.book("the-origin-of-species.pdf", 320);
        fixture.write_view_state(&[
            (&body, "2026-10-15T20:00:00Z", 41),
            (&explanation, "2026-10-15T19:30:00Z", 9),
        ]);
        fixture.desktop.front.replace(None);
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");
        fixture.write_view_state(&[
            (&body, "2026-10-15T20:00:00Z", 41),
            (&explanation, "2026-10-16T09:20:00Z", 20),
        ]);
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(fixture.view_state_path())
            .unwrap()
            .set_modified(later)
            .unwrap();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:21:00Z");

        // then
        assert_eq!(
            positions(&events),
            vec![("The Origin Of Species".to_string(), "p.21/320".to_string())]
        );
    }

    #[test]
    fn focused_library_document_is_activity_with_last_known_page() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("war-and-peace.pdf", 500);
        fixture.write_view_state(&[(&book, "2026-10-15T20:00:00Z", 41)]);
        fixture.desktop.preview_document.replace(Some(book.clone()));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        let Some(ReaderEvent::Activity { kind, position, .. }) = events.last() else {
            panic!("expected activity, got {events:?}");
        };
        assert_eq!(*kind, ActivityKind::Focused);
        assert_eq!(
            position.as_ref().unwrap().label.as_deref(),
            Some("p.42/500")
        );
    }

    #[test]
    fn spotlight_title_is_preferred_over_the_file_name() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("the-origin-of-species.pdf", 320);
        fixture
            .desktop
            .titles
            .borrow_mut()
            .insert(book.clone(), "The Origin of Species".into());
        fixture.write_view_state(&[(&book, "2026-10-15T19:30:00Z", 9)]);
        fixture.desktop.front.replace(None);
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert_eq!(
            positions(&events),
            vec![("The Origin of Species".to_string(), "p.10/320".to_string())]
        );
    }

    #[test]
    fn documents_outside_the_library_are_ignored() {
        // given
        let fixture = Fixture::new();
        fixture
            .desktop
            .preview_document
            .replace(Some(PathBuf::from("/Users/reader/Downloads/invoice.pdf")));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert!(events.is_empty());
    }

    #[test]
    fn a_long_page_without_input_still_counts() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("war-and-peace.pdf", 500);
        fixture.desktop.preview_document.replace(Some(book));
        fixture
            .desktop
            .idle
            .replace(Some(SignedDuration::from_mins(5)));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert!(matches!(
            events.last(),
            Some(ReaderEvent::Activity {
                kind: ActivityKind::Focused,
                ..
            })
        ));
    }

    #[test]
    fn no_input_past_the_away_limit_is_reported_away() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("war-and-peace.pdf", 500);
        fixture.desktop.preview_document.replace(Some(book));
        fixture
            .desktop
            .idle
            .replace(Some(SignedDuration::from_mins(20)));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        let Some(ReaderEvent::Away { last_input_at, .. }) = events.last() else {
            panic!("expected away, got {events:?}");
        };
        assert_eq!(
            *last_input_at,
            "2026-10-16T08:40:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[test]
    fn preview_in_background_is_not_reading() {
        // given
        let fixture = Fixture::new();
        let book = fixture.book("war-and-peace.pdf", 500);
        fixture.desktop.preview_document.replace(Some(book));
        fixture
            .desktop
            .front
            .replace(Some("com.raphaelamorim.rio".into()));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert!(events.is_empty());
    }

    #[test]
    fn renamed_file_keeps_its_position_through_the_inode() {
        // given
        let fixture = Fixture::new();
        let original = fixture.book("Long Title (Z-Library).pdf", 320);
        fixture.write_view_state(&[(&original, "2026-10-15T19:30:00Z", 9)]);
        let renamed = fixture.library[0].join("the-origin-of-species.pdf");
        fs::rename(&original, &renamed).unwrap();
        fixture.desktop.pages.borrow_mut().insert(renamed, 320);
        fixture.desktop.front.replace(None);
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-16T09:00:00Z");

        // then
        assert_eq!(
            positions(&events),
            vec![("The Origin Of Species".to_string(), "p.10/320".to_string())]
        );
    }
}
