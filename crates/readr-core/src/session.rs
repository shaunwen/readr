use std::collections::BTreeMap;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::config::Tracking;
use crate::model::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    PageTurn,
    Focused,
}

impl ActivityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PageTurn => "page_turn",
            Self::Focused => "focused",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "page_turn" => Some(Self::PageTurn),
            "focused" => Some(Self::Focused),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub source_id: i64,
    pub at: Timestamp,
    pub kind: ActivityKind,
    pub position: Option<Position>,
    pub prior_position: Option<Position>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenSession {
    pub id: Option<i64>,
    pub source_id: i64,
    pub day: Date,
    pub started_at: Timestamp,
    pub last_activity_at: Timestamp,
    pub last_kind: ActivityKind,
    pub active: SignedDuration,
    pub start_position: Option<Position>,
    pub end_position: Option<Position>,
}

impl OpenSession {
    pub fn ended_at(&self) -> Timestamp {
        self.last_activity_at
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Save(OpenSession),
    Finish(OpenSession),
    Discard(OpenSession),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    pub page_gap: SignedDuration,
    pub last_page: SignedDuration,
    pub focus_gap: SignedDuration,
    pub min_session: SignedDuration,
}

impl Rules {
    pub fn from_tracking(tracking: &Tracking) -> Self {
        Self {
            page_gap: SignedDuration::from_mins(i64::from(tracking.max_page_minutes)),
            last_page: SignedDuration::from_mins(i64::from(tracking.last_page_minutes)),
            focus_gap: SignedDuration::from_secs(3 * tracking.poll_seconds as i64),
            min_session: SignedDuration::from_secs(i64::from(tracking.min_session_seconds)),
        }
    }

    fn allowed_gap(&self, previous: ActivityKind, next: ActivityKind) -> SignedDuration {
        if previous == ActivityKind::PageTurn || next == ActivityKind::PageTurn {
            self.page_gap.max(self.focus_gap)
        } else {
            self.focus_gap
        }
    }

    fn tail(&self, last_kind: ActivityKind) -> SignedDuration {
        match last_kind {
            ActivityKind::PageTurn => self.last_page,
            ActivityKind::Focused => SignedDuration::ZERO,
        }
    }

    fn timeout(&self, last_kind: ActivityKind) -> SignedDuration {
        match last_kind {
            ActivityKind::PageTurn => self.page_gap.max(self.focus_gap),
            ActivityKind::Focused => self.focus_gap,
        }
    }
}

pub struct Engine {
    rules: Rules,
    tz: TimeZone,
    open: BTreeMap<i64, OpenSession>,
}

impl Engine {
    pub fn new(rules: Rules, tz: TimeZone) -> Self {
        Self {
            rules,
            tz,
            open: BTreeMap::new(),
        }
    }

    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    pub fn restore(&mut self, sessions: impl IntoIterator<Item = OpenSession>) {
        for session in sessions {
            self.open.insert(session.source_id, session);
        }
    }

    pub fn assign_id(&mut self, source_id: i64, id: i64) {
        if let Some(session) = self.open.get_mut(&source_id) {
            session.id = Some(id);
        }
    }

    pub fn open_sessions(&self) -> impl Iterator<Item = &OpenSession> {
        self.open.values()
    }

    pub fn record(&mut self, activity: Activity) -> Vec<Change> {
        let mut changes = Vec::new();
        let Some(mut session) = self.open.remove(&activity.source_id) else {
            let session = self.start(&activity, activity.at);
            changes.push(Change::Save(session.clone()));
            self.open.insert(activity.source_id, session);
            return changes;
        };

        if activity.at <= session.last_activity_at {
            if activity.position.is_some() {
                session.end_position = activity.position;
            }
            changes.push(Change::Save(session.clone()));
            self.open.insert(activity.source_id, session);
            return changes;
        }

        let gap = activity.at.duration_since(session.last_activity_at);
        if gap > self.rules.allowed_gap(session.last_kind, activity.kind) {
            let tail = self.rules.tail(session.last_kind);
            changes.extend(self.finish(session, tail));
            let next = self.start(&activity, activity.at);
            changes.push(Change::Save(next.clone()));
            self.open.insert(activity.source_id, next);
            return changes;
        }

        let day = activity.at.to_zoned(self.tz.clone()).date();
        let session = if day != session.day {
            let midnight = self.start_of_day(day);
            session.active += midnight.duration_since(session.last_activity_at);
            session.last_activity_at = midnight;
            let mut next = self.start(&activity, midnight);
            next.start_position = session.end_position.clone().or(next.start_position);
            changes.extend(self.finish(session, SignedDuration::ZERO));
            next
        } else {
            session
        };

        let mut session = session;
        session.active += activity.at.duration_since(session.last_activity_at);
        session.last_activity_at = activity.at;
        session.last_kind = activity.kind;
        if activity.position.is_some() {
            session.end_position = activity.position;
        }
        changes.push(Change::Save(session.clone()));
        self.open.insert(activity.source_id, session);
        changes
    }

