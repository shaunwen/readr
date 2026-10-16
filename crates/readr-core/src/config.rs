use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use jiff::SignedDuration;
use jiff::civil::Time;
use serde::Deserialize;

pub const DEFAULT_CONFIG_TOML: &str = include_str!("default_config.toml");

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub daily_goal_minutes: u32,
    pub count_off_shelf: bool,
    pub library_dirs: Vec<PathBuf>,
    pub remind: Remind,
    pub nightly: Nightly,
    pub tracking: Tracking,
    pub launch: Launch,
    pub goals: Vec<Goal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Remind {
    pub interval: SignedDuration,
    pub start: Time,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nightly {
    pub at: Time,
    pub email: Option<Email>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Email {
    pub to: String,
    pub from: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tracking {
    pub poll_seconds: u64,
    pub max_page_minutes: u32,
    pub last_page_minutes: u32,
    pub idle_seconds: u32,
    pub min_session_seconds: u32,
    pub use_window_focus: WindowFocus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowFocus {
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub terminal: Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Terminal {
    Rio,
    Herdr,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub key: String,
    pub label: String,
    pub color: Option<String>,
    pub weekly_minutes: Option<u32>,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub books_collections: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    daily_goal_minutes: Option<u32>,
    count_off_shelf: Option<bool>,
    library_dirs: Option<Vec<String>>,
    #[serde(default)]
    remind: FileRemind,
    #[serde(default)]
    nightly: FileNightly,
    #[serde(default)]
    tracking: FileTracking,
    #[serde(default)]
    launch: FileLaunch,
    goal: Option<Vec<Goal>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRemind {
    interval: Option<SignedDuration>,
    start: Option<Time>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileNightly {
    at: Option<Time>,
    email_to: Option<String>,
    email_from: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_user: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTracking {
    poll_seconds: Option<u64>,
    max_page_minutes: Option<u32>,
    last_page_minutes: Option<u32>,
    idle_seconds: Option<u32>,
    min_session_seconds: Option<u32>,
    use_window_focus: Option<WindowFocus>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLaunch {
    terminal: Option<Terminal>,
}

impl Config {
    pub fn load(path: &Path, home: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, home).with_context(|| format!("in {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::parse(DEFAULT_CONFIG_TOML, home)
            }
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn parse(text: &str, home: &Path) -> Result<Self> {
        let file: FileConfig = toml::from_str(text)?;
        let config = Self::from_file(file, home)?;
        config.validate()?;
        Ok(config)
    }

    pub fn suggest_goal(&self, locator: &str, collections: &[String]) -> Option<&Goal> {
        let path = Path::new(locator);
        let in_folder = |goal: &&Goal| {
            goal.folders.iter().any(|folder| {
                path.components()
                    .any(|part| part.as_os_str().to_string_lossy() == folder.as_str())
            })
        };
        let in_collection = |goal: &&Goal| {
            goal.books_collections
                .iter()
                .any(|name| collections.iter().any(|c| c.eq_ignore_ascii_case(name)))
        };
        self.goals
            .iter()
            .find(in_folder)
            .or_else(|| self.goals.iter().find(in_collection))
    }

    pub fn goal(&self, key: &str) -> Option<&Goal> {
        self.goals.iter().find(|goal| goal.key == key)
    }

    pub fn daily_goal(&self) -> SignedDuration {
        SignedDuration::from_mins(i64::from(self.daily_goal_minutes))
    }

    fn from_file(file: FileConfig, home: &Path) -> Result<Self> {
        let email = match (file.nightly.email_to, file.nightly.smtp_user) {
            (Some(to), Some(smtp_user)) => Some(Email {
                to,
                from: file.nightly.email_from.unwrap_or_else(|| smtp_user.clone()),
                smtp_host: file
                    .nightly
                    .smtp_host
                    .unwrap_or_else(|| "smtp.gmail.com".into()),
                smtp_port: file.nightly.smtp_port.unwrap_or(587),
                smtp_user,
            }),
            (None, None) => None,
            _ => bail!("nightly.email_to and nightly.smtp_user must be set together"),
        };
        let library_dirs = file
            .library_dirs
            .unwrap_or_else(|| vec!["~/Documents/Books".into()])
            .iter()
            .map(|dir| expand_home(dir, home))
            .collect();
        Ok(Self {
            daily_goal_minutes: file.daily_goal_minutes.unwrap_or(20),
            count_off_shelf: file.count_off_shelf.unwrap_or(false),
            library_dirs,
            remind: Remind {
                interval: file
                    .remind
                    .interval
                    .unwrap_or(SignedDuration::from_hours(2)),
                start: file.remind.start.unwrap_or(Time::constant(9, 0, 0, 0)),
            },
            nightly: Nightly {
                at: file.nightly.at.unwrap_or(Time::constant(21, 0, 0, 0)),
                email,
            },
            tracking: Tracking {
                poll_seconds: file.tracking.poll_seconds.unwrap_or(15),
                max_page_minutes: file.tracking.max_page_minutes.unwrap_or(5),
                last_page_minutes: file.tracking.last_page_minutes.unwrap_or(1),
                idle_seconds: file.tracking.idle_seconds.unwrap_or(120),
                min_session_seconds: file.tracking.min_session_seconds.unwrap_or(60),
                use_window_focus: file.tracking.use_window_focus.unwrap_or(WindowFocus::Auto),
            },
            launch: Launch {
                terminal: file.launch.terminal.unwrap_or(Terminal::Rio),
            },
            goals: file.goal.unwrap_or_default(),
        })
    }

    fn validate(&self) -> Result<()> {
        if self.daily_goal_minutes == 0 {
            bail!("daily_goal_minutes must be at least 1");
        }
        if self.remind.interval < SignedDuration::from_mins(1) {
            bail!("remind.interval must be at least 1 minute");
        }
        if self.remind.start >= self.nightly.at {
            bail!(
                "remind.start ({}) must be before nightly.at ({})",
                self.remind.start,
                self.nightly.at
            );
        }
        if self.tracking.poll_seconds == 0 {
            bail!("tracking.poll_seconds must be at least 1");
        }
        if self.tracking.max_page_minutes == 0 {
            bail!("tracking.max_page_minutes must be at least 1");
        }
        let mut keys = HashSet::new();
        for goal in &self.goals {
            if !is_goal_key(&goal.key) {
                bail!(
                    "goal key {:?} must be lowercase letters, digits and dashes",
                    goal.key
                );
            }
            if !keys.insert(goal.key.as_str()) {
                bail!("goal key {:?} is defined twice", goal.key);
            }
        }
        Ok(())
    }
}

fn is_goal_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/reader";

    fn parse(text: &str) -> Result<Config> {
        Config::parse(text, Path::new(HOME))
    }

    #[test]
    fn empty_file_uses_defaults_without_goals() {
        // given
        let text = "";

        // when
        let config = parse(text).unwrap();

        // then
        assert_eq!(config.daily_goal_minutes, 20);
        assert_eq!(config.remind.interval, SignedDuration::from_hours(2));
        assert_eq!(config.remind.start, Time::constant(9, 0, 0, 0));
        assert_eq!(config.nightly.at, Time::constant(21, 0, 0, 0));
        assert_eq!(config.nightly.email, None);
        assert_eq!(
            config.library_dirs,
            vec![PathBuf::from("/Users/reader/Documents/Books")]
        );
        assert!(config.goals.is_empty());
    }

    #[test]
    fn default_template_defines_the_four_goals() {
        // given
        let text = DEFAULT_CONFIG_TOML;

        // when
        let config = parse(text).unwrap();

        // then
        let keys: Vec<&str> = config.goals.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["health", "learning", "communication", "competence"]
        );
        assert_eq!(config.nightly.at, Time::constant(21, 0, 0, 0));
    }

    #[test]
    fn missing_file_falls_back_to_default_template() {
        // given
        let dir = tempfile::tempdir().unwrap();

        // when
        let config = Config::load(&dir.path().join("config.toml"), Path::new(HOME)).unwrap();

        // then
        assert_eq!(config.goals.len(), 4);
    }

    #[test]
    fn reminder_times_are_configurable() {
        // given
        let text = r#"
            [remind]
            interval = "90m"
            start = "08:30"

            [nightly]
            at = "22:15"
        "#;

        // when
        let config = parse(text).unwrap();

        // then
        assert_eq!(config.remind.interval, SignedDuration::from_mins(90));
        assert_eq!(config.remind.start, Time::constant(8, 30, 0, 0));
        assert_eq!(config.nightly.at, Time::constant(22, 15, 0, 0));
    }

    #[test]
    fn email_defaults_to_gmail_smtp_and_sender_is_smtp_user() {
        // given
        let text = r#"
            [nightly]
            email_to = "me@example.com"
            smtp_user = "sender@gmail.com"
        "#;

        // when
        let email = parse(text).unwrap().nightly.email.unwrap();

        // then
        assert_eq!(email.smtp_host, "smtp.gmail.com");
        assert_eq!(email.smtp_port, 587);
        assert_eq!(email.from, "sender@gmail.com");
    }

    #[test]
    fn email_needs_both_recipient_and_smtp_user() {
        // given
        let text = "[nightly]\nemail_to = \"me@example.com\"";

        // when
        let error = parse(text).unwrap_err();

        // then
        assert!(error.to_string().contains("must be set together"));
    }

    #[test]
    fn reminders_must_start_before_nightly_check() {
        // given
        let text = "[remind]\nstart = \"21:30\"\n[nightly]\nat = \"21:00\"";

        // when
        let error = parse(text).unwrap_err();

        // then
        assert!(error.to_string().contains("must be before nightly.at"));
    }

    #[test]
    fn duplicate_goal_keys_are_rejected() {
        // given
        let text = r#"
            [[goal]]
            key = "health"
            label = "Health"

            [[goal]]
            key = "health"
            label = "Body"
        "#;

        // when
        let error = parse(text).unwrap_err();

        // then
        assert!(error.to_string().contains("defined twice"));
    }

    #[test]
    fn goal_keys_must_be_slugs() {
        // given
        let text = "[[goal]]\nkey = \"Health Goal\"\nlabel = \"Health\"";

        // when
        let error = parse(text).unwrap_err();

        // then
        assert!(error.to_string().contains("lowercase letters"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        // given
        let text = "daily_goal = 30";

        // when
        let error = parse(text).unwrap_err();

        // then
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn goal_is_suggested_from_the_book_folder() {
        // given
        let config = parse(DEFAULT_CONFIG_TOML).unwrap();

        // when
        let goal = config.suggest_goal(
            "/Users/reader/Documents/Books/communication/walden.pdf",
            &[],
        );

        // then
        assert_eq!(goal.map(|g| g.key.as_str()), Some("communication"));
    }

    #[test]
    fn goal_is_suggested_from_an_apple_books_collection() {
        // given
        let config = parse(DEFAULT_CONFIG_TOML).unwrap();

        // when
        let goal = config.suggest_goal("4F3A9B2C", &["health".to_string()]);

        // then
        assert_eq!(goal.map(|g| g.key.as_str()), Some("health"));
    }

    #[test]
    fn folder_names_must_match_whole_path_components() {
        // given
        let config = parse(DEFAULT_CONFIG_TOML).unwrap();

        // when
        let goal = config.suggest_goal("/Books/communication-extra/x.pdf", &[]);

        // then
        assert_eq!(goal, None);
    }

    #[test]
    fn library_dirs_expand_home() {
        // given
        let text = r#"library_dirs = ["~/Books", "/Volumes/ebooks"]"#;

        // when
        let config = parse(text).unwrap();

        // then
        assert_eq!(
            config.library_dirs,
            vec![
                PathBuf::from("/Users/reader/Books"),
                PathBuf::from("/Volumes/ebooks")
            ]
        );
    }
}
