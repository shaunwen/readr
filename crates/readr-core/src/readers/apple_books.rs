use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OpenFlags};

use super::desktop::BOOKS_BUNDLE;
use super::{BookRef, PollContext, ReaderAdapter, ReaderEvent};
use crate::model::{Position, Reader};
use crate::session::ActivityKind;

const CORE_DATA_EPOCH_OFFSET: f64 = 978_307_200.0;

pub fn default_library_dir(home: &Path) -> PathBuf {
    home.join("Library/Containers/com.apple.iBooksX/Data/Documents/BKLibrary")
}

#[derive(Debug, Clone, PartialEq)]
struct Asset {
    id: String,
    title: Option<String>,
    author: Option<String>,
    progress: Option<f64>,
    last_open: Option<Timestamp>,
    collections: Vec<String>,
}

impl Asset {
    fn book(&self) -> BookRef {
        BookRef {
            reader: Reader::AppleBooks,
            locator: self.id.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            collections: self.collections.clone(),
        }
    }

    fn position(&self) -> Position {
        Position::new(self.progress, None)
    }
}

pub struct AppleBooks {
    dir: PathBuf,
    signature: Option<Vec<SystemTime>>,
    assets: Vec<Asset>,
    progress: HashMap<String, Option<f64>>,
}

impl AppleBooks {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            signature: None,
            assets: Vec::new(),
            progress: HashMap::new(),
        }
    }

    fn database(&self) -> Option<PathBuf> {
        let entries = std::fs::read_dir(&self.dir).ok()?;
        entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("BKLibrary-") && name.ends_with(".sqlite"))
            })
            .max_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
    }

    fn refresh(&mut self, now: Timestamp) -> Result<Vec<ReaderEvent>> {
        let Some(database) = self.database() else {
            return Ok(Vec::new());
        };
        let signature = file_signature(&database);
        if self.signature.as_ref() == Some(&signature) {
            return Ok(Vec::new());
        }
        self.assets = read_assets(&database)?;
        self.signature = Some(signature);

        let mut events = Vec::new();
        for asset in &self.assets {
            if self.progress.get(&asset.id) == Some(&asset.progress) {
                continue;
            }
            self.progress.insert(asset.id.clone(), asset.progress);
            events.push(ReaderEvent::Position {
                book: asset.book(),
                at: asset.last_open.unwrap_or(now).min(now),
                position: asset.position(),
            });
        }
        Ok(events)
    }

    fn current(&self) -> Option<&Asset> {
        self.assets
            .iter()
            .filter(|asset| asset.last_open.is_some())
            .max_by_key(|asset| asset.last_open)
    }
}

pub fn check(dir: &Path) -> Result<usize> {
    let adapter = AppleBooks::new(dir.to_path_buf());
    let database = adapter
        .database()
        .with_context(|| format!("no BKLibrary-*.sqlite in {}", dir.display()))?;
    Ok(read_assets(&database)?.len())
}

impl ReaderAdapter for AppleBooks {
    fn reader(&self) -> Reader {
        Reader::AppleBooks
    }

    fn poll(&mut self, ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
        let mut events = self.refresh(ctx.now)?;
        let front = ctx.desktop.frontmost_bundle_id();
        if front.as_deref() == Some(BOOKS_BUNDLE)
            && ctx.user_is_active()
            && let Some(asset) = self.current()
        {
            events.push(ReaderEvent::Activity {
                book: asset.book(),
                at: ctx.now,
                kind: ActivityKind::Focused,
                position: Some(asset.position()),
            });
        }
        Ok(events)
    }
}