    pub fn tick(&mut self, now: Timestamp) -> Vec<Change> {
        let stale: Vec<i64> = self
            .open
            .values()
            .filter(|s| now.duration_since(s.last_activity_at) > self.rules.timeout(s.last_kind))
            .map(|s| s.source_id)
            .collect();
        let mut changes = Vec::new();
        for source_id in stale {
            let session = self.open.remove(&source_id).expect("stale session is open");
            let tail = self.rules.tail(session.last_kind);
            changes.extend(self.finish(session, tail));
        }
        changes
    }

    pub fn close_source(&mut self, source_id: i64, at: Timestamp) -> Vec<Change> {
        let Some(session) = self.open.remove(&source_id) else {
            return Vec::new();
        };
        let since_last = at
            .duration_since(session.last_activity_at)
            .max(SignedDuration::ZERO);
        let tail = self.rules.tail(session.last_kind).min(since_last);
        self.finish(session, tail)
    }

    pub fn away(&mut self, source_id: i64, last_input_at: Timestamp) -> Vec<Change> {
        let page_gap = self.rules.page_gap;
        let Some(session) = self.open.get_mut(&source_id) else {
            return Vec::new();
        };
        let cutoff = (last_input_at + page_gap).max(session.started_at);
        if session.last_activity_at <= cutoff {
            return Vec::new();
        }
        let excess = session.last_activity_at.duration_since(cutoff);
        session.active = (session.active - excess).max(SignedDuration::ZERO);
        session.last_activity_at = cutoff;
        session.last_kind = ActivityKind::Focused;
        vec![Change::Save(session.clone())]
    }

    pub fn forget(&mut self, source_id: i64) {
        self.open.remove(&source_id);
    }

    pub fn update_position(&mut self, source_id: i64, position: Position) -> Vec<Change> {
        let Some(session) = self.open.get_mut(&source_id) else {
            return Vec::new();
        };
        session.end_position = Some(position);
        vec![Change::Save(session.clone())]
    }

    fn start(&self, activity: &Activity, at: Timestamp) -> OpenSession {
        OpenSession {
            id: None,
            source_id: activity.source_id,
            day: at.to_zoned(self.tz.clone()).date(),
            started_at: at,
            last_activity_at: at,
            last_kind: activity.kind,
            active: SignedDuration::ZERO,
            start_position: activity
                .prior_position
                .clone()
                .or_else(|| activity.position.clone()),
            end_position: activity.position.clone(),
        }
    }

    fn finish(&self, mut session: OpenSession, tail: SignedDuration) -> Vec<Change> {
        let day_end = self.start_of_day(session.day.tomorrow().unwrap_or(session.day));
        let tail = tail.min(day_end.duration_since(session.last_activity_at));
        session.active += tail;
        session.last_activity_at += tail;
        if session.active < self.rules.min_session {
            vec![Change::Discard(session)]
        } else {
            vec![Change::Finish(session)]
        }
    }

    fn start_of_day(&self, day: Date) -> Timestamp {
        day.to_zoned(self.tz.clone())
            .map(|zoned| zoned.timestamp())
            .unwrap_or(Timestamp::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            page_gap: SignedDuration::from_mins(5),
            last_page: SignedDuration::from_mins(1),
            focus_gap: SignedDuration::from_secs(45),
            min_session: SignedDuration::from_secs(60),
        }
    }

    fn engine() -> Engine {
        Engine::new(rules(), TimeZone::get("Australia/Sydney").unwrap())
    }

    fn at(local: &str) -> Timestamp {
        format!("{local}[Australia/Sydney]")
            .parse::<jiff::Zoned>()
            .unwrap()
            .timestamp()
    }

    fn page_turn(source_id: i64, local: &str, page: u32) -> Activity {
        Activity {
            source_id,
            at: at(local),
            kind: ActivityKind::PageTurn,
            position: Some(Position::new(None, Some(format!("p.{page}")))),
            prior_position: None,
        }
    }

    fn focused(source_id: i64, local: &str) -> Activity {
        Activity {
            source_id,
            at: at(local),
            kind: ActivityKind::Focused,
            position: None,
            prior_position: None,
        }
    }

