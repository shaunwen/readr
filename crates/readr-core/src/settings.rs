use anyhow::{Context, Result, anyhow, bail};
use jiff::SignedDuration;
use jiff::civil::Time;
use serde::Deserialize;
use serde::de::IntoDeserializer;

use crate::config::{Config, Goal, NotifyChannel, Terminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Bool,
    Text,
    Time,
    Duration,
    List,
    Choice(&'static [&'static str]),
}

impl Kind {
    pub fn choices(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Bool => Some(&["true", "false"]),
            Self::Choice(choices) => Some(choices),
            _ => None,
        }
    }

    fn toml(self, value: &str) -> Option<toml::Value> {
        if value.is_empty() {
            return None;
        }
        Some(match self {
            Self::Number => toml::Value::Integer(value.parse().ok()?),
            Self::Bool => toml::Value::Boolean(value == "true"),
            Self::List => toml::Value::Array(list(value).into_iter().map(Into::into).collect()),
            Self::Text | Self::Time | Self::Duration | Self::Choice(_) => value.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub key: &'static str,
    pub section: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

const fn field(
    key: &'static str,
    section: &'static str,
    label: &'static str,
    kind: Kind,
    help: &'static str,
) -> Field {
    Field {
        key,
        section,
        label,
        help,
        kind,
    }
}

pub const SETTINGS: &[Field] = &[
    field(
        "daily_goal_minutes",
        "Reading",
        "Daily goal (min)",
        Kind::Number,
        "Minutes to read each day",
    ),
    field(
        "count_off_shelf",
        "Reading",
        "Count off-shelf reading",
        Kind::Bool,
        "Reading in books that are not on the shelf counts toward the daily goal",
    ),
    field(
        "library_dirs",
        "Reading",
        "Library folders",
        Kind::List,
        "Folders whose PDFs are tracked in Preview, comma separated",
    ),
    field(
        "remind.interval",
        "Reminders",
        "Remind every",
        Kind::Duration,
        "Time between reminders while the goal is not met, e.g. 2h or 90m",
    ),
    field(
        "remind.start",
        "Reminders",
        "First reminder at",
        Kind::Time,
        "No reminders before this time (HH:MM)",
    ),
    field(
        "nightly.at",
        "Reminders",
        "Nightly check at",
        Kind::Time,
        "When the day is checked and the email goes out if the goal was missed (HH:MM)",
    ),
    field(
        "notify.channel",
        "Reminders",
        "Notify via",
        Kind::Choice(&["system", "herdr"]),
        "herdr: herdr -> terminal-notifier -> macOS · system: terminal-notifier -> macOS",
    ),
    field(
        "launch.terminal",
        "Reminders",
        "Open bookokrat in",
        Kind::Choice(&["rio", "herdr"]),
        "Where `readr go` opens bookokrat books",
    ),
    field(
        "nightly.email_to",
        "Email",
        "Send to",
        Kind::Text,
        "Nightly email recipient; email is on once this and the SMTP user are set",
    ),
    field(
        "nightly.smtp_user",
        "Email",
        "SMTP user",
        Kind::Text,
        "Login for the SMTP server; the password lives in the Keychain (readr email set-password)",
    ),
    field(
        "nightly.email_from",
        "Email",
        "From",
        Kind::Text,
        "Sender address; empty sends from the SMTP user",
    ),
    field(
        "nightly.smtp_host",
        "Email",
        "SMTP host",
        Kind::Text,
        "SMTP server, using STARTTLS",
    ),
    field(
        "nightly.smtp_port",
        "Email",
        "SMTP port",
        Kind::Number,
        "SMTP port, usually 587",
    ),
    field(
        "tracking.poll_seconds",
        "Tracking",
        "Check readers every (s)",
        Kind::Number,
        "How often the daemon looks at the readers",
    ),
    field(
        "tracking.max_page_minutes",
        "Tracking",
        "Longest page (min)",
        Kind::Number,
        "Without window focus, a gap between page turns counts up to this long",
    ),
    field(
        "tracking.last_page_minutes",
        "Tracking",
        "Last page (min)",
        Kind::Number,
        "Time credited for the page you stopped on",
    ),
    field(
        "tracking.away_minutes",
        "Tracking",
        "Away after (min)",
        Kind::Number,
        "Without keyboard or mouse input this long, the session is cut back",
    ),
    field(
        "tracking.min_session_seconds",
        "Tracking",
        "Shortest session (s)",
        Kind::Number,
        "Shorter sessions are dropped",
    ),
    field(
        "tracking.window_focus",
        "Tracking",
        "Window focus",
        Kind::Bool,
        "Count time while a reader window is focused, even between page turns (needs Accessibility)",
    ),
];

const COLORS: &[&str] = &[
    "",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "lightred",
    "lightgreen",
    "lightyellow",
    "lightblue",
    "lightmagenta",
    "lightcyan",
];

pub const GOAL_FIELDS: &[Field] = &[
    field(
        "label",
        "Goal",
        "Name",
        Kind::Text,
        "Shown on the shelf and in stats",
    ),
    field(
        "key",
        "Goal",
        "Key",
        Kind::Text,
        "Stored on books and sessions; changing it moves them along",
    ),
    field(
        "weekly_minutes",
        "Goal",
        "Weekly minutes",
        Kind::Number,
        "Target per week; the goal furthest behind is suggested next. Empty shares the daily goal",
    ),
    field(
        "color",
        "Goal",
        "Color",
        Kind::Choice(COLORS),
        "Color on the shelf and in stats",
    ),
    field(
        "folders",
        "Goal",
        "Folders",
        Kind::List,
        "Books under these folder names are suggested for the goal, comma separated",
    ),
    field(
        "books_collections",
        "Goal",
        "Books collections",
        Kind::List,
        "Apple Books collections whose books are suggested for the goal, comma separated",
    ),
];

pub fn setting(key: &str) -> Option<&'static Field> {
    SETTINGS.iter().find(|field| field.key == key)
}

impl Config {
    pub fn get(&self, key: &str) -> Option<String> {
        Some(match key {
            "daily_goal_minutes" => self.daily_goal_minutes.to_string(),
            "count_off_shelf" => self.count_off_shelf.to_string(),
            "library_dirs" => self.library_dirs.join(", "),
            "remind.interval" => format!("{:#}", self.remind.interval),
            "remind.start" => clock(self.remind.start),
            "nightly.at" => clock(self.nightly.at),
            "notify.channel" => self.notify.channel.as_str().into(),
            "launch.terminal" => self.launch.terminal.as_str().into(),
            "nightly.email_to" => self.nightly.email_to.clone().unwrap_or_default(),
            "nightly.smtp_user" => self.nightly.smtp_user.clone().unwrap_or_default(),
            "nightly.email_from" => self.nightly.email_from.clone().unwrap_or_default(),
            "nightly.smtp_host" => self.nightly.smtp_host.clone(),
            "nightly.smtp_port" => self.nightly.smtp_port.to_string(),
            "tracking.poll_seconds" => self.tracking.poll_seconds.to_string(),
            "tracking.max_page_minutes" => self.tracking.max_page_minutes.to_string(),
            "tracking.last_page_minutes" => self.tracking.last_page_minutes.to_string(),
            "tracking.away_minutes" => self.tracking.away_minutes.to_string(),
            "tracking.min_session_seconds" => self.tracking.min_session_seconds.to_string(),
            "tracking.window_focus" => self.tracking.window_focus.to_string(),
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let value = value.trim();
        match key {
            "daily_goal_minutes" => self.daily_goal_minutes = number(key, value)?,
            "count_off_shelf" => self.count_off_shelf = boolean(key, value)?,
            "library_dirs" => self.library_dirs = list(value),
            "remind.interval" => self.remind.interval = duration(key, value)?,
            "remind.start" => self.remind.start = time(key, value)?,
            "nightly.at" => self.nightly.at = time(key, value)?,
            "notify.channel" => self.notify.channel = choice(key, value)?,
            "launch.terminal" => self.launch.terminal = choice(key, value)?,
            "nightly.email_to" => self.nightly.email_to = optional(value),
            "nightly.smtp_user" => self.nightly.smtp_user = optional(value),
            "nightly.email_from" => self.nightly.email_from = optional(value),
            "nightly.smtp_host" => self.nightly.smtp_host = required(key, value)?,
            "nightly.smtp_port" => self.nightly.smtp_port = number(key, value)?,
            "tracking.poll_seconds" => self.tracking.poll_seconds = number(key, value)?,
            "tracking.max_page_minutes" => self.tracking.max_page_minutes = number(key, value)?,
            "tracking.last_page_minutes" => self.tracking.last_page_minutes = number(key, value)?,
            "tracking.away_minutes" => self.tracking.away_minutes = number(key, value)?,
            "tracking.min_session_seconds" => {
                self.tracking.min_session_seconds = number(key, value)?
            }
            "tracking.window_focus" => self.tracking.window_focus = boolean(key, value)?,
            _ => bail!("unknown setting {key:?}"),
        }
        Ok(())
    }

    pub fn with_setting(&self, key: &str, value: &str) -> Result<Self> {
        let mut config = self.clone();
        config.set(key, value)?;
        config.validate()?;
        Ok(config)
    }

    pub fn with_goal_field(&self, goal: &str, field: &str, value: &str) -> Result<Self> {
        let mut config = self.clone();
        config
            .goals
            .iter_mut()
            .find(|g| g.key == goal)
            .ok_or_else(|| anyhow!("no goal {goal:?}"))?
            .set(field, value)?;
        config.validate()?;
        Ok(config)
    }

    pub fn with_new_goal(&self, label: &str) -> Result<(Self, String)> {
        let base = slug(label);
        if base.is_empty() {
            bail!("give the goal a name with letters or digits");
        }
        let key = (1..)
            .map(|n| match n {
                1 => base.clone(),
                n => format!("{base}-{n}"),
            })
            .find(|key| self.goal(key).is_none())
            .unwrap_or(base);
        let mut config = self.clone();
        config.goals.push(Goal {
            key: key.clone(),
            label: label.trim().into(),
            color: None,
            weekly_minutes: None,
            folders: Vec::new(),
            books_collections: Vec::new(),
        });
        config.validate()?;
        Ok((config, key))
    }

    pub fn without_goal(&self, key: &str) -> Self {
        let mut config = self.clone();
        config.goals.retain(|goal| goal.key != key);
        config
    }

    pub fn with_goal_moved(&self, key: &str, delta: isize) -> Self {
        let mut config = self.clone();
        if let Some(from) = config.goals.iter().position(|goal| goal.key == key) {
            let to = (from as isize + delta).clamp(0, config.goals.len() as isize - 1) as usize;
            let goal = config.goals.remove(from);
            config.goals.insert(to, goal);
        }
        config
    }

    pub fn to_toml(&self) -> String {
        let mut root = toml::Table::new();
        for field in SETTINGS {
            let Some(value) = self.get(field.key).and_then(|v| field.kind.toml(&v)) else {
                continue;
            };
            match field.key.split_once('.') {
                Some((section, name)) => {
                    if let toml::Value::Table(table) = root
                        .entry(section)
                        .or_insert_with(|| toml::Table::new().into())
                    {
                        table.insert(name.into(), value);
                    }
                }
                None => {
                    root.insert(field.key.into(), value);
                }
            }
        }
        let goals: Vec<toml::Value> = self
            .goals
            .iter()
            .map(|goal| {
                let mut table = toml::Table::new();
                for field in GOAL_FIELDS {
                    if let Some(value) = field.kind.toml(&goal.get(field.key)) {
                        table.insert(field.key.into(), value);
                    }
                }
                for list in ["folders", "books_collections"] {
                    table
                        .entry(list)
                        .or_insert_with(|| toml::Value::Array(Vec::new()));
                }
                table.into()
            })
            .collect();
        if !goals.is_empty() {
            root.insert("goal".into(), toml::Value::Array(goals));
        }
        root.to_string()
    }
}

impl Goal {
    pub fn get(&self, field: &str) -> String {
        match field {
            "key" => self.key.clone(),
            "label" => self.label.clone(),
            "weekly_minutes" => self
                .weekly_minutes
                .map(|m| m.to_string())
                .unwrap_or_default(),
            "color" => self.color.clone().unwrap_or_default(),
            "folders" => self.folders.join(", "),
            "books_collections" => self.books_collections.join(", "),
            _ => String::new(),
        }
    }

    pub fn set(&mut self, field: &str, value: &str) -> Result<()> {
        let value = value.trim();
        match field {
            "key" => self.key = value.into(),
            "label" => self.label = required(field, value)?,
            "weekly_minutes" => {
                self.weekly_minutes = match value {
                    "" => None,
                    value => Some(number(field, value)?),
                }
            }
            "color" if COLORS.contains(&value) => self.color = optional(value),
            "color" => bail!("color must be one of {}", COLORS[1..].join(", ")),
            "folders" => self.folders = list(value),
            "books_collections" => self.books_collections = list(value),
            _ => bail!("unknown goal field {field:?}"),
        }
        Ok(())
    }
}

pub fn slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.trim().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn clock(time: Time) -> String {
    time.strftime("%H:%M").to_string()
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

fn optional(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn required(key: &str, value: &str) -> Result<String> {
    if value.is_empty() {
        bail!("{key} can't be empty");
    }
    Ok(value.into())
}

fn number<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .parse()
        .map_err(|_| anyhow!("{key} must be a whole number, not {value:?}"))
}

fn boolean(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" | "on" | "yes" => Ok(true),
        "false" | "off" | "no" => Ok(false),
        _ => bail!("{key} must be true or false, not {value:?}"),
    }
}

fn time(key: &str, value: &str) -> Result<Time> {
    value
        .parse()
        .with_context(|| format!("{key} must be a time like 21:00, not {value:?}"))
}

fn duration(key: &str, value: &str) -> Result<SignedDuration> {
    value
        .parse()
        .with_context(|| format!("{key} must be a duration like 2h or 90m, not {value:?}"))
}

fn choice<T: for<'de> Deserialize<'de>>(key: &str, value: &str) -> Result<T> {
    T::deserialize(IntoDeserializer::<serde::de::value::Error>::into_deserializer(value))
        .map_err(|error| anyhow!("{key}: {error}"))
}

impl NotifyChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Herdr => "herdr",
        }
    }
}

impl Terminal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rio => "rio",
            Self::Herdr => "herdr",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_CONFIG_TOML;

    fn defaults() -> Config {
        Config::parse(DEFAULT_CONFIG_TOML).unwrap()
    }

    #[test]
    fn every_setting_reads_back_what_was_set() {
        // given
        let config = defaults();

        // when
        let mut changed = config.clone();
        for field in SETTINGS {
            let value = config.get(field.key).unwrap();
            changed.set(field.key, &value).unwrap();
        }

        // then
        assert_eq!(changed, config);
    }

    #[test]
    fn settings_show_in_the_format_they_are_typed() {
        // given
        let config = defaults();

        // when
        let shown: Vec<String> = ["remind.interval", "nightly.at", "library_dirs"]
            .iter()
            .map(|key| config.get(key).unwrap())
            .collect();

        // then
        assert_eq!(shown, ["2h", "21:00", "~/Documents/Books"]);
    }

    #[test]
    fn invalid_values_explain_what_is_expected() {
        // given
        let config = defaults();

        // when
        let errors: Vec<String> = [
            ("daily_goal_minutes", "half an hour"),
            ("nightly.at", "9pm"),
            ("notify.channel", "slack"),
            ("daily_goal_minutes", "0"),
        ]
        .iter()
        .map(|(key, value)| format!("{:#}", config.with_setting(key, value).unwrap_err()))
        .collect();

        // then
        assert!(errors[0].contains("whole number"));
        assert!(errors[1].contains("time like 21:00"));
        assert!(errors[2].contains("unknown variant"));
        assert!(errors[3].contains("at least 1"));
    }

    #[test]
    fn empty_optional_settings_are_unset() {
        // given
        let config = defaults()
            .with_setting("nightly.email_to", "me@example.com")
            .unwrap();

        // when
        let cleared = config.with_setting("nightly.email_to", " ").unwrap();

        // then
        assert_eq!(cleared.nightly.email_to, None);
    }

    #[test]
    fn exported_toml_parses_back_to_the_same_config() {
        // given
        let config = defaults()
            .with_setting("nightly.email_to", "me@example.com")
            .unwrap()
            .with_setting("nightly.smtp_user", "sender@example.com")
            .unwrap()
            .with_goal_field("health", "weekly_minutes", "45")
            .unwrap();

        // when
        let parsed = Config::parse(&config.to_toml()).unwrap();

        // then
        assert_eq!(parsed, config);
    }

    #[test]
    fn new_goals_get_a_unique_key_from_their_name() {
        // given
        let config = defaults();

        // when
        let (config, first) = config.with_new_goal("Money & freedom").unwrap();
        let (_, second) = config.with_new_goal("Money, freedom").unwrap();

        // then
        assert_eq!(first, "money-freedom");
        assert_eq!(second, "money-freedom-2");
    }

    #[test]
    fn goal_keys_stay_slugs() {
        // given
        let config = defaults();

        // when
        let error = config
            .with_goal_field("health", "key", "Body Mind")
            .unwrap_err();

        // then
        assert!(error.to_string().contains("lowercase letters"));
    }

    #[test]
    fn goal_colors_come_from_the_terminal_palette() {
        // given
        let config = defaults();

        // when
        let light = config.with_goal_field("health", "color", "lightred");
        let unknown = config.with_goal_field("health", "color", "teal");

        // then
        assert!(light.is_ok());
        assert!(unknown.unwrap_err().to_string().contains("lightred"));
    }

    #[test]
    fn goals_move_within_bounds() {
        // given
        let config = defaults();

        // when
        let moved = config
            .with_goal_moved("communication", -1)
            .with_goal_moved("competence", 5);

        // then
        let keys: Vec<&str> = moved.goals.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["health", "communication", "learning", "competence"]);
    }

    #[test]
    fn empty_weekly_minutes_shares_the_daily_goal() {
        // given
        let config = defaults()
            .with_goal_field("health", "weekly_minutes", "60")
            .unwrap();

        // when
        let cleared = config
            .with_goal_field("health", "weekly_minutes", "")
            .unwrap();

        // then
        assert_eq!(cleared.goal("health").unwrap().weekly_minutes, None);
    }
}
