//! Fills a new database with made-up reading for demos and screenshots:
//! public-domain books, about ten weeks of sessions and a reading streak.
//!
//! cargo run -p readr-core --example demo_data -- target/demo/data/readr/readr.db

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use readr_core::config::{Config, DEFAULT_CONFIG_TOML};
use readr_core::db::{Db, NewBook};
use readr_core::model::{BookStatus, Position, Reader};
use readr_core::session::{ActivityKind, OpenSession};

#[derive(Clone, Copy)]
enum Progress {
    Pages(u32),
    Chapters(u32),
    Percent,
}

struct Book {
    title: &'static str,
    author: &'static str,
    goal: &'static str,
    shelf: Option<BookStatus>,
    reader: Reader,
    locator: &'static str,
    progress: Progress,
    from: f64,
    to: f64,
}

const MEDITATIONS: usize = 0;
const WALDEN: usize = 1;
const ELEMENTS: usize = 2;
const ART_OF_WAR: usize = 3;
const FRANKENSTEIN: usize = 4;
const REPUBLIC: usize = 9;
const LEAVES: usize = 10;

const BOOKS: &[Book] = &[
    Book {
        title: "Meditations",
        author: "Marcus Aurelius",
        goal: "health",
        shelf: Some(BookStatus::Active),
        reader: Reader::AppleBooks,
        locator: "6F1D3A0E5B2C4A7F9E8D1C0B2A3F4E5D",
        progress: Progress::Percent,
        from: 0.04,
        to: 0.58,
    },
    Book {
        title: "Walden",
        author: "Henry David Thoreau",
        goal: "learning",
        shelf: Some(BookStatus::Active),
        reader: Reader::Preview,
        locator: "/Users/reader/Books/walden.pdf",
        progress: Progress::Pages(352),
        from: 0.02,
        to: 0.47,
    },
    Book {
        title: "The Elements of Style",
        author: "William Strunk Jr.",
        goal: "communication",
        shelf: Some(BookStatus::Active),
        reader: Reader::Bookokrat,
        locator: "/Users/reader/Books/the-elements-of-style.epub",
        progress: Progress::Chapters(12),
        from: 0.0,
        to: 0.55,
    },
    Book {
        title: "The Art of War",
        author: "Sun Tzu",
        goal: "competence",
        shelf: Some(BookStatus::Active),
        reader: Reader::Bookokrat,
        locator: "/Users/reader/Books/the-art-of-war.pdf",
        progress: Progress::Pages(96),
        from: 0.05,
        to: 0.71,
    },
    Book {
        title: "Frankenstein",
        author: "Mary Shelley",
        goal: "communication",
        shelf: Some(BookStatus::Done),
        reader: Reader::Bookokrat,
        locator: "/Users/reader/Books/frankenstein.epub",
        progress: Progress::Chapters(24),
        from: 0.0,
        to: 1.0,
    },
    Book {
        title: "The Prince",
        author: "Niccolò Machiavelli",
        goal: "competence",
        shelf: Some(BookStatus::Queued),
        reader: Reader::Bookokrat,
        locator: "/Users/reader/Books/the-prince.pdf",
        progress: Progress::Pages(140),
        from: 0.0,
        to: 0.0,
    },
    Book {
        title: "Middlemarch",
        author: "George Eliot",
        goal: "communication",
        shelf: Some(BookStatus::Queued),
        reader: Reader::AppleBooks,
        locator: "A9C4E2B7D1F0483E9B6A5C2D8E7F1A03",
        progress: Progress::Percent,
        from: 0.0,
        to: 0.0,
    },
    Book {
        title: "The Odyssey",
        author: "Homer",
        goal: "learning",
        shelf: Some(BookStatus::Queued),
        reader: Reader::Bookokrat,
        locator: "/Users/reader/Books/the-odyssey.epub",
        progress: Progress::Chapters(24),
        from: 0.0,
        to: 0.0,
    },
    Book {
        title: "The Anatomy of Melancholy",
        author: "Robert Burton",
        goal: "health",
        shelf: Some(BookStatus::Queued),
        reader: Reader::Preview,
        locator: "/Users/reader/Books/the-anatomy-of-melancholy.pdf",
        progress: Progress::Pages(880),
        from: 0.0,
        to: 0.0,
    },
    Book {
        title: "The Republic",
        author: "Plato",
        goal: "",
        shelf: None,
        reader: Reader::Preview,
        locator: "/Users/reader/Books/the-republic.pdf",
        progress: Progress::Pages(416),
        from: 0.10,
        to: 0.12,
    },
    Book {
        title: "Leaves of Grass",
        author: "Walt Whitman",
        goal: "",
        shelf: None,
        reader: Reader::AppleBooks,
        locator: "C3E8A1F6B2D94075A8E1C6B3F9D2E4A7",
        progress: Progress::Percent,
        from: 0.20,
        to: 0.21,
    },
];

