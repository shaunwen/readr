mod commands;
mod context;
mod daemon;
mod doctor;
mod email;
mod launch;
mod launchd;
mod next;
mod notify;
mod remind;
mod shelf;
mod status;

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use readr_core::model::BookStatus;
use readr_core::stats::{minutes, remaining};

use crate::context::App;

#[derive(Parser)]
#[command(
    name = "readr",
    version,
    about = "Track daily reading across bookokrat, Preview and Apple Books"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Today's reading time, streak and sessions
    Status {
        /// One line for a shell prompt, e.g. 12/20m
        #[arg(long)]
        short: bool,
    },
    /// Open the suggested book, or the one named, in its reader
    Go { book: Option<String> },
    /// Books you are reading, grouped by goal
    Shelf {
        #[command(subcommand)]
        action: Option<ShelfAction>,
    },
    /// Log reading done away from the Mac, e.g. `readr log 25m "Atomic Habits"`
    Log {
        duration: String,
        book: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Reading in books that are not on the shelf
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
    },
    /// The last 7 days, streaks and minutes per goal
    Stats,
    /// Goal maintenance
    Goal {
        #[command(subcommand)]
        action: GoalAction,
    },
    /// Background tracker
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Nightly email settings
    Email {
        #[command(subcommand)]
        action: EmailAction,
    },
    /// Show a sample reminder in herdr (or as a macOS notification)
    NotifyTest,
    /// Write the default config and show the setup steps
    Init,
    /// Check config, readers, permissions, daemon and email
    Doctor,
}

#[derive(Subcommand)]
enum ShelfAction {
    /// Put a book on the shelf by title, author or file path
    Add {
        query: String,
        /// Goal key from config.toml, or "none"
        #[arg(long)]
        goal: Option<String>,
        /// Add to the queue instead of the active books
        #[arg(long)]
        queue: bool,
    },
    /// Mark a book finished and promote the next queued one
    Finish { book: String },
    /// Stop reading a book and promote the next queued one
    Drop { book: String },
    /// Move a book back to the active books
    Activate { book: String },
    /// Move a book to the queue
    Queue { book: String },
    /// Change a book's goal
    SetGoal {
        book: String,
        goal: String,
        /// Move past sessions to the new goal as well
        #[arg(long)]
        move_history: bool,
    },
}

#[derive(Subcommand)]
enum InboxAction {
    /// Attach an inbox item to a book on the shelf
    Link { source: String, book: String },
    /// Stop showing an inbox item
    Ignore { source: String },
}

#[derive(Subcommand)]
enum GoalAction {
    /// Rename a goal key in the database (then edit config.toml)
    Rename { old: String, new: String },
}

#[derive(Subcommand)]
enum EmailAction {
    /// Save the SMTP password for nightly.smtp_user in the macOS Keychain
    SetPassword,
    /// Send the nightly email now
    Test,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Run the tracker in the foreground
    Run,
    /// Start the tracker at login with launchd
    Install,
    /// Stop the tracker and remove it from launchd
    Uninstall,
    /// Show whether the tracker is running
    Status,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("readr: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let app = App::load()?;
    let output = match cli.command {
        None | Some(Command::Status { short: false }) => {
            let today = status::Today::load(&app)?;
            let next = next::next_up(&app)?;
            status::long(&app, &today, next.as_ref())
        }
        Some(Command::Status { short: true }) => {
            format!("{}\n", status::short(&status::Today::load(&app)?))
        }
        Some(Command::Go { book }) => commands::go(&app, book.as_deref())?,
        Some(Command::Shelf { action }) => shelf_command(&app, action)?,
        Some(Command::Log {
            duration,
            book,
            note,
        }) => commands::log(&app, &duration, &book, note.as_deref())?,
        Some(Command::Inbox { action }) => match action {
            None => commands::inbox(&app)?,
            Some(InboxAction::Link { source, book }) => commands::inbox_link(&app, &source, &book)?,
            Some(InboxAction::Ignore { source }) => commands::inbox_ignore(&app, &source)?,
        },
        Some(Command::Stats) => commands::stats(&app)?,
        Some(Command::Goal {
            action: GoalAction::Rename { old, new },
        }) => commands::rename_goal(&app, &old, &new)?,
        Some(Command::Daemon { action }) => {
            match action {
                DaemonAction::Run => daemon::run(app)?,
                DaemonAction::Install => daemon::install(&app)?,
                DaemonAction::Uninstall => daemon::uninstall(&app)?,
                DaemonAction::Status => daemon::status(&app),
            }
            String::new()
        }
        Some(Command::Email { action }) => email_command(&app, action)?,
        Some(Command::NotifyTest) => notify_test(&app)?,
        Some(Command::Init) => doctor::init(&app)?,
        Some(Command::Doctor) => doctor::doctor(&app)?,
    };
    print!("{output}");
    Ok(())
}

fn shelf_command(app: &App, action: Option<ShelfAction>) -> Result<String> {
    match action {
        None => shelf::list(app),
        Some(ShelfAction::Add { query, goal, queue }) => shelf::add(
            app,
            shelf::AddOptions {
                query: &query,
                goal: goal.as_deref(),
                queue,
            },
        ),
        Some(ShelfAction::Finish { book }) => shelf::set_status(app, &book, BookStatus::Done),
        Some(ShelfAction::Drop { book }) => shelf::set_status(app, &book, BookStatus::Dropped),
        Some(ShelfAction::Activate { book }) => shelf::set_status(app, &book, BookStatus::Active),
        Some(ShelfAction::Queue { book }) => shelf::set_status(app, &book, BookStatus::Queued),
        Some(ShelfAction::SetGoal {
            book,
            goal,
            move_history,
        }) => shelf::set_goal(app, &book, &goal, move_history),
    }
}

fn email_command(app: &App, action: EmailAction) -> Result<String> {
    let Some(settings) = &app.config.nightly.email else {
        bail!(
            "set nightly.email_to and nightly.smtp_user in {}",
            app.paths.config_file.display()
        );
    };
    match action {
        EmailAction::SetPassword => {
            email::set_password(&settings.smtp_user)?;
            Ok(format!(
                "saved Keychain item readr-smtp for {}\n",
                settings.smtp_user
            ))
        }
        EmailAction::Test => {
            let today = status::Today::load(app)?;
            let next = next::next_up(app)?.map(|n| n.describe());
            let (subject, body) = remind::nightly_email(
                minutes(today.total),
                minutes(today.goal),
                today.streak,
                next.as_deref(),
            );
            email::send(settings, &subject, &body)?;
            Ok(format!("sent \"{subject}\" to {}\n", settings.to))
        }
    }
}

fn notify_test(app: &App) -> Result<String> {
    let today = status::Today::load(app)?;
    let next = next::next_up(app)?.map(|n| n.describe());
    let left = minutes(remaining(today.total, today.goal));
    let (title, body) = remind::remind_text(left, next.as_deref());
    Ok(match notify::notify(&title, &body)? {
        notify::Channel::Herdr => format!("shown in herdr: {title} / {body}\n"),
        notify::Channel::MacOs { herdr_error } => {
            format!("shown as a macOS notification (herdr: {herdr_error}): {title} / {body}\n")
        }
    })
}
