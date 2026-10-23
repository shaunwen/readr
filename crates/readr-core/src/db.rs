use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::config::{Config, Goal};
use crate::find::title_key;
use crate::model::{BookStatus, Position, Reader};
use crate::session::{ActivityKind, OpenSession};
use crate::settings::SETTINGS;

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE books (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    author      TEXT,
    goal        TEXT,
    status      TEXT NOT NULL CHECK (status IN ('active', 'queued', 'done', 'dropped')),
    priority    INTEGER NOT NULL DEFAULT 0,
    added_at    TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE book_sources (
    id            INTEGER PRIMARY KEY,
    book_id       INTEGER REFERENCES books(id) ON DELETE SET NULL,
    reader        TEXT NOT NULL,
    locator       TEXT NOT NULL,
    title         TEXT,
    author        TEXT,
    ignored       INTEGER NOT NULL DEFAULT 0,
    last_progress REAL,
    last_position TEXT,
    last_seen_at  TEXT,
    UNIQUE (reader, locator)
);

CREATE TABLE sessions (
    id             INTEGER PRIMARY KEY,
    source_id      INTEGER NOT NULL REFERENCES book_sources(id) ON DELETE CASCADE,
    goal           TEXT,
    day            TEXT NOT NULL,
    started_at     TEXT NOT NULL,
    ended_at       TEXT NOT NULL,
    last_kind      TEXT NOT NULL,
    active_secs    INTEGER NOT NULL,
    open           INTEGER NOT NULL DEFAULT 0,
    start_progress REAL,
    start_position TEXT,
    end_progress   REAL,
    end_position   TEXT,
    note           TEXT,
    edited         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_day ON sessions(day);
CREATE INDEX sessions_source ON sessions(source_id);

CREATE TABLE notifications (
    day     TEXT NOT NULL,
    kind    TEXT NOT NULL,
    sent_at TEXT NOT NULL,
    PRIMARY KEY (day, kind)
);
"#,
    r#"
ALTER TABLE book_sources ADD COLUMN collections TEXT;
"#,
    r#"
ALTER TABLE book_sources ADD COLUMN untracked INTEGER NOT NULL DEFAULT 0;
CREATE TABLE untracked_titles (
    key          TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    untracked_at TEXT NOT NULL
);
"#,
    r#"
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE goals (
    key               TEXT PRIMARY KEY,
    position          INTEGER NOT NULL,
    label             TEXT NOT NULL,
    color             TEXT,
    weekly_minutes    INTEGER,
    folders           TEXT NOT NULL,
    books_collections TEXT NOT NULL
);
"#,
];

#[derive(Debug, Clone, PartialEq)]
pub struct Book {
    pub id: i64,
    pub title: String,
    pub author: Option<String>,
    pub goal: Option<String>,
    pub status: BookStatus,
    pub priority: i64,
    pub added_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewBook {
    pub title: String,
    pub author: Option<String>,
    pub goal: Option<String>,
    pub status: BookStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub id: i64,
    pub book_id: Option<i64>,
    pub reader: Reader,
    pub locator: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub ignored: bool,
    pub last_position: Option<Position>,
    pub last_seen_at: Option<Timestamp>,
    pub collections: Vec<String>,
    pub untracked: bool,
}

impl Source {
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .or_else(|| crate::readers::file_title(&self.locator))
            .unwrap_or_else(|| self.locator.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub id: i64,
    pub source_id: i64,
    pub book_id: Option<i64>,
    pub title: String,
    pub reader: Reader,
    pub goal: Option<String>,
    pub day: Date,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
    pub active: SignedDuration,
    pub start_position: Option<Position>,
    pub end_position: Option<Position>,
    pub open: bool,
    pub edited: bool,
    pub note: Option<String>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        for (index, migration) in (0_i64..).zip(MIGRATIONS).skip_while(|(i, _)| *i < version) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(migration)?;
            tx.pragma_update(None, "user_version", index + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn add_book(&self, book: &NewBook, now: Timestamp) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO books (title, author, goal, status, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                book.title,
                book.author,
                book.goal,
                book.status.as_str(),
                now.to_string()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn books(&self) -> Result<Vec<Book>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, author, goal, status, priority, added_at, finished_at
             FROM books ORDER BY priority, id",
        )?;
        let rows = stmt.query_map([], book_from_row)?;
        rows.map(|row| row?).collect()
    }

    pub fn book(&self, id: i64) -> Result<Option<Book>> {
        self.conn
            .query_row(
                "SELECT id, title, author, goal, status, priority, added_at, finished_at
                 FROM books WHERE id = ?1",
                [id],
                book_from_row,
            )
            .optional()?
            .transpose()
    }

    pub fn set_book_status(&self, id: i64, status: BookStatus, now: Timestamp) -> Result<()> {
        let finished_at = (status == BookStatus::Done).then(|| now.to_string());
        self.conn.execute(
            "UPDATE books SET status = ?2, finished_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), finished_at],
        )?;
        Ok(())
    }

    pub fn set_book_priority(&self, id: i64, priority: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE books SET priority = ?2 WHERE id = ?1",
            params![id, priority],
        )?;
        Ok(())
    }

    pub fn set_book_goal(&self, id: i64, goal: Option<&str>, move_history: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE books SET goal = ?2 WHERE id = ?1",
            params![id, goal],
        )?;
        if move_history {
            tx.execute(
                "UPDATE sessions SET goal = ?2
                 WHERE source_id IN (SELECT id FROM book_sources WHERE book_id = ?1)",
                params![id, goal],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn rename_goal(&self, old: &str, new: &str) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let books = tx.execute("UPDATE books SET goal = ?2 WHERE goal = ?1", [old, new])?;
        tx.execute("UPDATE sessions SET goal = ?2 WHERE goal = ?1", [old, new])?;
        tx.execute("UPDATE goals SET key = ?2 WHERE key = ?1", [old, new])?;
        tx.commit()?;
        Ok(books)
    }

    pub fn config(&self) -> Result<Option<Config>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let settings: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if settings.is_empty() {
            return Ok(None);
        }
        let mut config = Config::parse("")?;
        for (key, value) in &settings {
            if crate::settings::setting(key).is_some() {
                config
                    .set(key, value)
                    .with_context(|| format!("setting {key} in the database"))?;
            }
        }
        let mut stmt = self.conn.prepare(
            "SELECT key, label, color, weekly_minutes, folders, books_collections
             FROM goals ORDER BY position, key",
        )?;
        config.goals = stmt
            .query_map([], |row| {
                Ok((
                    Goal {
                        key: row.get(0)?,
                        label: row.get(1)?,
                        color: row.get(2)?,
                        weekly_minutes: row.get(3)?,
                        folders: Vec::new(),
                        books_collections: Vec::new(),
                    },
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?
            .map(|row| {
                let (mut goal, folders, collections) = row?;
                goal.folders = serde_json::from_str(&folders)?;
                goal.books_collections = serde_json::from_str(&collections)?;
                Ok(goal)
            })
            .collect::<Result<_>>()?;
        config.validate().context("settings in the database")?;
        Ok(Some(config))
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM settings", [])?;
        for field in SETTINGS {
            if let Some(value) = config.get(field.key) {
                tx.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                    params![field.key, value],
                )?;
            }
        }
        tx.execute("DELETE FROM goals", [])?;
        for (position, goal) in (0_i64..).zip(&config.goals) {
            tx.execute(
                "INSERT INTO goals (key, position, label, color, weekly_minutes, folders, books_collections)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    goal.key,
                    position,
                    goal.label,
                    goal.color,
                    goal.weekly_minutes,
                    serde_json::to_string(&goal.folders)?,
                    serde_json::to_string(&goal.books_collections)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn goals_in_use(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT goal FROM books WHERE goal IS NOT NULL
             UNION SELECT goal FROM sessions WHERE goal IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn upsert_source(
        &self,
        reader: Reader,
        locator: &str,
        title: Option<&str>,
        author: Option<&str>,
    ) -> Result<Source> {
        self.conn.execute(
            "INSERT INTO book_sources (reader, locator, title, author) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (reader, locator) DO UPDATE SET
                 title = coalesce(excluded.title, title),
                 author = coalesce(excluded.author, author)",
            params![reader.as_str(), locator, title, author],
        )?;
        self.source_by_locator(reader, locator)?
            .ok_or_else(|| anyhow!("source {reader}:{locator} vanished after insert"))
    }

    pub fn source(&self, id: i64) -> Result<Option<Source>> {
        self.conn
            .query_row(
                &format!("{SOURCE_SELECT} WHERE id = ?1"),
                [id],
                source_from_row,
            )
            .optional()?
            .transpose()
    }

    pub fn source_by_locator(&self, reader: Reader, locator: &str) -> Result<Option<Source>> {
        self.conn
            .query_row(
                &format!("{SOURCE_SELECT} WHERE reader = ?1 AND locator = ?2"),
                params![reader.as_str(), locator],
                source_from_row,
            )
            .optional()?
            .transpose()
    }

    pub fn sources_for_book(&self, book_id: i64) -> Result<Vec<Source>> {
        self.query_sources(
            &format!("{SOURCE_SELECT} WHERE book_id = ?1 ORDER BY last_seen_at DESC"),
            [book_id],
        )
    }

    pub fn inbox_sources(&self) -> Result<Vec<Source>> {
        self.query_sources(
            &format!(
                "{SOURCE_SELECT} WHERE book_id IS NULL AND ignored = 0 AND untracked = 0
                 AND id IN (SELECT source_id FROM sessions)
                 ORDER BY last_seen_at DESC"
            ),
            [],
        )
    }

    pub fn all_sources(&self) -> Result<Vec<Source>> {
        self.query_sources(&format!("{SOURCE_SELECT} ORDER BY last_seen_at DESC"), [])
    }

    fn query_sources(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Source>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params, source_from_row)?;
        rows.map(|row| row?).collect()
    }

    pub fn set_source_position(
        &self,
        id: i64,
        position: &Position,
        seen_at: Timestamp,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE book_sources SET last_progress = ?2, last_position = ?3,
                 last_seen_at = max(coalesce(last_seen_at, ''), ?4)
             WHERE id = ?1",
            params![id, position.progress, position.label, seen_at.to_string()],
        )?;
        Ok(())
    }

    pub fn set_source_collections(&self, id: i64, collections: &[String]) -> Result<()> {
        let joined = (!collections.is_empty()).then(|| collections.join("\n"));
        self.conn.execute(
            "UPDATE book_sources SET collections = ?2 WHERE id = ?1",
            params![id, joined],
        )?;
        Ok(())
    }

    pub fn touch_source(&self, id: i64, seen_at: Timestamp) -> Result<()> {
        self.conn.execute(
            "UPDATE book_sources SET last_seen_at = max(coalesce(last_seen_at, ''), ?2)
             WHERE id = ?1",
            params![id, seen_at.to_string()],
        )?;
        Ok(())
    }

    pub fn update_recent_session_end(
        &self,
        source_id: i64,
        position: &Position,
        ended_since: Timestamp,
    ) -> Result<bool> {
        let updated = self.conn.execute(
            "UPDATE sessions SET end_progress = ?2, end_position = ?3
             WHERE id = (SELECT id FROM sessions
                         WHERE source_id = ?1 AND open = 0 AND edited = 0 AND ended_at >= ?4
                         ORDER BY ended_at DESC LIMIT 1)",
            params![
                source_id,
                position.progress,
                position.label,
                ended_since.to_string()
            ],
        )?;
        Ok(updated > 0)
    }

    pub fn link_source(&self, source_id: i64, book_id: Option<i64>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE book_sources SET book_id = ?2, ignored = 0 WHERE id = ?1",
            params![source_id, book_id],
        )?;
        if let Some(book_id) = book_id {
            tx.execute(
                "UPDATE sessions SET goal = (SELECT goal FROM books WHERE id = ?2)
                 WHERE source_id = ?1 AND goal IS NULL",
                params![source_id, book_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn ignore_source(&self, source_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE book_sources SET ignored = 1 WHERE id = ?1",
            [source_id],
        )?;
        Ok(())
    }

    pub fn save_session(&self, session: &OpenSession, open: bool) -> Result<i64> {
        let start = session.start_position.as_ref();
        let end = session.end_position.as_ref();
        let values = params![
            session.source_id,
            session.day.to_string(),
            session.started_at.to_string(),
            session.ended_at().to_string(),
            session.last_kind.as_str(),
            session.active.as_secs(),
            open,
            start.and_then(|p| p.progress),
            start.and_then(|p| p.label.clone()),
            end.and_then(|p| p.progress),
            end.and_then(|p| p.label.clone()),
        ];
        match session.id {
            Some(id) => {
                let mut stmt = self.conn.prepare_cached(
                    "UPDATE sessions SET source_id = ?1, day = ?2, started_at = ?3, ended_at = ?4,
                         last_kind = ?5, active_secs = ?6, open = ?7, start_progress = ?8,
                         start_position = ?9, end_progress = ?10, end_position = ?11
                     WHERE id = ?12 AND edited = 0",
                )?;
                let mut values = values.to_vec();
                values.push(&id);
                stmt.execute(values.as_slice())?;
                Ok(id)
            }
            None => {
                let mut stmt = self.conn.prepare_cached(
                    "INSERT INTO sessions (source_id, day, started_at, ended_at, last_kind,
                         active_secs, open, start_progress, start_position, end_progress,
                         end_position, goal)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                         (SELECT b.goal FROM book_sources s JOIN books b ON b.id = s.book_id
                          WHERE s.id = ?1))",
                )?;
                stmt.execute(values)?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    pub fn delete_session(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn open_sessions(&self) -> Result<Vec<OpenSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_id, day, started_at, ended_at, last_kind, active_secs,
                    start_progress, start_position, end_progress, end_position
             FROM sessions WHERE open = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((|| -> Result<OpenSession> {
                Ok(OpenSession {
                    id: Some(row.get(0)?),
                    source_id: row.get(1)?,
                    day: row.get::<_, String>(2)?.parse()?,
                    started_at: row.get::<_, String>(3)?.parse()?,
                    last_activity_at: row.get::<_, String>(4)?.parse()?,
                    last_kind: ActivityKind::parse(&row.get::<_, String>(5)?)
                        .ok_or_else(|| anyhow!("unknown activity kind"))?,
                    active: SignedDuration::from_secs(row.get(6)?),
                    start_position: position_from(row.get(7)?, row.get(8)?),
                    end_position: position_from(row.get(9)?, row.get(10)?),
                })
            })())
        })?;
        rows.map(|row| row?).collect()
    }

    pub fn add_manual_session(
        &self,
        book_id: i64,
        started_at: Timestamp,
        day: Date,
        active: SignedDuration,
        note: Option<&str>,
    ) -> Result<i64> {
        let source = self.upsert_source(Reader::Manual, &format!("book:{book_id}"), None, None)?;
        self.link_source(source.id, Some(book_id))?;
        let ended_at = started_at + active;
        self.conn.execute(
            "INSERT INTO sessions (source_id, goal, day, started_at, ended_at, last_kind,
                 active_secs, note, edited)
             VALUES (?1, (SELECT goal FROM books WHERE id = ?2), ?3, ?4, ?5, 'focused', ?6, ?7, 1)",
            params![
                source.id,
                book_id,
                day.to_string(),
                started_at.to_string(),
                ended_at.to_string(),
                active.as_secs(),
                note
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn edit_session(&self, id: i64, active: SignedDuration, source_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET active_secs = ?2, source_id = ?3, edited = 1,
                 goal = coalesce(
                     (SELECT b.goal FROM book_sources s JOIN books b ON b.id = s.book_id
                      WHERE s.id = ?3),
                     goal)
             WHERE id = ?1 AND open = 0",
            params![id, active.as_secs(), source_id],
        )?;
        Ok(())
    }

    pub fn sessions_between(&self, from: Date, to: Date) -> Result<Vec<SessionView>> {
        let mut stmt = self.conn.prepare(
            "SELECT se.id, se.source_id, so.book_id, coalesce(b.title, so.title, so.locator),
                    so.reader, se.goal, se.day, se.started_at, se.ended_at, se.active_secs,
                    se.start_progress, se.start_position, se.end_progress, se.end_position,
                    se.open, se.edited, se.note
             FROM sessions se
             JOIN book_sources so ON so.id = se.source_id
             LEFT JOIN books b ON b.id = so.book_id
             WHERE se.day BETWEEN ?1 AND ?2
             ORDER BY se.started_at",
        )?;
        let rows = stmt.query_map([from.to_string(), to.to_string()], |row| {
            Ok((|| -> Result<SessionView> {
                Ok(SessionView {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    book_id: row.get(2)?,
                    title: row.get(3)?,
                    reader: row.get::<_, String>(4)?.parse()?,
                    goal: row.get(5)?,
                    day: row.get::<_, String>(6)?.parse()?,
                    started_at: row.get::<_, String>(7)?.parse()?,
                    ended_at: row.get::<_, String>(8)?.parse()?,
                    active: SignedDuration::from_secs(row.get(9)?),
                    start_position: position_from(row.get(10)?, row.get(11)?),
                    end_position: position_from(row.get(12)?, row.get(13)?),
                    open: row.get(14)?,
                    edited: row.get(15)?,
                    note: row.get(16)?,
                })
            })())
        })?;
        rows.map(|row| row?).collect()
    }

    pub fn daily_totals(
        &self,
        from: Date,
        to: Date,
        count_off_shelf: bool,
    ) -> Result<BTreeMap<Date, SignedDuration>> {
        let mut stmt = self.conn.prepare(
            "SELECT se.day, sum(se.active_secs)
             FROM sessions se JOIN book_sources so ON so.id = se.source_id
             WHERE se.day BETWEEN ?1 AND ?2 AND (?3 OR so.book_id IS NOT NULL)
             GROUP BY se.day",
        )?;
        let rows = stmt.query_map(
            params![from.to_string(), to.to_string(), count_off_shelf],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let mut totals = BTreeMap::new();
        for row in rows {
            let (day, secs) = row?;
            totals.insert(day.parse()?, SignedDuration::from_secs(secs));
        }
        Ok(totals)
    }

    pub fn day_total(&self, day: Date, count_off_shelf: bool) -> Result<SignedDuration> {
        Ok(self
            .daily_totals(day, day, count_off_shelf)?
            .remove(&day)
            .unwrap_or(SignedDuration::ZERO))
    }

    pub fn book_totals(&self) -> Result<BTreeMap<i64, SignedDuration>> {
        let mut stmt = self.conn.prepare(
            "SELECT so.book_id, sum(se.active_secs)
             FROM sessions se JOIN book_sources so ON so.id = se.source_id
             WHERE so.book_id IS NOT NULL GROUP BY so.book_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)?)))?;
        let mut totals = BTreeMap::new();
        for row in rows {
            let (book_id, secs) = row?;
            totals.insert(book_id, SignedDuration::from_secs(secs));
        }
        Ok(totals)
    }

    pub fn source_totals(&self) -> Result<HashMap<i64, SignedDuration>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_id, sum(active_secs) FROM sessions GROUP BY source_id")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)?)))?;
        let mut totals = HashMap::new();
        for row in rows {
            let (source_id, secs) = row?;
            totals.insert(source_id, SignedDuration::from_secs(secs));
        }
        Ok(totals)
    }

    pub fn progress_read_by_book(&self) -> Result<HashMap<i64, f64>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT so.book_id, sum(min(se.end_progress - se.start_progress,
                                        se.active_secs / 60.0 * {MAX_PROGRESS_PER_MINUTE}))
             FROM sessions se JOIN book_sources so ON so.id = se.source_id
             WHERE so.book_id IS NOT NULL AND se.end_progress > se.start_progress
             GROUP BY so.book_id"
        ))?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, f64>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn last_read_by_book(&self) -> Result<HashMap<i64, Timestamp>> {
        let mut stmt = self.conn.prepare(
            "SELECT so.book_id, max(se.ended_at)
             FROM sessions se JOIN book_sources so ON so.id = se.source_id
             WHERE so.book_id IS NOT NULL GROUP BY so.book_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?;
        let mut last = HashMap::new();
        for row in rows {
            let (book_id, ended_at) = row?;
            last.insert(book_id, ended_at.parse()?);
        }
        Ok(last)
    }

    pub fn goal_totals(&self, from: Date, to: Date) -> Result<HashMap<String, SignedDuration>> {
        let mut stmt = self.conn.prepare(
            "SELECT goal, sum(active_secs) FROM sessions
             WHERE goal IS NOT NULL AND day BETWEEN ?1 AND ?2 GROUP BY goal",
        )?;
        let rows = stmt.query_map([from.to_string(), to.to_string()], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut totals = HashMap::new();
        for row in rows {
            let (goal, secs) = row?;
            totals.insert(goal, SignedDuration::from_secs(secs));
        }
        Ok(totals)
    }

    pub fn forget(
        &self,
        source_ids: &[i64],
        title: &str,
        keep_history: bool,
        at: Timestamp,
    ) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut sessions = 0;
        for source_id in source_ids {
            tx.execute(
                "UPDATE book_sources SET untracked = 1, last_progress = NULL, last_position = NULL
                 WHERE id = ?1",
                [source_id],
            )?;
            sessions += if keep_history {
                tx.execute(
                    "UPDATE sessions SET open = 0 WHERE source_id = ?1 AND open = 1",
                    [source_id],
                )?;
                0
            } else {
                tx.execute("DELETE FROM sessions WHERE source_id = ?1", [source_id])?
            };
        }
        tx.execute(
            "INSERT OR REPLACE INTO untracked_titles (key, title, untracked_at) VALUES (?1, ?2, ?3)",
            params![title_key(title), title, at.to_string()],
        )?;
        tx.commit()?;
        Ok(sessions)
    }

    pub fn track_again(&self, title: &str) -> Result<usize> {
        let key = title_key(title);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM untracked_titles WHERE key = ?1", [&key])?;
        let mut stmt =
            tx.prepare("SELECT id, title, locator FROM book_sources WHERE untracked = 1")?;
        let rows: Vec<(i64, Option<String>, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        let mut count = 0;
        for (id, source_title, locator) in rows {
            let display = source_title
                .or_else(|| crate::readers::file_title(&locator))
                .unwrap_or(locator);
            if title_key(&display) == key {
                count += tx.execute("UPDATE book_sources SET untracked = 0 WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn is_untracked_title(&self, title: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM untracked_titles WHERE key = ?1",
                [title_key(title)],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mark_untracked(&self, source_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE book_sources SET untracked = 1, last_progress = NULL, last_position = NULL
             WHERE id = ?1",
            [source_id],
        )?;
        Ok(())
    }

    pub fn untracked_titles(&self) -> Result<Vec<(String, Timestamp)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT title, untracked_at FROM untracked_titles ORDER BY title")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut titles = Vec::new();
        for row in rows {
            let (title, at) = row?;
            titles.push((title, at.parse()?));
        }
        Ok(titles)
    }

    pub fn delete_book(&self, book_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM books WHERE id = ?1", [book_id])?;
        Ok(())
    }

    pub fn notified(&self, day: Date, kind: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM notifications WHERE day = ?1 AND kind = ?2",
                [day.to_string(), kind.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mark_notified(&self, day: Date, kind: &str, at: Timestamp) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO notifications (day, kind, sent_at) VALUES (?1, ?2, ?3)",
            [day.to_string(), kind.to_string(), at.to_string()],
        )?;
        Ok(inserted > 0)
    }
}

// A forward move faster than 1% of the book per minute of reading is a jump, not reading.
const MAX_PROGRESS_PER_MINUTE: f64 = 0.01;

const SOURCE_SELECT: &str = "SELECT id, book_id, reader, locator, title, author, ignored,
    last_progress, last_position, last_seen_at, collections, untracked FROM book_sources";

fn source_from_row(row: &Row<'_>) -> rusqlite::Result<Result<Source>> {
    Ok((|| -> Result<Source> {
        Ok(Source {
            id: row.get(0)?,
            book_id: row.get(1)?,
            reader: row.get::<_, String>(2)?.parse()?,
            locator: row.get(3)?,
            title: row.get(4)?,
            author: row.get(5)?,
            ignored: row.get(6)?,
            last_position: position_from(row.get(7)?, row.get(8)?),
            last_seen_at: row
                .get::<_, Option<String>>(9)?
                .map(|t| t.parse())
                .transpose()?,
            collections: row
                .get::<_, Option<String>>(10)?
                .map(|text| text.lines().map(str::to_string).collect())
                .unwrap_or_default(),
            untracked: row.get(11)?,
        })
    })())
}

fn book_from_row(row: &Row<'_>) -> rusqlite::Result<Result<Book>> {
    Ok((|| -> Result<Book> {
        Ok(Book {
            id: row.get(0)?,
            title: row.get(1)?,
            author: row.get(2)?,
            goal: row.get(3)?,
            status: row.get::<_, String>(4)?.parse()?,
            priority: row.get(5)?,
            added_at: row.get::<_, String>(6)?.parse()?,
            finished_at: row
                .get::<_, Option<String>>(7)?
                .map(|t| t.parse())
                .transpose()?,
        })
    })())
}

fn position_from(progress: Option<f64>, label: Option<String>) -> Option<Position> {
    (progress.is_some() || label.is_some()).then(|| Position::new(progress, label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-10-16T09:00:00Z".parse().unwrap()
    }

    fn day() -> Date {
        Date::constant(2026, 10, 16)
    }

    fn book(goal: &str) -> NewBook {
        NewBook {
            title: "Walden".into(),
            author: Some("Henry David Thoreau".into()),
            goal: Some(goal.into()),
            status: BookStatus::Active,
        }
    }

    fn session(source_id: i64, minutes: i64) -> OpenSession {
        OpenSession {
            id: None,
            source_id,
            day: day(),
            started_at: now(),
            last_activity_at: now() + SignedDuration::from_mins(minutes),
            last_kind: ActivityKind::PageTurn,
            active: SignedDuration::from_mins(minutes),
            start_position: Some(Position::new(Some(0.1), Some("p.9".into()))),
            end_position: Some(Position::new(Some(0.12), Some("p.12".into()))),
        }
    }

    #[test]
    fn migrations_are_idempotent_on_reopen() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/readr.db");
        Db::open(&path).unwrap();

        // when
        let reopened = Db::open(&path);

        // then
        assert!(reopened.is_ok());
    }

    #[test]
    fn upserting_a_source_twice_keeps_one_row_and_fills_metadata() {
        // given
        let db = Db::open_in_memory().unwrap();
        let first = db
            .upsert_source(Reader::Preview, "/books/a.pdf", None, None)
            .unwrap();

        // when
        let second = db
            .upsert_source(Reader::Preview, "/books/a.pdf", Some("A"), None)
            .unwrap();

        // then
        assert_eq!(first.id, second.id);
        assert_eq!(second.title.as_deref(), Some("A"));
    }

    #[test]
    fn source_collections_round_trip() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::AppleBooks, "asset-1", Some("Meditations"), None)
            .unwrap();

        // when
        db.set_source_collections(source.id, &["Library".into(), "Health".into()])
            .unwrap();

        // then
        let source = db.source(source.id).unwrap().unwrap();
        assert_eq!(source.collections, vec!["Library", "Health"]);
    }

    #[test]
    fn session_snapshots_the_goal_of_its_book() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("communication"), now()).unwrap();
        let source = db
            .upsert_source(Reader::Bookokrat, "/books/s.pdf", None, None)
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();

        // when
        db.save_session(&session(source.id, 7), false).unwrap();

        // then
        let sessions = db.sessions_between(day(), day()).unwrap();
        assert_eq!(sessions[0].goal.as_deref(), Some("communication"));
        assert_eq!(sessions[0].title, "Walden");
        assert_eq!(sessions[0].active, SignedDuration::from_mins(7));
        assert_eq!(
            sessions[0].end_position.as_ref().unwrap().label.as_deref(),
            Some("p.12")
        );
    }

    #[test]
    fn off_shelf_time_counts_only_when_enabled() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("health"), now()).unwrap();
        let shelf = db
            .upsert_source(Reader::AppleBooks, "asset-1", None, None)
            .unwrap();
        db.link_source(shelf.id, Some(book_id)).unwrap();
        let stray = db
            .upsert_source(Reader::Preview, "/tmp/invoice.pdf", None, None)
            .unwrap();
        db.save_session(&session(shelf.id, 12), false).unwrap();
        db.save_session(&session(stray.id, 5), false).unwrap();

        // when
        let shelf_only = db.day_total(day(), false).unwrap();
        let everything = db.day_total(day(), true).unwrap();

        // then
        assert_eq!(shelf_only, SignedDuration::from_mins(12));
        assert_eq!(everything, SignedDuration::from_mins(17));
    }

    #[test]
    fn linking_an_inbox_source_makes_its_past_sessions_count() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Preview, "/books/new.pdf", None, None)
            .unwrap();
        db.save_session(&session(source.id, 9), false).unwrap();
        assert_eq!(db.inbox_sources().unwrap().len(), 1);
        let book_id = db.add_book(&book("learning"), now()).unwrap();

        // when
        db.link_source(source.id, Some(book_id)).unwrap();

        // then
        assert!(db.inbox_sources().unwrap().is_empty());
        assert_eq!(
            db.day_total(day(), false).unwrap(),
            SignedDuration::from_mins(9)
        );
        let sessions = db.sessions_between(day(), day()).unwrap();
        assert_eq!(sessions[0].goal.as_deref(), Some("learning"));
    }

    #[test]
    fn open_sessions_round_trip_for_restart() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Bookokrat, "/books/s.pdf", None, None)
            .unwrap();
        let mut open = session(source.id, 3);
        open.id = Some(db.save_session(&open, true).unwrap());

        // when
        let restored = db.open_sessions().unwrap();

        // then
        assert_eq!(restored, vec![open]);
    }

    #[test]
    fn edited_sessions_are_not_overwritten_by_the_daemon() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Bookokrat, "/books/s.pdf", None, None)
            .unwrap();
        let mut finished = session(source.id, 10);
        finished.id = Some(db.save_session(&finished, false).unwrap());
        db.edit_session(
            finished.id.unwrap(),
            SignedDuration::from_mins(4),
            source.id,
        )
        .unwrap();

        // when
        db.save_session(&session_with_id(&finished, 15), false)
            .unwrap();

        // then
        let sessions = db.sessions_between(day(), day()).unwrap();
        assert_eq!(sessions[0].active, SignedDuration::from_mins(4));
        assert!(sessions[0].edited);
    }

    fn session_with_id(base: &OpenSession, minutes: i64) -> OpenSession {
        OpenSession {
            active: SignedDuration::from_mins(minutes),
            ..base.clone()
        }
    }

    #[test]
    fn moving_a_book_to_another_goal_can_move_its_history() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("health"), now()).unwrap();
        let source = db
            .upsert_source(Reader::AppleBooks, "asset-1", None, None)
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();
        db.save_session(&session(source.id, 10), false).unwrap();

        // when
        db.set_book_goal(book_id, Some("learning"), false).unwrap();
        let kept = db.sessions_between(day(), day()).unwrap()[0].goal.clone();
        db.set_book_goal(book_id, Some("learning"), true).unwrap();
        let moved = db.sessions_between(day(), day()).unwrap()[0].goal.clone();

        // then
        assert_eq!(kept.as_deref(), Some("health"));
        assert_eq!(moved.as_deref(), Some("learning"));
    }

    #[test]
    fn renaming_a_goal_updates_books_and_sessions() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("competence"), now()).unwrap();
        let source = db
            .upsert_source(Reader::AppleBooks, "asset-1", None, None)
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();
        db.save_session(&session(source.id, 10), false).unwrap();

        // when
        let renamed = db.rename_goal("competence", "craft").unwrap();

        // then
        assert_eq!(renamed, 1);
        assert_eq!(db.goals_in_use().unwrap(), vec!["craft".to_string()]);
    }

    #[test]
    fn a_new_database_has_no_settings_yet() {
        // given
        let db = Db::open_in_memory().unwrap();

        // when
        let config = db.config().unwrap();

        // then
        assert_eq!(config, None);
    }

    #[test]
    fn saved_settings_and_goals_load_back_in_order() {
        // given
        let db = Db::open_in_memory().unwrap();
        let config = Config::parse(crate::config::DEFAULT_CONFIG_TOML)
            .unwrap()
            .with_setting("daily_goal_minutes", "30")
            .unwrap()
            .with_goal_moved("competence", -3)
            .with_goal_field("health", "books_collections", "Health, Food")
            .unwrap();

        // when
        db.save_config(&config).unwrap();

        // then
        assert_eq!(db.config().unwrap(), Some(config));
    }

    #[test]
    fn renaming_a_goal_renames_its_settings_row() {
        // given
        let db = Db::open_in_memory().unwrap();
        db.save_config(&Config::parse(crate::config::DEFAULT_CONFIG_TOML).unwrap())
            .unwrap();

        // when
        db.rename_goal("competence", "craft").unwrap();

        // then
        let config = db.config().unwrap().unwrap();
        assert_eq!(config.goals[3].key, "craft");
        assert_eq!(config.goals[3].label, "Circle of competence");
    }

    #[test]
    fn settings_from_a_newer_readr_are_ignored() {
        // given
        let db = Db::open_in_memory().unwrap();
        db.save_config(&Config::parse("").unwrap()).unwrap();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('future.knob', 'x')",
                [],
            )
            .unwrap();

        // when
        let config = db.config().unwrap();

        // then
        assert_eq!(config, Some(Config::parse("").unwrap()));
    }

    #[test]
    fn manual_session_counts_toward_its_book() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("health"), now()).unwrap();

        // when
        db.add_manual_session(
            book_id,
            now(),
            day(),
            SignedDuration::from_mins(25),
            Some("paper"),
        )
        .unwrap();

        // then
        assert_eq!(
            db.day_total(day(), false).unwrap(),
            SignedDuration::from_mins(25)
        );
        assert_eq!(
            db.book_totals().unwrap().get(&book_id),
            Some(&SignedDuration::from_mins(25))
        );
    }

    #[test]
    fn notifications_are_recorded_once_per_day_and_kind() {
        // given
        let db = Db::open_in_memory().unwrap();

        // when
        let first = db.mark_notified(day(), "nightly", now()).unwrap();
        let second = db.mark_notified(day(), "nightly", now()).unwrap();

        // then
        assert!(first);
        assert!(!second);
        assert!(db.notified(day(), "nightly").unwrap());
        assert!(!db.notified(day(), "remind@09:00").unwrap());
    }

    #[test]
    fn goal_totals_and_last_read_follow_sessions() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("health"), now()).unwrap();
        let source = db
            .upsert_source(Reader::AppleBooks, "asset-1", None, None)
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();
        db.save_session(&session(source.id, 10), false).unwrap();
        db.save_session(&session(source.id, 6), false).unwrap();

        // when
        let goals = db.goal_totals(day(), day()).unwrap();
        let last = db.last_read_by_book().unwrap();

        // then
        assert_eq!(goals.get("health"), Some(&SignedDuration::from_mins(16)));
        assert_eq!(
            last.get(&book_id),
            Some(&(now() + SignedDuration::from_mins(10)))
        );
    }

    #[test]
    fn progress_read_counts_forward_reading_and_caps_jumps() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("learning"), now()).unwrap();
        let source = db
            .upsert_source(Reader::Preview, "/books/a.pdf", None, None)
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();
        let mut reading = session(source.id, 10);
        reading.start_position = Some(Position::new(Some(0.10), None));
        reading.end_position = Some(Position::new(Some(0.13), None));
        let mut jump = session(source.id, 2);
        jump.start_position = Some(Position::new(Some(0.13), None));
        jump.end_position = Some(Position::new(Some(0.63), None));
        let mut backwards = session(source.id, 5);
        backwards.start_position = Some(Position::new(Some(0.63), None));
        backwards.end_position = Some(Position::new(Some(0.20), None));
        for s in [&reading, &jump, &backwards] {
            db.save_session(s, false).unwrap();
        }

        // when
        let read = db.progress_read_by_book().unwrap()[&book_id];

        // then
        assert!((read - (0.03 + 0.02)).abs() < 1e-9, "{read}");
    }

    #[test]
    fn forgetting_deletes_history_clears_the_place_and_remembers_the_title() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Bookokrat, "/books/walden.pdf", Some("Walden"), None)
            .unwrap();
        db.set_source_position(
            source.id,
            &Position::new(Some(0.4), Some("p.40".into())),
            now(),
        )
        .unwrap();
        db.save_session(&session(source.id, 12), false).unwrap();

        // when
        let deleted = db.forget(&[source.id], "Walden", false, now()).unwrap();

        // then
        let source = db.source(source.id).unwrap().unwrap();
        assert_eq!(deleted, 1);
        assert!(source.untracked);
        assert_eq!(source.last_position, None);
        assert!(db.sessions_between(day(), day()).unwrap().is_empty());
        assert!(
            db.is_untracked_title("walden: or, life in the woods")
                .unwrap()
        );
        assert_eq!(db.untracked_titles().unwrap()[0].0, "Walden");
    }

    #[test]
    fn forgetting_with_history_keeps_sessions_but_closes_open_ones() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Preview, "/books/walden.pdf", Some("Walden"), None)
            .unwrap();
        db.save_session(&session(source.id, 12), true).unwrap();

        // when
        let deleted = db.forget(&[source.id], "Walden", true, now()).unwrap();

        // then
        assert_eq!(deleted, 0);
        let sessions = db.sessions_between(day(), day()).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].open);
    }

    #[test]
    fn tracking_again_clears_every_copy_with_that_title() {
        // given
        let db = Db::open_in_memory().unwrap();
        let pdf = db
            .upsert_source(Reader::Bookokrat, "/books/walden.pdf", Some("Walden"), None)
            .unwrap();
        let books = db
            .upsert_source(Reader::AppleBooks, "asset-9", Some("Walden"), None)
            .unwrap();
        db.forget(&[pdf.id, books.id], "Walden", false, now())
            .unwrap();

        // when
        let tracked = db.track_again("Walden").unwrap();

        // then
        assert_eq!(tracked, 2);
        assert!(!db.source(pdf.id).unwrap().unwrap().untracked);
        assert!(!db.is_untracked_title("Walden").unwrap());
    }

    #[test]
    fn untracked_books_stay_out_of_the_inbox() {
        // given
        let db = Db::open_in_memory().unwrap();
        let source = db
            .upsert_source(Reader::Preview, "/books/walden.pdf", Some("Walden"), None)
            .unwrap();
        db.save_session(&session(source.id, 5), false).unwrap();

        // when
        db.forget(&[source.id], "Walden", true, now()).unwrap();

        // then
        assert!(db.inbox_sources().unwrap().is_empty());
    }

    #[test]
    fn finishing_a_book_records_when() {
        // given
        let db = Db::open_in_memory().unwrap();
        let book_id = db.add_book(&book("health"), now()).unwrap();

        // when
        db.set_book_status(book_id, BookStatus::Done, now())
            .unwrap();

        // then
        let book = db.book(book_id).unwrap().unwrap();
        assert_eq!(book.status, BookStatus::Done);
        assert_eq!(book.finished_at, Some(now()));
    }
}
