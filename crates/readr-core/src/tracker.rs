use anyhow::Result;
use jiff::SignedDuration;
use jiff::tz::TimeZone;

use crate::db::Db;
use crate::readers::{BookRef, PollContext, ReaderAdapter, ReaderEvent};
use crate::session::{Activity, Change, Engine, Rules};

const LATE_POSITION_WINDOW: SignedDuration = SignedDuration::from_mins(30);

pub struct Tracker {
    engine: Engine,
    adapters: Vec<Box<dyn ReaderAdapter>>,
}

#[derive(Debug, Default)]
pub struct PollReport {
    pub events: usize,
    pub errors: Vec<String>,
}

impl Tracker {
    pub fn new(
        db: &Db,
        rules: Rules,
        tz: TimeZone,
        adapters: Vec<Box<dyn ReaderAdapter>>,
    ) -> Result<Self> {
        let mut engine = Engine::new(rules, tz);
        engine.restore(db.open_sessions()?);
        Ok(Self { engine, adapters })
    }

    pub fn set_rules(&mut self, rules: Rules) {
        self.engine.set_rules(rules);
    }

    pub fn poll(&mut self, db: &Db, ctx: &PollContext<'_>) -> Result<PollReport> {
        let mut report = PollReport::default();
        let mut events = Vec::new();
        for adapter in &mut self.adapters {
            match adapter.poll(ctx) {
                Ok(adapter_events) => events.extend(adapter_events),
                Err(error) => report
                    .errors
                    .push(format!("{}: {error:#}", adapter.reader())),
            }
        }
        report.events = events.len();
        for event in events {
            let changes = self.handle(db, event)?;
            self.apply(db, changes)?;
        }
        let changes = self.engine.tick(ctx.now);
        self.apply(db, changes)?;
        Ok(report)
    }

    fn handle(&mut self, db: &Db, event: ReaderEvent) -> Result<Vec<Change>> {
        match event {
            ReaderEvent::Activity {
                book,
                at,
                kind,
                position,
            } => {
                let Some(source) = self.tracked(db, &book)? else {
                    return Ok(Vec::new());
                };
                match &position {
                    Some(position) => db.set_source_position(source.id, position, at)?,
                    None => db.touch_source(source.id, at)?,
                }
                Ok(self.engine.record(Activity {
                    source_id: source.id,
                    at,
                    kind,
                    position,
                    prior_position: source.last_position,
                }))
            }
            ReaderEvent::Position { book, at, position } => {
                let Some(source) = self.tracked(db, &book)? else {
                    return Ok(Vec::new());
                };
                db.set_source_position(source.id, &position, at)?;
                let changes = self.engine.update_position(source.id, position.clone());
                if changes.is_empty() {
                    db.update_recent_session_end(source.id, &position, at - LATE_POSITION_WINDOW)?;
                }
                Ok(changes)
            }
            ReaderEvent::Away {
                book,
                last_input_at,
            } => match db.source_by_locator(book.reader, &book.locator)? {
                Some(source) if !source.untracked => Ok(self.engine.away(source.id, last_input_at)),
                _ => Ok(Vec::new()),
            },
            ReaderEvent::Closed { reader, at } => {
                let open: Vec<i64> = self.engine.open_sessions().map(|s| s.source_id).collect();
                let mut changes = Vec::new();
                for source_id in open {
                    if db.source(source_id)?.is_some_and(|s| s.reader == reader) {
                        changes.extend(self.engine.close_source(source_id, at));
                    }
                }
                Ok(changes)
            }
        }
    }

    fn tracked(&mut self, db: &Db, book: &BookRef) -> Result<Option<crate::db::Source>> {
        let source = upsert(db, book)?;
        if !source.untracked && !db.is_untracked_title(&source.display_title())? {
            return Ok(Some(source));
        }
        if !source.untracked {
            db.mark_untracked(source.id)?;
        }
        self.engine.forget(source.id);
        Ok(None)
    }

