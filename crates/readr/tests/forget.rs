use assert_cmd::Command;
use jiff::{SignedDuration, Timestamp, Zoned};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use readr_core::db::{Db, NewBook};
use readr_core::model::{BookStatus, Position, Reader};
use readr_core::session::{ActivityKind, OpenSession};
use tempfile::TempDir;

struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Self {
        let env = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        let db = env.db();
        let book_id = db
            .add_book(
                &NewBook {
                    title: "Walden".into(),
                    author: None,
                    goal: Some("learning".into()),
                    status: BookStatus::Active,
                },
                Timestamp::now(),
            )
            .unwrap();
        let walden = db
            .upsert_source(Reader::Bookokrat, "/Books/walden.pdf", Some("Walden"), None)
            .unwrap();
        db.link_source(walden.id, Some(book_id)).unwrap();
        let middlemarch = db
            .upsert_source(
                Reader::Preview,
                "/Books/middlemarch.pdf",
                Some("Middlemarch"),
                None,
            )
            .unwrap();
        save_session(&db, walden.id, 12);
        save_session(&db, middlemarch.id, 7);
        env
    }

    fn db(&self) -> Db {
        Db::open(&self.dir.path().join("data/readr/readr.db")).unwrap()
    }

    fn readr(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        Command::cargo_bin("readr")
            .unwrap()
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("cfg"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .args(args)
            .assert()
    }
}

fn save_session(db: &Db, source_id: i64, minutes: i64) {
    let now = Zoned::now();
    db.save_session(
        &OpenSession {
            id: None,
            source_id,
            day: now.date(),
            started_at: now.timestamp() - SignedDuration::from_mins(minutes),
            last_activity_at: now.timestamp(),
            last_kind: ActivityKind::Focused,
            active: SignedDuration::from_mins(minutes),
            start_position: Some(Position::new(None, Some("p.4".into()))),
            end_position: Some(Position::new(None, Some("p.9".into()))),
        },
        false,
    )
    .unwrap();
}

#[test]
fn a_shelf_book_is_not_forgotten_without_confirmation() {
    // given
    let env = Env::new();

    // when
    let assert = env.readr(&["forget", "Walden"]);

    // then
    assert.failure().stderr(contains("--yes"));
    env.readr(&["status"]).success().stdout(contains("Walden"));
}

#[test]
fn forgetting_removes_the_history_and_the_shelf_entry() {
    // given
    let env = Env::new();

    // when
    let assert = env.readr(&["forget", "Walden", "--yes"]);

    // then
    assert.success().stdout(contains(
        "forgot Walden (bookokrat): 1 session deleted, removed from the shelf",
    ));
    env.readr(&["status"])
        .success()
        .stdout(contains("Walden").not());
    env.readr(&["shelf"])
        .success()
        .stdout(contains("Walden").not());
    env.readr(&["forget", "--list"])
        .success()
        .stdout(contains("Walden"));
}

#[test]
fn track_brings_a_forgotten_book_back() {
    // given
    let env = Env::new();
    env.readr(&["forget", "Walden", "--yes"]).success();

    // when
    let assert = env.readr(&["track", "walden"]);

    // then
    assert.success().stdout(contains("tracking Walden again"));
    env.readr(&["forget", "--list"])
        .success()
        .stdout(contains("no forgotten books"));
}

#[test]
fn adding_a_forgotten_book_to_the_shelf_tracks_it_again() {
    // given
    let env = Env::new();
    env.readr(&["forget", "Middlemarch"]).success();

    // when
    let assert = env.readr(&["shelf", "add", "Middlemarch", "--goal", "learning"]);

    // then
    assert
        .success()
        .stdout(contains("it was forgotten; tracking it again"));
    env.readr(&["forget", "--list"])
        .success()
        .stdout(contains("no forgotten books"));
}

#[test]
fn keep_history_stops_tracking_but_leaves_past_sessions() {
    // given
    let env = Env::new();

    // when
    let assert = env.readr(&["forget", "Middlemarch", "--keep-history"]);

    // then
    assert.success().stdout(contains("past sessions kept"));
    env.readr(&["status"])
        .success()
        .stdout(contains("Middlemarch"));
}

#[test]
fn forget_needs_a_book_or_list() {
    // given
    let env = Env::new();

    // when
    let assert = env.readr(&["forget"]);

    // then
    assert.failure().stderr(contains("name the book to forget"));
}
