use assert_cmd::Command;
use jiff::{SignedDuration, Timestamp, Zoned};
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
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::cargo_bin("readr").unwrap();
        cmd.env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("cfg"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"));
        cmd
    }

    fn db(&self) -> Db {
        Db::open(&self.dir.path().join("data/readr/readr.db")).unwrap()
    }

    fn read_today(&self, title: &str, minutes: i64, open: bool) {
        let db = self.db();
        let now = Zoned::now();
        let book_id = db
            .add_book(
                &NewBook {
                    title: title.into(),
                    author: None,
                    goal: Some("communication".into()),
                    status: BookStatus::Active,
                },
                Timestamp::now(),
            )
            .unwrap();
        let source = db
            .upsert_source(
                Reader::Bookokrat,
                &format!("/Books/{title}.pdf"),
                None,
                None,
            )
            .unwrap();
        db.link_source(source.id, Some(book_id)).unwrap();
        let started = now.timestamp() - SignedDuration::from_mins(minutes);
        db.save_session(
            &OpenSession {
                id: None,
                source_id: source.id,
                day: now.date(),
                started_at: started,
                last_activity_at: now.timestamp(),
                last_kind: ActivityKind::PageTurn,
                active: SignedDuration::from_mins(minutes),
                start_position: Some(Position::new(None, Some("p.4".into()))),
                end_position: Some(Position::new(None, Some("p.9".into()))),
            },
            open,
        )
        .unwrap();
    }
}

#[test]
fn fresh_install_reports_no_reading() {
    // given
    let env = Env::new();

    // when
    let assert = env.command().arg("status").assert();

    // then
    assert
        .success()
        .stdout(contains("Today 0/20 min · streak 0"))
        .stdout(contains("no reading yet"));
}

#[test]
fn short_status_fits_a_prompt() {
    // given
    let env = Env::new();
    env.read_today("Walden", 12, false);

    // when
    let assert = env.command().args(["status", "--short"]).assert();

    // then
    assert.success().stdout("12/20m\n");
}

#[test]
fn status_lists_sessions_with_page_movement() {
    // given
    let env = Env::new();
    env.read_today("Walden", 21, true);

    // when
    let assert = env.command().arg("status").assert();

    // then
    assert
        .success()
        .stdout(contains("Today 21/20 min · streak 1  done"))
        .stdout(contains("Walden"))
        .stdout(contains("p.4 → p.9  (reading)"));
}

#[test]
fn invalid_config_is_reported() {
    // given
    let env = Env::new();
    std::fs::create_dir_all(env.dir.path().join("cfg/readr")).unwrap();
    std::fs::write(
        env.dir.path().join("cfg/readr/config.toml"),
        "[nightly]\nat = \"25:00\"\n",
    )
    .unwrap();

    // when
    let assert = env.command().arg("status").assert();

    // then
    assert.failure().stderr(contains("config.toml"));
}
