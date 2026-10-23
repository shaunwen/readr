use std::time::Duration;

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use readr_core::readers::apple_books::{self, AppleBooks};
use readr_core::readers::bookokrat::{self, Bookokrat};
use readr_core::readers::preview::{self, Preview};
use readr_core::readers::{Desktop, MacDesktop, PollContext, ReaderAdapter};
use readr_core::session::Rules;
use readr_core::tracker::Tracker;

use crate::context::App;
use crate::launchd::{self, Agent};
use crate::remind::Reminders;

pub fn run(mut app: App) -> Result<()> {
    let home = app.paths.home.clone();
    let adapters: Vec<Box<dyn ReaderAdapter>> = vec![
        Box::new(Bookokrat::new(bookokrat::default_root(&home))),
        Box::new(Preview::new(preview::default_view_state(&home))),
        Box::new(AppleBooks::new(apple_books::default_library_dir(&home))),
    ];
    let mut tracker = Tracker::new(
        &app.db,
        Rules::from_tracking(&app.config.tracking),
        app.tz.clone(),
        adapters,
    )?;
    let mut config_error: Option<String> = None;
    let mut last_errors: Vec<String> = Vec::new();
    let mut reminders = Reminders::default();
    let mut cleared_reminders_at: Option<std::time::Instant> = None;
    let mut focus_trusted = None;
    log(&format!(
        "started; database {}",
        app.paths.db_file.display()
    ));

    loop {
        match app.db.config() {
            Ok(Some(config)) if config != app.config => {
                tracker.set_rules(Rules::from_tracking(&config.tracking));
                app.config = config;
                config_error = None;
                log("settings changed");
            }
            Ok(_) => config_error = None,
            Err(error) => {
                let error = format!("{error:#}");
                if config_error.as_ref() != Some(&error) {
                    log(&format!(
                        "settings error, keeping the last good ones: {error}"
                    ));
                }
                config_error = Some(error);
            }
        }

        if app.config.tracking.window_focus {
            let trusted = MacDesktop.accessibility_trusted();
            if focus_trusted != Some(trusted) {
                if trusted {
                    log("window focus tracking on");
                } else {
                    if focus_trusted.is_none() {
                        MacDesktop.request_accessibility();
                    }
                    log(&format!(
                        "window focus tracking off: grant Accessibility to {} in System Settings > Privacy & Security > Accessibility",
                        std::env::current_exe()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| "readr".into())
                    ));
                }
                focus_trusted = Some(trusted);
            }
        }

        let library_dirs = app.config.library_paths(&home);
        let ctx = PollContext {
            now: Timestamp::now(),
            desktop: &MacDesktop,
            away_after: SignedDuration::from_mins(i64::from(app.config.tracking.away_minutes)),
            window_focus: app.config.tracking.window_focus,
            library_dirs: &library_dirs,
        };
        match tracker.poll(&app.db, &ctx) {
            Ok(report) => {
                if report.errors != last_errors {
                    for error in &report.errors {
                        log(error);
                    }
                    last_errors = report.errors;
                }
            }
            Err(error) => log(&format!("poll failed: {error:#}")),
        }
        if let Err(error) = reminders.tick(&app) {
            log(&format!("reminders failed: {error:#}"));
        }
        let settled = reminders
            .last_notified_at()
            .is_none_or(|at| at.elapsed() >= REMINDER_SETTLE);
        if settled && cleared_reminders_at.is_none_or(|at| at.elapsed() >= REMINDER_CLEANUP) {
            crate::notify::clear_older_reminders();
            cleared_reminders_at = Some(std::time::Instant::now());
        }

        std::thread::sleep(Duration::from_secs(app.config.tracking.poll_seconds));
    }
}

pub fn log(message: &str) {
    eprintln!(
        "{} {message}",
        Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ")
    );
}

const REMINDER_CLEANUP: Duration = Duration::from_secs(300);
const REMINDER_SETTLE: Duration = Duration::from_secs(60);

const DAEMON_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

fn agent(app: &App) -> Result<Agent> {
    let program = std::env::current_exe()?.canonicalize()?;
    let mut env = vec![("PATH".to_string(), DAEMON_PATH.to_string())];
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        if let Ok(value) = std::env::var(key) {
            env.push((key.to_string(), value));
        }
    }
    Ok(Agent::new(&app.paths.home, program, env))
}

pub fn install(app: &App) -> Result<()> {
    let agent = agent(app)?;
    if agent
        .program
        .components()
        .any(|c| c.as_os_str() == "target")
    {
        eprintln!(
            "warning: installing a build from {}; run `make install` and install from there instead",
            agent.program.display()
        );
    }
    agent.install()?;
    println!("installed {}", agent.plist.display());
    println!("program   {}", agent.program.display());
    println!("log       {}", agent.log.display());
    Ok(())
}

pub fn uninstall(app: &App) -> Result<()> {
    let agent = agent(app)?;
    agent.uninstall()?;
    println!("removed {}", agent.plist.display());
    Ok(())
}

pub fn status(app: &App) {
    let log = app.paths.home.join("Library/Logs/readr/daemon.log");
    match launchd::state() {
        launchd::State::Running { pid } => println!("running (pid {pid}), log {}", log.display()),
        launchd::State::Loaded { state } => println!("loaded but {state}, log {}", log.display()),
        launchd::State::NotLoaded => println!("not installed; run `readr daemon install`"),
    }
}
