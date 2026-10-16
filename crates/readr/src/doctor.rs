use std::fmt::Write;
use std::process::Command;

use anyhow::Result;
use readr_core::config::DEFAULT_CONFIG_TOML;
use readr_core::readers::desktop::Desktop;
use readr_core::readers::{MacDesktop, apple_books, bookokrat, preview};

use crate::context::App;
use crate::email;
use crate::launchd;

const FULL_DISK_ACCESS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

struct Report {
    out: String,
    problems: usize,
}

impl Report {
    fn ok(&mut self, label: &str, detail: impl AsRef<str>) {
        let _ = writeln!(self.out, "  ok    {label:<14} {}", detail.as_ref());
    }

    fn warn(&mut self, label: &str, detail: impl AsRef<str>) {
        self.problems += 1;
        let _ = writeln!(self.out, "  !!    {label:<14} {}", detail.as_ref());
    }

    fn info(&mut self, label: &str, detail: impl AsRef<str>) {
        let _ = writeln!(self.out, "  --    {label:<14} {}", detail.as_ref());
    }
}

pub fn doctor(app: &App) -> Result<String> {
    let mut report = Report {
        out: String::new(),
        problems: 0,
    };
    let home = &app.paths.home;

    if app.paths.config_file.exists() {
        report.ok(
            "config",
            format!(
                "{} · {} goals · nightly check {}",
                app.paths.config_file.display(),
                app.config.goals.len(),
                app.config.nightly.at.strftime("%H:%M")
            ),
        );
    } else {
        report.warn("config", "no config file yet; run `readr init`");
    }
    let books = app.db.books()?.len();
    let sources = app.db.all_sources()?.len();
    report.ok(
        "database",
        format!(
            "{} · {books} books on the shelf · {sources} seen in readers",
            app.paths.db_file.display()
        ),
    );

    match launchd::state() {
        launchd::State::Running { pid } => report.ok("daemon", format!("running (pid {pid})")),
        launchd::State::Loaded { state } => report.warn("daemon", format!("loaded but {state}")),
        launchd::State::NotLoaded => {
            report.warn("daemon", "not installed; run `readr daemon install`")
        }
    }
    let log = home.join("Library/Logs/readr/daemon.log");
    if let Ok(text) = std::fs::read_to_string(&log) {
        let denied: Vec<&str> = text
            .lines()
            .rev()
            .take(200)
            .filter(|line| {
                line.contains("authorization denied") || line.contains("Operation not permitted")
            })
            .collect();
        if let Some(line) = denied.first() {
            report.warn("daemon access", line);
            report.info(
                "",
                format!(
                    "grant Full Disk Access to {} (open \"{FULL_DISK_ACCESS}\")",
                    std::env::current_exe()?.display()
                ),
            );
        }
    }

    match bookokrat::check(&bookokrat::default_root(home)) {
        Ok(count) => report.ok("bookokrat", format!("{count} books in its libraries")),
        Err(error) => report.warn("bookokrat", format!("{error:#}")),
    }
    match preview::check(&preview::default_view_state(home)) {
        Ok(count) => report.ok("Preview", format!("{count} documents with a saved page")),
        Err(error) => report.warn("Preview", format!("{error:#}")),
    }
    match apple_books::check(&apple_books::default_library_dir(home)) {
        Ok(count) => report.ok("Apple Books", format!("{count} books in the library")),
        Err(error) => report.warn(
            "Apple Books",
            format!("{error:#} (needs Full Disk Access: open \"{FULL_DISK_ACCESS}\")"),
        ),
    }
    let dirs: Vec<String> = app
        .config
        .library_dirs
        .iter()
        .map(|dir| {
            let mark = if dir.is_dir() { "" } else { " (missing)" };
            format!("{}{mark}", dir.display())
        })
        .collect();
    report.info("library dirs", dirs.join(", "));

    let desktop = MacDesktop;
    match (desktop.frontmost_bundle_id(), desktop.idle()) {
        (Some(front), Some(idle)) => report.ok(
            "activity",
            format!("front app {front}, idle {}s", idle.as_secs()),
        ),
        _ => report.warn("activity", "cannot read the front app or idle time"),
    }
    report.info(
        "Preview",
        "macOS asks once to let readr control Preview (to read the open document)",
    );

    let herdr = Command::new("herdr")
        .args(["status", "server"])
        .output()
        .is_ok_and(|output| output.status.success());
    if herdr {
        report.ok("herdr", "reminders show as herdr notifications");
    } else {
        report.info(
            "herdr",
            "not reachable; reminders fall back to macOS notifications",
        );
    }

    match &app.config.nightly.email {
        Some(settings) if email::has_password(&settings.smtp_user) => report.ok(
            "email",
            format!(
                "{} via {}:{}",
                settings.to, settings.smtp_host, settings.smtp_port
            ),
        ),
        Some(settings) => report.warn(
            "email",
            format!(
                "no Keychain password for {}; run `readr email set-password`",
                settings.smtp_user
            ),
        ),
        None => report.info(
            "email",
            "off; set nightly.email_to and nightly.smtp_user in config.toml",
        ),
    }

    let summary = match report.problems {
        0 => "everything looks good".to_string(),
        1 => "1 thing to fix".to_string(),
        n => format!("{n} things to fix"),
    };
    Ok(format!(
        "{}\n{summary}\nchecks run with this terminal's permissions; the daemon log is {}\n",
        report.out,
        log.display()
    ))
}

pub fn init(app: &App) -> Result<String> {
    let mut out = String::new();
    if app.paths.config_file.exists() {
        let _ = writeln!(out, "config exists: {}", app.paths.config_file.display());
    } else {
        if let Some(dir) = app.paths.config_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&app.paths.config_file, DEFAULT_CONFIG_TOML)?;
        let _ = writeln!(out, "wrote {}", app.paths.config_file.display());
    }
    let _ = writeln!(out, "database: {}", app.paths.db_file.display());
    let _ = writeln!(
        out,
        "\nnext:\n  1. edit goals, reminder times and email in the config\n  2. readr email set-password      (if you set nightly.email_to)\n  3. readr daemon install\n  4. grant Full Disk Access to {} (open \"{FULL_DISK_ACCESS}\")\n  5. readr shelf add <title> --goal <goal>\n  6. readr doctor",
        std::env::current_exe()?.display()
    );
    Ok(out)
}