const ACTIVE: [usize; 4] = [MEDITATIONS, WALDEN, ELEMENTS, ART_OF_WAR];
const FIRST_DAY: i64 = 72;
const STREAK_DAYS: i64 = 13;
const FRANKENSTEIN_UNTIL: i64 = 45;

struct Planned {
    book: usize,
    started_at: Timestamp,
    minutes: i64,
    open: bool,
}

fn main() -> Result<()> {
    let Some(path) = std::env::args().nth(1).map(PathBuf::from) else {
        bail!("usage: demo_data <path to a new readr.db>");
    };
    if path.exists() {
        bail!(
            "{} already exists; demo data only goes into a new database",
            path.display()
        );
    }
    let db = Db::open(&path)?;
    let tz = TimeZone::system();
    let now = Zoned::now();

    db.save_config(&config()?)?;
    let plan = plan(&now, &tz)?;
    let sources = add_books(&db, now.timestamp())?;
    record(&db, &plan, &sources, &tz)?;

    println!("demo data written to {}", path.display());
    Ok(())
}

fn config() -> Result<Config> {
    let mut config = Config::parse(DEFAULT_CONFIG_TOML)?
        .with_setting("daily_goal_minutes", "30")?
        .with_setting("library_dirs", "~/Books")?;
    for (goal, minutes) in [
        ("health", "45"),
        ("learning", "60"),
        ("communication", "45"),
        ("competence", "60"),
    ] {
        config = config.with_goal_field(goal, "weekly_minutes", minutes)?;
    }
    Ok(config)
}

fn plan(now: &Zoned, tz: &TimeZone) -> Result<Vec<Planned>> {
    let today = now.date();
    let mut random = Lcg(20_261_024);
    let mut plan = Vec::new();
    for days_ago in (1..=FIRST_DAY).rev() {
        let day = today.checked_sub(days_ago.days())?;
        let total = match days_ago {
            d if d <= STREAK_DAYS => 30 + random.below(26),
            d if d == STREAK_DAYS + 1 => 11,
            _ if random.below(4) == 0 => 0,
            _ => 12 + random.below(40),
        };
        let starts = [
            Time::constant(7, 20, 0, 0),
            Time::constant(12, 35, 0, 0),
            Time::constant(21, 5, 0, 0),
        ];
        let count = if total < 20 {
            1
        } else {
            2 + random.below(2) as usize
        };
        for (slot, start) in starts.iter().take(count).enumerate() {
            let minutes = total / count as i64;
            let mut book = ACTIVE[(days_ago as usize * 3 + slot) % ACTIVE.len()];
            if book == ELEMENTS && days_ago > FRANKENSTEIN_UNTIL {
                book = FRANKENSTEIN;
            }
            let jitter = random.below(25).minutes();
            plan.push(Planned {
                book,
                started_at: at(day, *start, tz)?.checked_add(jitter)?,
                minutes,
                open: false,
            });
        }
    }
    for (book, days_ago, minutes) in [(REPUBLIC, 9, 9), (LEAVES, 4, 4)] {
        let day = today.checked_sub(i64::from(days_ago).days())?;
        plan.push(Planned {
            book,
            started_at: at(day, Time::constant(18, 10, 0, 0), tz)?,
            minutes,
            open: false,
        });
    }
    let midnight = at(today, Time::midnight(), tz)?;
    for (book, ago, minutes, open) in [
        (MEDITATIONS, 190, 12, false),
        (ART_OF_WAR, 95, 9, false),
        (WALDEN, 14, 14, true),
    ] {
        let started_at = now
            .timestamp()
            .checked_sub(SignedDuration::from_mins(ago))?;
        plan.push(Planned {
            book,
            started_at: started_at.max(midnight),
            minutes,
            open,
        });
    }
    plan.sort_by_key(|planned| planned.started_at);
    Ok(plan)
}

