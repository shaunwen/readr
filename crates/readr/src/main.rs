mod context;
mod daemon;
mod email;
mod launchd;
mod next;
mod notify;
mod remind;
mod status;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
    match cli.command {
        None | Some(Command::Status { short: false }) => {
            let today = status::Today::load(&app)?;
            let next = next::next_up(&app)?;
            print!("{}", status::long(&app, &today, next.as_ref()));
        }
        Some(Command::Status { short: true }) => {
            let today = status::Today::load(&app)?;
            println!("{}", status::short(&today));
        }
        Some(Command::Daemon { action }) => match action {
            DaemonAction::Run => daemon::run(app)?,
            DaemonAction::Install => daemon::install(&app)?,
            DaemonAction::Uninstall => daemon::uninstall(&app)?,
            DaemonAction::Status => daemon::status(&app),
        },
        Some(Command::Email { action }) => {
            let Some(settings) = &app.config.nightly.email else {
                anyhow::bail!(
                    "set nightly.email_to and nightly.smtp_user in {}",
                    app.paths.config_file.display()
                );
            };
            match action {
                EmailAction::SetPassword => {
                    email::set_password(&settings.smtp_user)?;
                    println!("saved Keychain item readr-smtp for {}", settings.smtp_user);
                }
                EmailAction::Test => {
                    let today = status::Today::load(&app)?;
                    let next = next::next_up(&app)?.map(|n| n.describe());
                    let (subject, body) = remind::nightly_email(
                        readr_core::stats::minutes(today.total),
                        readr_core::stats::minutes(today.goal),
                        today.streak,
                        next.as_deref(),
                    );
                    email::send(settings, &subject, &body)?;
                    println!("sent \"{subject}\" to {}", settings.to);
                }
            }
        }
        Some(Command::NotifyTest) => {
            let today = status::Today::load(&app)?;
            let next = next::next_up(&app)?.map(|n| n.describe());
            let left =
                readr_core::stats::minutes(readr_core::stats::remaining(today.total, today.goal));
            let (title, body) = remind::remind_text(left, next.as_deref());
            match notify::notify(&title, &body)? {
                notify::Channel::Herdr => println!("shown in herdr: {title} / {body}"),
                notify::Channel::MacOs { herdr_error } => println!(
                    "shown as a macOS notification (herdr: {herdr_error}): {title} / {body}"
                ),
            }
        }
    }
    Ok(())
}
