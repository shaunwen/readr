use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use jiff::{SignedDuration, Timestamp};
use readr_core::config::Config;
use readr_core::readers::apple_books::{self, AppleBooks};
use readr_core::readers::bookokrat::{self, Bookokrat};
use readr_core::readers::preview::{self, Preview};
use readr_core::readers::{MacDesktop, PollContext, ReaderAdapter};
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
    let mut config_mtime = mtime(&app.paths.config_file);
    let mut last_errors: Vec<String> = Vec::new();
    let mut reminders = Reminders::default();
    log(&format!(
        "started; config {}, database {}",
        app.paths.config_file.display(),
        app.paths.db_file.display()
    ));

    loop {
        let current_mtime = mtime(&app.paths.config_file);
        if current_mtime != config_mtime {
            config_mtime = current_mtime;
            match Config::load(&app.paths.config_file, &app.paths.home) {
                Ok(config) => {
                    tracker.set_rules(Rules::from_tracking(&config.tracking));
                    app.config = config;
                    log("config reloaded");
                }
                Err(error) => log(&format!(
                    "config error, keeping last good config: {error:#}"
                )),
            }
        }

        let ctx = PollContext {
            now: Timestamp::now(),
            desktop: &MacDesktop,
            idle_limit: SignedDuration::from_secs(i64::from(app.config.tracking.idle_seconds)),
            library_dirs: &app.config.library_dirs,
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

        std::thread::sleep(Duration::from_secs(app.config.tracking.poll_seconds));
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn log(message: &str) {
    eprintln!(
        "{} {message}",
        Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ")
    );
}

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
