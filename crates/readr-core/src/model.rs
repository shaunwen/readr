use std::fmt;
use std::str::FromStr;

use anyhow::{Error, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reader {
    Bookokrat,
    Preview,
    AppleBooks,
    Manual,
}

impl Reader {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bookokrat => "bookokrat",
            Self::Preview => "preview",
            Self::AppleBooks => "apple_books",
            Self::Manual => "manual",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Bookokrat => "bookokrat",
            Self::Preview => "Preview",
            Self::AppleBooks => "Books",
            Self::Manual => "manual",
        }
    }
}

impl fmt::Display for Reader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Reader {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "bookokrat" => Self::Bookokrat,
            "preview" => Self::Preview,
            "apple_books" => Self::AppleBooks,
            "manual" => Self::Manual,
            other => bail!("unknown reader {other:?}"),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookStatus {
    Active,
    Queued,
    Done,
    Dropped,
}

impl BookStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Queued => "queued",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }
}

impl fmt::Display for BookStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BookStatus {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "active" => Self::Active,
            "queued" => Self::Queued,
            "done" => Self::Done,
            "dropped" => Self::Dropped,
            other => bail!("unknown book status {other:?}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Position {
    pub progress: Option<f64>,
    pub label: Option<String>,
}

impl Position {
    pub fn new(progress: Option<f64>, label: Option<String>) -> Self {
        Self {
            progress: progress.map(|p| p.clamp(0.0, 1.0)),
            label,
        }
    }

    pub fn display(&self) -> String {
        match (&self.label, self.progress) {
            (Some(label), _) => label.clone(),
            (None, Some(progress)) => format_progress(progress),
            (None, None) => "-".into(),
        }
    }
}

pub fn format_progress(progress: f64) -> String {
    let percent = progress * 100.0;
    if percent < 10.0 && percent.fract() != 0.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.0}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_round_trips_through_its_name() {
        // given
        let readers = [
            Reader::Bookokrat,
            Reader::Preview,
            Reader::AppleBooks,
            Reader::Manual,
        ];

        // when
        let parsed: Vec<Reader> = readers
            .iter()
            .map(|r| r.as_str().parse().unwrap())
            .collect();

        // then
        assert_eq!(parsed, readers);
    }

    #[test]
    fn position_prefers_label_over_progress() {
        // given
        let position = Position::new(Some(0.25), Some("p.42/500".into()));

        // when
        let shown = position.display();

        // then
        assert_eq!(shown, "p.42/500");
    }

    #[test]
    fn small_progress_keeps_one_decimal() {
        // given
        let progress = 0.004;

        // when
        let shown = format_progress(progress);

        // then
        assert_eq!(shown, "0.4%");
    }

    #[test]
    fn progress_is_clamped_to_unit_range() {
        // given
        let raw = 1.7;

        // when
        let position = Position::new(Some(raw), None);

        // then
        assert_eq!(position.progress, Some(1.0));
    }
}