    fn apply(&mut self, db: &Db, changes: Vec<Change>) -> Result<()> {
        for change in changes {
            match change {
                Change::Save(session) => {
                    let id = db.save_session(&session, true)?;
                    self.engine.assign_id(session.source_id, id);
                }
                Change::Finish(session) => {
                    db.save_session(&session, false)?;
                }
                Change::Discard(session) => {
                    if let Some(id) = session.id {
                        db.delete_session(id)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn upsert(db: &Db, book: &BookRef) -> Result<crate::db::Source> {
    let mut source = db.upsert_source(
        book.reader,
        &book.locator,
        book.title.as_deref(),
        book.author.as_deref(),
    )?;
    if source.collections != book.collections {
        db.set_source_collections(source.id, &book.collections)?;
        source.collections = book.collections.clone();
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use jiff::civil::Date;

    use super::*;
    use crate::model::{Position, Reader};
    use crate::readers::fake::FakeDesktop;
    use crate::session::ActivityKind;

    type Script = Rc<RefCell<VecDeque<Vec<ReaderEvent>>>>;

    struct Scripted {
        reader: Reader,
        script: Script,
    }

    impl ReaderAdapter for Scripted {
        fn reader(&self) -> Reader {
            self.reader
        }

        fn poll(&mut self, _ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
            Ok(self.script.borrow_mut().pop_front().unwrap_or_default())
        }
    }

    struct Failing;

    impl ReaderAdapter for Failing {
        fn reader(&self) -> Reader {
            Reader::AppleBooks
        }

        fn poll(&mut self, _ctx: &PollContext<'_>) -> Result<Vec<ReaderEvent>> {
            anyhow::bail!("Full Disk Access missing")
        }
    }

    fn rules() -> Rules {
        Rules {
            page_gap: SignedDuration::from_mins(5),
            last_page: SignedDuration::from_mins(1),
            focus_gap: SignedDuration::from_secs(45),
            min_session: SignedDuration::from_secs(60),
        }
    }

    fn book() -> BookRef {
        BookRef {
            reader: Reader::Bookokrat,
            locator: "/Books/walden.pdf".into(),
            title: Some("Walden".into()),
            author: None,
            collections: Vec::new(),
        }
    }

    fn turn(at: &str, page: u32) -> ReaderEvent {
        ReaderEvent::Activity {
            book: book(),
            at: at.parse().unwrap(),
            kind: ActivityKind::PageTurn,
            position: Some(Position::new(None, Some(format!("p.{page}")))),
        }
    }

    struct Harness {
        db: Db,
        tracker: Tracker,
        script: Script,
        desktop: FakeDesktop,
    }

    impl Harness {
        fn new(extra: Vec<Box<dyn ReaderAdapter>>) -> Self {
            let db = Db::open_in_memory().unwrap();
            let script: Script = Rc::default();
            let mut adapters: Vec<Box<dyn ReaderAdapter>> = vec![Box::new(Scripted {
                reader: Reader::Bookokrat,
                script: script.clone(),
            })];
            adapters.extend(extra);
            let tracker = Tracker::new(&db, rules(), TimeZone::UTC, adapters).unwrap();
            Self {
                db,
                tracker,
                script,
                desktop: FakeDesktop::default(),
            }
        }

        fn poll(&mut self, now: &str, events: Vec<ReaderEvent>) -> PollReport {
            self.script.borrow_mut().push_back(events);
            let ctx = PollContext {
                now: now.parse().unwrap(),
                desktop: &self.desktop,
                away_after: SignedDuration::from_mins(15),
                window_focus: true,
                library_dirs: &[],
            };
            self.tracker.poll(&self.db, &ctx).unwrap()
        }

        fn today(&self) -> Vec<crate::db::SessionView> {
            let day = Date::constant(2026, 10, 16);
            self.db.sessions_between(day, day).unwrap()
        }
    }

    #[test]
    fn page_turns_are_stored_as_an_open_session_while_reading() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );

        // when
        harness.poll(
            "2026-10-16T09:03:00Z",
            vec![turn("2026-10-16T09:03:00Z", 5)],
        );

        // then
        let sessions = harness.today();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].open);
        assert_eq!(sessions[0].active, SignedDuration::from_mins(3));
        assert_eq!(sessions[0].title, "Walden");
    }

    #[test]
    fn session_is_finished_after_reading_stops() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        harness.poll(
            "2026-10-16T09:04:00Z",
            vec![turn("2026-10-16T09:04:00Z", 5)],
        );

        // when
        harness.poll("2026-10-16T09:20:00Z", Vec::new());

        // then
        let sessions = harness.today();
        assert!(!sessions[0].open);
        assert_eq!(sessions[0].active, SignedDuration::from_mins(5));
        assert!(harness.db.open_sessions().unwrap().is_empty());
    }

    #[test]
    fn reader_exit_closes_only_its_sessions() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        harness.poll(
            "2026-10-16T09:02:00Z",
            vec![turn("2026-10-16T09:02:00Z", 5)],
        );

        // when
        harness.poll(
            "2026-10-16T09:02:30Z",
            vec![ReaderEvent::Closed {
                reader: Reader::Bookokrat,
                at: "2026-10-16T09:02:30Z".parse().unwrap(),
            }],
        );

        // then
        let sessions = harness.today();
        assert!(!sessions[0].open);
        assert_eq!(sessions[0].active, SignedDuration::from_secs(150));
    }

    #[test]
    fn late_position_update_fills_the_finished_session_end() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        harness.poll(
            "2026-10-16T09:04:00Z",
            vec![turn("2026-10-16T09:04:00Z", 5)],
        );
        harness.poll("2026-10-16T09:20:00Z", Vec::new());

        // when
        harness.poll(
            "2026-10-16T09:21:00Z",
            vec![ReaderEvent::Position {
                book: book(),
                at: "2026-10-16T09:21:00Z".parse().unwrap(),
                position: Position::new(None, Some("p.9".into())),
            }],
        );

        // then
        let sessions = harness.today();
        let end = sessions[0].end_position.as_ref().unwrap();
        assert_eq!(end.label.as_deref(), Some("p.9"));
    }

