use std::collections::BTreeMap;

use anyhow::Result;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span};
use readr_core::config::Config;
use readr_core::db::{SessionView, Source};
use readr_core::model::{BookStatus, Reader};
use readr_core::stats::{best_streak, streak};

use crate::context::App;
use crate::next::next_up;
use crate::shelf::{self, ShelfRow};

pub const HEATMAP_WEEKS: i64 = 16;

pub struct NextView {
    pub book_id: i64,
    pub text: String,
    pub reason: String,
}

pub struct GoalWeek {
    pub key: String,
    pub label: String,
    pub color: Option<String>,
    pub read: SignedDuration,
    pub target: Option<u32>,
}

pub struct Data {
    pub tz: TimeZone,
    pub today: Date,
    pub total: SignedDuration,
    pub goal: SignedDuration,
    pub streak: u32,
    pub best_streak: u32,
    pub sessions: Vec<SessionView>,
    pub next: Option<NextView>,
    pub shelf: Vec<ShelfRow>,
    pub candidates: Vec<Source>,
    pub inbox: Vec<(Source, SignedDuration)>,
    pub heat: BTreeMap<Date, SignedDuration>,
    pub week_goals: Vec<GoalWeek>,
    pub goals: Vec<(String, String)>,
    pub config: Config,
}

impl Data {
    pub fn load(app: &App) -> Result<Self> {
        let today = app.today();
        let goal = app.config.daily_goal();
        let year_ago = today.saturating_sub(Span::new().days(400));
        let heat = app
            .db
            .daily_totals(year_ago, today, app.config.count_off_shelf)?;

        let mut rows = shelf::rows(app)?;
        rows.retain(|row| row.book.status != BookStatus::Dropped);

        let source_totals = app.db.source_totals()?;
        let inbox = app
            .db
            .inbox_sources()?
            .into_iter()
            .map(|source| {
                let time = source_totals.get(&source.id).copied().unwrap_or_default();
                (source, time)
            })
            .collect();
        let candidates = app
            .db
            .all_sources()?
            .into_iter()
            .filter(|s| {
                s.book_id.is_none() && !s.ignored && !s.untracked && s.reader != Reader::Manual
            })
            .collect();

        let monday = today
            .saturating_sub(Span::new().days(i64::from(today.weekday().to_monday_zero_offset())));
        let week = app.db.goal_totals(monday, today)?;
        let week_goals = app
            .config
            .goals
            .iter()
            .map(|g| GoalWeek {
                key: g.key.clone(),
                label: g.label.clone(),
                color: g.color.clone(),
                read: week.get(&g.key).copied().unwrap_or_default(),
                target: g.weekly_minutes,
            })
            .collect();

        Ok(Self {
            tz: app.tz.clone(),
            today,
            total: heat.get(&today).copied().unwrap_or_default(),
            goal,
            streak: streak(&heat, today, goal),
            best_streak: best_streak(&heat, goal),
            sessions: app.db.sessions_between(today, today)?,
            next: next_up(app)?.map(|next| NextView {
                book_id: next.book.id,
                text: next.describe(),
                reason: next.reason,
            }),
            shelf: rows,
            candidates,
            inbox,
            heat,
            week_goals,
            goals: app
                .config
                .goals
                .iter()
                .map(|g| (g.key.clone(), g.label.clone()))
                .collect(),
            config: app.config.clone(),
        })
    }

    pub fn goal_color(&self, key: Option<&str>) -> Option<&str> {
        let key = key?;
        self.week_goals
            .iter()
            .find(|g| g.key == key)
            .and_then(|g| g.color.as_deref())
    }
}