    fn finished(changes: &[Change]) -> Vec<&OpenSession> {
        changes
            .iter()
            .filter_map(|c| match c {
                Change::Finish(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    fn label(position: &Option<Position>) -> Option<&str> {
        position.as_ref().and_then(|p| p.label.as_deref())
    }

    #[test]
    fn page_turns_within_the_gap_accumulate_time() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));

        // when
        engine.record(page_turn(1, "2026-10-16T20:03:00", 11));
        let changes = engine.record(page_turn(1, "2026-10-16T20:07:00", 12));

        // then
        let Change::Save(session) = changes.last().unwrap() else {
            panic!("expected save");
        };
        assert_eq!(session.active, SignedDuration::from_mins(7));
        assert_eq!(label(&session.start_position), Some("p.10"));
        assert_eq!(label(&session.end_position), Some("p.12"));
    }

    #[test]
    fn gap_longer_than_max_page_starts_a_new_session() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        engine.record(page_turn(1, "2026-10-16T20:04:00", 11));

        // when
        let changes = engine.record(page_turn(1, "2026-10-16T20:30:00", 12));

        // then
        let done = finished(&changes);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].active, SignedDuration::from_mins(5));
        assert_eq!(done[0].ended_at(), at("2026-10-16T20:05:00"));
        let open: Vec<_> = engine.open_sessions().collect();
        assert_eq!(open[0].started_at, at("2026-10-16T20:30:00"));
    }

    #[test]
    fn idle_session_is_finished_on_tick_with_last_page_allowance() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        engine.record(page_turn(1, "2026-10-16T20:02:00", 11));

        // when
        let changes = engine.tick(at("2026-10-16T20:08:00"));

        // then
        let done = finished(&changes);
        assert_eq!(done[0].active, SignedDuration::from_mins(3));
        assert_eq!(engine.open_sessions().count(), 0);
    }

    #[test]
    fn tick_within_the_gap_keeps_the_session_open() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));

        // when
        let changes = engine.tick(at("2026-10-16T20:04:00"));

        // then
        assert!(changes.is_empty());
        assert_eq!(engine.open_sessions().count(), 1);
    }

    #[test]
    fn sessions_shorter_than_the_minimum_are_discarded() {
        // given
        let mut engine = engine();
        engine.record(focused(1, "2026-10-16T20:00:00"));
        engine.record(focused(1, "2026-10-16T20:00:15"));

        // when
        let changes = engine.tick(at("2026-10-16T20:05:00"));

        // then
        assert!(matches!(changes.as_slice(), [Change::Discard(_)]));
    }

    #[test]
    fn focused_ticks_count_until_they_stop() {
        // given
        let mut engine = engine();
        let mut seconds = 0;
        while seconds <= 180 {
            let local = format!("2026-10-16T08:{:02}:{:02}", seconds / 60, seconds % 60);
            engine.record(focused(2, &local));
            seconds += 15;
        }

        // when
        let changes = engine.tick(at("2026-10-16T08:05:00"));

        // then
        let done = finished(&changes);
        assert_eq!(done[0].active, SignedDuration::from_mins(3));
    }

    #[test]
    fn focused_gap_longer_than_three_polls_splits_the_session() {
        // given
        let mut engine = engine();
        engine.record(focused(2, "2026-10-16T08:00:00"));
        engine.record(focused(2, "2026-10-16T08:01:30"));

        // when
        let open: Vec<_> = engine.open_sessions().cloned().collect();

        // then
        assert_eq!(open[0].started_at, at("2026-10-16T08:01:30"));
    }

    #[test]
    fn reader_exit_finishes_with_tail_capped_by_exit_time() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        engine.record(page_turn(1, "2026-10-16T20:04:00", 11));

        // when
        let changes = engine.close_source(1, at("2026-10-16T20:04:20"));

        // then
        let done = finished(&changes);
        assert_eq!(done[0].active, SignedDuration::from_secs(4 * 60 + 20));
    }

    #[test]
    fn session_crossing_midnight_is_split_by_day() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T23:57:00", 10));

        // when
        let changes = engine.record(page_turn(1, "2026-10-17T00:02:00", 11));

        // then
        let done = finished(&changes);
        assert_eq!(done[0].day, Date::constant(2026, 10, 16));
        assert_eq!(done[0].active, SignedDuration::from_mins(3));
        let open: Vec<_> = engine.open_sessions().collect();
        assert_eq!(open[0].day, Date::constant(2026, 10, 17));
        assert_eq!(open[0].active, SignedDuration::from_mins(2));
        assert_eq!(label(&open[0].start_position), Some("p.10"));
    }

    #[test]
    fn tail_does_not_spill_into_the_next_day() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T23:55:00", 10));
        engine.record(page_turn(1, "2026-10-16T23:59:30", 11));

        // when
        let changes = engine.tick(at("2026-10-17T00:10:00"));

        // then
        let done = finished(&changes);
        assert_eq!(done[0].active, SignedDuration::from_mins(5));
        assert_eq!(done[0].ended_at(), at("2026-10-17T00:00:00"));
    }

    #[test]
    fn prior_position_becomes_the_start_position() {
        // given
        let mut engine = engine();
        let mut first = page_turn(1, "2026-10-16T20:00:00", 10);
        first.prior_position = Some(Position::new(None, Some("p.9".into())));

        // when
        let changes = engine.record(first);

        // then
        let Change::Save(session) = &changes[0] else {
            panic!("expected save");
        };
        assert_eq!(label(&session.start_position), Some("p.9"));
        assert_eq!(label(&session.end_position), Some("p.10"));
    }

    #[test]
    fn sources_are_tracked_independently() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        engine.record(focused(2, "2026-10-16T20:00:00"));

        // when
        let changes = engine.tick(at("2026-10-16T20:02:00"));

        // then
        assert!(matches!(changes.as_slice(), [Change::Discard(s)] if s.source_id == 2));
        assert_eq!(engine.open_sessions().count(), 1);
    }

    fn ticks(engine: &mut Engine, source_id: i64, from_min: i64, to_min: i64) {
        let start = at("2026-10-16T20:00:00");
        let mut offset = SignedDuration::from_mins(from_min);
        while offset <= SignedDuration::from_mins(to_min) {
            engine.record(Activity {
                source_id,
                at: start + offset,
                kind: ActivityKind::Focused,
                position: None,
                prior_position: None,
            });
            offset += SignedDuration::from_secs(15);
        }
    }

    #[test]
    fn focused_ticks_count_a_page_read_for_longer_than_max_page() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        ticks(&mut engine, 1, 0, 9);

        // when
        let changes = engine.record(page_turn(1, "2026-10-16T20:09:30", 11));

        // then
        let Change::Save(session) = changes.last().unwrap() else {
            panic!("expected save");
        };
        assert_eq!(session.active, SignedDuration::from_secs(9 * 60 + 30));
    }

    #[test]
    fn stepping_away_trims_back_to_last_input_plus_max_page() {
        // given
        let mut engine = engine();
        engine.record(page_turn(1, "2026-10-16T20:00:00", 10));
        ticks(&mut engine, 1, 0, 15);

        // when
        let trimmed = engine.away(1, at("2026-10-16T20:00:00"));
        let finished = engine.tick(at("2026-10-16T20:16:00"));

        // then
        assert!(matches!(trimmed.as_slice(), [Change::Save(_)]));
        let done = finished
            .iter()
            .find_map(|c| match c {
                Change::Finish(s) => Some(s),
                _ => None,
            })
            .expect("finished");
        assert_eq!(done.active, SignedDuration::from_mins(5));
        assert_eq!(done.ended_at(), at("2026-10-16T20:05:00"));
    }

    #[test]
    fn away_before_the_session_started_empties_it() {
        // given
        let mut engine = engine();
        ticks(&mut engine, 2, 10, 12);

        // when
        engine.away(2, at("2026-10-16T19:00:00"));
        let changes = engine.tick(at("2026-10-16T20:20:00"));

        // then
        assert!(matches!(changes.as_slice(), [Change::Discard(_)]));
    }

    #[test]
    fn away_within_the_allowance_changes_nothing() {
        // given
        let mut engine = engine();
        ticks(&mut engine, 2, 0, 4);

        // when
        let changes = engine.away(2, at("2026-10-16T20:01:00"));

        // then
        assert!(changes.is_empty());
    }

    #[test]
    fn restored_session_continues_after_restart() {
        // given
        let mut first = engine();
        first.record(page_turn(1, "2026-10-16T20:00:00", 10));
        let saved: Vec<OpenSession> = first.open_sessions().cloned().collect();
        let mut second = engine();
        second.restore(saved);

        // when
        let changes = second.record(page_turn(1, "2026-10-16T20:03:00", 11));

        // then
        let Change::Save(session) = changes.last().unwrap() else {
            panic!("expected save");
        };
        assert_eq!(session.active, SignedDuration::from_mins(3));
    }
}