    #[test]
    fn previous_position_becomes_the_session_start() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T08:00:00Z",
            vec![ReaderEvent::Position {
                book: book(),
                at: "2026-10-15T21:00:00Z".parse().unwrap(),
                position: Position::new(None, Some("p.3".into())),
            }],
        );

        // when
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );

        // then
        let sessions = harness.today();
        let start = sessions[0].start_position.as_ref().unwrap();
        assert_eq!(start.label.as_deref(), Some("p.3"));
    }

    #[test]
    fn stepping_away_trims_the_stored_session() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        let start: jiff::Timestamp = "2026-10-16T09:00:00Z".parse().unwrap();
        for step in 1..=80 {
            let at = start + SignedDuration::from_secs(15 * step);
            harness.poll(
                &at.to_string(),
                vec![ReaderEvent::Activity {
                    book: book(),
                    at,
                    kind: ActivityKind::Focused,
                    position: None,
                }],
            );
        }

        // when
        harness.poll(
            "2026-10-16T09:20:15Z",
            vec![ReaderEvent::Away {
                book: book(),
                last_input_at: "2026-10-16T09:00:00Z".parse().unwrap(),
            }],
        );
        harness.poll("2026-10-16T09:21:30Z", Vec::new());

        // then
        let sessions = harness.today();
        assert!(!sessions[0].open);
        assert_eq!(sessions[0].active, SignedDuration::from_mins(5));
    }

    #[test]
    fn untracked_books_are_not_recorded() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        let source = harness
            .db
            .source_by_locator(Reader::Bookokrat, "/Books/walden.pdf")
            .unwrap()
            .unwrap();
        harness
            .db
            .forget(
                &[source.id],
                "Walden",
                false,
                "2026-10-16T09:01:00Z".parse().unwrap(),
            )
            .unwrap();

        // when
        harness.poll(
            "2026-10-16T09:03:00Z",
            vec![turn("2026-10-16T09:03:00Z", 5)],
        );
        harness.poll("2026-10-16T09:20:00Z", Vec::new());

        // then
        assert!(harness.today().is_empty());
        let source = harness.db.source(source.id).unwrap().unwrap();
        assert_eq!(source.last_position, None);
    }

    #[test]
    fn a_new_copy_of_a_forgotten_title_is_skipped_too() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness
            .db
            .forget(
                &[],
                "Walden",
                false,
                "2026-10-16T08:00:00Z".parse().unwrap(),
            )
            .unwrap();
        let copy = BookRef {
            reader: Reader::Preview,
            locator: "/Downloads/walden-annotated.pdf".into(),
            title: Some("Walden: Or, Life in the Woods".into()),
            author: None,
            collections: Vec::new(),
        };

        // when
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![ReaderEvent::Activity {
                book: copy,
                at: "2026-10-16T09:00:00Z".parse().unwrap(),
                kind: ActivityKind::Focused,
                position: None,
            }],
        );

        // then
        assert!(harness.today().is_empty());
        let source = harness
            .db
            .source_by_locator(Reader::Preview, "/Downloads/walden-annotated.pdf")
            .unwrap()
            .unwrap();
        assert!(source.untracked);
    }

    #[test]
    fn failing_adapter_is_reported_without_stopping_others() {
        // given
        let mut harness = Harness::new(vec![Box::new(Failing)]);

        // when
        let report = harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );

        // then
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("Full Disk Access"));
        assert_eq!(harness.today().len(), 1);
    }

    #[test]
    fn open_session_survives_a_tracker_restart() {
        // given
        let mut harness = Harness::new(Vec::new());
        harness.poll(
            "2026-10-16T09:00:00Z",
            vec![turn("2026-10-16T09:00:00Z", 4)],
        );
        let script: Script = Rc::default();
        harness.tracker = Tracker::new(
            &harness.db,
            rules(),
            TimeZone::UTC,
            vec![Box::new(Scripted {
                reader: Reader::Bookokrat,
                script: script.clone(),
            })],
        )
        .unwrap();
        harness.script = script;

        // when
        harness.poll(
            "2026-10-16T09:03:00Z",
            vec![turn("2026-10-16T09:03:00Z", 5)],
        );

        // then
        let sessions = harness.today();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].active, SignedDuration::from_mins(3));
    }
}
