mod context;
mod daemon;
mod launchd;
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
            print!("{}", status::long(&app, &today));
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
    }
    Ok(())
}