fn add_books(db: &Db, now: Timestamp) -> Result<Vec<i64>> {
    let mut sources = Vec::new();
    for book in BOOKS {
        let source = db.upsert_source(
            book.reader,
            book.locator,
            Some(book.title),
            Some(book.author),
        )?;
        if let Some(status) = book.shelf {
            let id = db.add_book(
                &NewBook {
                    title: book.title.into(),
                    author: Some(book.author.into()),
                    goal: Some(book.goal.into()),
                    status,
                },
                now,
            )?;
            db.link_source(source.id, Some(id))?;
            if status == BookStatus::Done {
                let finished =
                    now.checked_sub(SignedDuration::from_hours(24 * FRANKENSTEIN_UNTIL))?;
                db.set_book_status(id, BookStatus::Done, finished)?;
            }
        }
        sources.push(source.id);
    }
    Ok(sources)
}

fn record(db: &Db, plan: &[Planned], sources: &[i64], tz: &TimeZone) -> Result<()> {
    let total: Vec<i64> = (0..BOOKS.len())
        .map(|book| {
            plan.iter()
                .filter(|p| p.book == book)
                .map(|p| p.minutes)
                .sum()
        })
        .collect();
    let mut read = vec![0_i64; BOOKS.len()];
    for planned in plan {
        let book = &BOOKS[planned.book];
        let fraction = |minutes: i64| {
            book.from + (book.to - book.from) * minutes as f64 / total[planned.book].max(1) as f64
        };
        let start = position(book.progress, fraction(read[planned.book]));
        read[planned.book] += planned.minutes;
        let end = position(book.progress, fraction(read[planned.book]));
        let active = SignedDuration::from_mins(planned.minutes);
        let source_id = sources[planned.book];
        let ended_at = planned.started_at.checked_add(active)?;
        db.save_session(
            &OpenSession {
                id: None,
                source_id,
                day: planned.started_at.to_zoned(tz.clone()).date(),
                started_at: planned.started_at,
                last_activity_at: ended_at,
                last_kind: ActivityKind::PageTurn,
                active,
                start_position: Some(start),
                end_position: Some(end.clone()),
            },
            planned.open,
        )?;
        db.set_source_position(source_id, &end, ended_at)
            .context("saving the last position")?;
    }
    Ok(())
}

fn position(progress: Progress, fraction: f64) -> Position {
    let fraction = fraction.clamp(0.0, 1.0);
    match progress {
        Progress::Pages(pages) => {
            let page = ((fraction * f64::from(pages)).round() as u32).clamp(1, pages);
            Position::new(
                Some(f64::from(page) / f64::from(pages)),
                Some(format!("p.{page}/{pages}")),
            )
        }
        Progress::Chapters(chapters) => {
            let chapter = ((fraction * f64::from(chapters)).ceil() as u32).clamp(1, chapters);
            Position::new(Some(fraction), Some(format!("ch.{chapter}/{chapters}")))
        }
        Progress::Percent => Position::new(Some(fraction), None),
    }
}

fn at(day: Date, time: Time, tz: &TimeZone) -> Result<Timestamp> {
    Ok(day.to_datetime(time).to_zoned(tz.clone())?.timestamp())
}

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: u64) -> i64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % bound) as i64
    }
}
