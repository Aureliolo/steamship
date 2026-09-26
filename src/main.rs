//! The command line.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this is the layer whose output is the interface"
)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use steamship::check;

/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
const REFUSED: u8 = 2;

#[derive(Debug, Parser)]
#[command(version, about = "Uploads game builds to Steam with Valve's steamcmd.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read an app build script and its depot scripts, without logging in, and refuse anything
    /// that would upload the wrong thing.
    Check {
        /// The app build script, such as `steam/app_build.vdf`.
        script: PathBuf,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { script } => run_check(&script),
    }
}

fn run_check(script: &Path) -> ExitCode {
    let report = check::check(script);
    if !report.problems.is_empty() {
        for problem in &report.problems {
            eprintln!("refused: {problem}");
        }
        return ExitCode::from(REFUSED);
    }
    let depots: Vec<String> = report
        .depots
        .iter()
        .map(|depot| {
            let count = depot.files.len();
            let noun = if count == 1 { "file" } else { "files" };
            format!("depot {} ({count} {noun})", depot.depot_id)
        })
        .collect();
    println!(
        "{}: app {}, {}; nothing refused",
        script.display(),
        report.app_id.unwrap_or_default(),
        depots.join(", ")
    );
    ExitCode::SUCCESS
}
