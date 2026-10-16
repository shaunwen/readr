mod context;
mod daemon;
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
        Some(Command::Daemon {
            action: DaemonAction::Run,
        }) => daemon::run(app)?,
    }
    Ok(())
}