fn file_signature(database: &Path) -> Vec<SystemTime> {
    let wal = PathBuf::from(format!("{}-wal", database.display()));
    [database.to_path_buf(), wal]
        .iter()
        .filter_map(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .collect()
}

fn read_assets(database: &Path) -> Result<Vec<Asset>> {
    let conn = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening {}", database.display()))?;
    let mut stmt = conn.prepare(
        "SELECT ZASSETID, ZTITLE, ZAUTHOR, ZREADINGPROGRESS, ZLASTOPENDATE
         FROM ZBKLIBRARYASSET WHERE ZASSETID IS NOT NULL",
    )?;
    let mut collections = read_collections(&conn).unwrap_or_default();
    let rows = stmt.query_map([], |row| {
        Ok(Asset {
            id: row.get(0)?,
            title: row.get(1)?,
            author: row.get(2)?,
            progress: row.get(3)?,
            last_open: row.get::<_, Option<f64>>(4)?.and_then(core_data_time),
            collections: Vec::new(),
        })
    })?;
    let mut assets: Vec<Asset> = rows.collect::<rusqlite::Result<_>>()?;
    for asset in &mut assets {
        asset.collections = collections.remove(&asset.id).unwrap_or_default();
    }
    Ok(assets)
}

fn read_collections(conn: &Connection) -> rusqlite::Result<HashMap<String, Vec<String>>> {
    let mut stmt = conn.prepare(
        "SELECT m.ZASSETID, c.ZTITLE FROM ZBKCOLLECTIONMEMBER m
         JOIN ZBKCOLLECTION c ON c.Z_PK = m.ZCOLLECTION
         WHERE m.ZASSETID IS NOT NULL AND c.ZTITLE IS NOT NULL AND coalesce(c.ZDELETEDFLAG, 0) = 0
         ORDER BY c.ZTITLE",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut collections: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (asset, title) = row?;
        collections.entry(asset).or_default().push(title);
    }
    Ok(collections)
}

fn core_data_time(seconds: f64) -> Option<Timestamp> {
    let unix = seconds + CORE_DATA_EPOCH_OFFSET;
    Timestamp::from_duration(SignedDuration::try_from_secs_f64(unix).ok()?).ok()
}

#[cfg(test)]
mod tests {
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
                desktop: FakeDesktop::active_in(BOOKS_BUNDLE),
            };
            let conn = Connection::open(fixture.database()).unwrap();
            conn.execute_batch(
                "CREATE TABLE ZBKLIBRARYASSET (Z_PK INTEGER PRIMARY KEY, ZASSETID VARCHAR,
                     ZTITLE VARCHAR, ZAUTHOR VARCHAR, ZREADINGPROGRESS FLOAT, ZLASTOPENDATE TIMESTAMP)",
            )
            .unwrap();
            fixture
        }

        fn database(&self) -> PathBuf {
            self.dir.path().join("BKLibrary-1-091020131601.sqlite")
        }

        fn upsert(&self, id: &str, title: &str, progress: f64, last_open: &str) {
            let last_open: Timestamp = last_open.parse().unwrap();
            let core_data = last_open.as_second() as f64 - CORE_DATA_EPOCH_OFFSET;
            let conn = Connection::open(self.database()).unwrap();
            conn.execute("DELETE FROM ZBKLIBRARYASSET WHERE ZASSETID = ?1", [id])
                .unwrap();
            conn.execute(
                "INSERT INTO ZBKLIBRARYASSET (ZASSETID, ZTITLE, ZAUTHOR, ZREADINGPROGRESS, ZLASTOPENDATE)
                 VALUES (?1, ?2, 'Author', ?3, ?4)",
                rusqlite::params![id, title, progress, core_data],
            )
            .unwrap();
            let bump =
                SystemTime::now() + std::time::Duration::from_secs(progress as u64 * 1000 + 1);
            std::fs::File::options()
                .write(true)
                .open(self.database())
                .unwrap()
                .set_modified(bump)
                .unwrap();
        }

        fn poll(&self, adapter: &mut AppleBooks, now: &str) -> Vec<ReaderEvent> {
            let ctx = PollContext {
                now: now.parse().unwrap(),
                desktop: &self.desktop,
                idle_limit: SignedDuration::from_mins(2),
                library_dirs: &[],
            };
            adapter.poll(&ctx).unwrap()
        }

        fn adapter(&self) -> AppleBooks {
            AppleBooks::new(self.dir.path().to_path_buf())
        }
    }

    #[test]
    fn front_books_app_reads_the_most_recently_opened_book() {
        // given
        let fixture = Fixture::new();
        fixture.upsert("A1", "Middlemarch", 0.10, "2026-10-15T10:00:00Z");
        fixture.upsert("B2", "Moby Dick", 0.20, "2026-10-15T21:00:00Z");
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-15T21:10:00Z");

        // then
        let Some(ReaderEvent::Activity { book, kind, .. }) = events.last() else {
            panic!("expected activity, got {events:?}");
        };
        assert_eq!(book.locator, "B2");
        assert_eq!(book.title.as_deref(), Some("Moby Dick"));
        assert_eq!(*kind, ActivityKind::Focused);
    }

    #[test]
    fn collections_are_attached_to_their_books() {
        // given
        let fixture = Fixture::new();
        fixture.upsert("A1", "Meditations", 0.30, "2026-10-15T10:30:00Z");
        let conn = Connection::open(fixture.database()).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZBKCOLLECTION (Z_PK INTEGER PRIMARY KEY, ZTITLE VARCHAR, ZDELETEDFLAG INTEGER);
             CREATE TABLE ZBKCOLLECTIONMEMBER (Z_PK INTEGER PRIMARY KEY, ZCOLLECTION INTEGER, ZASSETID VARCHAR);
             INSERT INTO ZBKCOLLECTION VALUES (1, 'Health', 0), (2, 'Old', 1);
             INSERT INTO ZBKCOLLECTIONMEMBER VALUES (1, 1, 'A1'), (2, 2, 'A1');",
        )
        .unwrap();
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-15T21:00:00Z");

        // then
        let Some(ReaderEvent::Activity { book, .. }) = events.last() else {
            panic!("expected activity, got {events:?}");
        };
        assert_eq!(book.collections, vec!["Health"]);
    }

    #[test]
    fn progress_changes_are_position_updates() {
        // given
        let fixture = Fixture::new();
        fixture.desktop.front.replace(None);
        fixture.upsert("A1", "Meditations", 0.30, "2026-10-15T10:30:00Z");
        let mut adapter = fixture.adapter();
        fixture.poll(&mut adapter, "2026-10-15T21:00:00Z");
        fixture.upsert("A1", "Meditations", 0.35, "2026-10-15T21:05:00Z");

        // when
        let events = fixture.poll(&mut adapter, "2026-10-15T21:06:00Z");

        // then
        let [ReaderEvent::Position { at, position, .. }] = events.as_slice() else {
            panic!("expected one position, got {events:?}");
        };
        assert_eq!(position.progress, Some(0.35));
        assert_eq!(*at, "2026-10-15T21:05:00Z".parse::<Timestamp>().unwrap());
    }

    #[test]
    fn books_in_background_is_not_reading() {
        // given
        let fixture = Fixture::new();
        fixture.upsert("A1", "Meditations", 0.30, "2026-10-15T10:30:00Z");
        fixture
            .desktop
            .front
            .replace(Some("com.raphaelamorim.rio".into()));
        let mut adapter = fixture.adapter();

        // when
        let events = fixture.poll(&mut adapter, "2026-10-15T21:00:00Z");

        // then
        assert!(
            events
                .iter()
                .all(|event| matches!(event, ReaderEvent::Position { .. }))
        );
    }

    #[test]
    fn core_data_timestamps_start_in_2001() {
        // given
        let seconds = 0.0;

        // when
        let time = core_data_time(seconds).unwrap();

        // then
        assert_eq!(time, "2001-01-01T00:00:00Z".parse::<Timestamp>().unwrap());
    }

    #[test]
    fn missing_library_is_not_an_error() {
        // given
        let fixture = Fixture::new();
        let mut adapter = AppleBooks::new(fixture.dir.path().join("missing"));

        // when
        let events = fixture.poll(&mut adapter, "2026-10-15T21:00:00Z");

        // then
        assert!(events.is_empty());
    }
}
