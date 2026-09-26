//! The command line.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this is the layer whose output is the interface"
)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use steamship::check;
use steamship::install::{self, Outcome};
use steamship::manifest::Manifest;
use steamship::platform::Platform;

/// Something failed; what, and why, is printed.
const FAILED: u8 = 1;
/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
const REFUSED: u8 = 2;
/// steamcmd is missing, altered, or has updated itself past the pin.
const STEAMCMD: u8 = 4;

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
    /// Install the pinned steamcmd, or verify the one already installed. `login` and `upload`
    /// do this themselves; this does it ahead of time.
    Install,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { script } => run_check(&script),
        Command::Install => run_install(),
    }
}

fn run_install() -> ExitCode {
    let home = match Platform::THIS.home(|name| env::var_os(name)) {
        Ok(home) => home,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(FAILED);
        }
    };
    let manifest = match Manifest::pinned(Platform::THIS) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(FAILED);
        }
    };
    let root = home.join(install::FOLDER);
    let pinned = format!("steamcmd {} ({})", manifest.version, manifest.system);
    match install::install(&home, &manifest) {
        Ok(Outcome::Installed) => println!("{pinned} installed in {}", root.display()),
        Ok(Outcome::Verified) => println!("{pinned} in {} is as pinned", root.display()),
        Err(error @ install::Error::Altered { .. }) => {
            eprintln!("{error}");
            return ExitCode::from(STEAMCMD);
        }
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(FAILED);
        }
    }
    ExitCode::SUCCESS
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
