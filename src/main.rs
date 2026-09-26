//! The command line.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this is the layer whose output is the interface"
)]

use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use steamship::account::Account;
use steamship::install::{self, Outcome};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::{check, run, steamcmd};

/// Something failed; what, and why, is printed.
const FAILED: u8 = 1;
/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
const REFUSED: u8 = 2;
/// steamcmd is missing or has been altered.
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
    /// Log the build account in to steamcmd, in this terminal: steamcmd asks for the password
    /// and a Steam Guard code, and steamship reads neither.
    Login {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = "STEAMSHIP_ACCOUNT")]
        account: Option<String>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { script } => run_check(&script),
        Command::Install => match ready(true) {
            Ok(_) => ExitCode::SUCCESS,
            Err(code) => code,
        },
        Command::Login { account } => run_login(account.as_deref()),
    }
}

/// The home, and the pinned steamcmd in it, installed or verified. `always` says whether to say
/// so when it was already there.
fn ready(always: bool) -> Result<(PathBuf, Manifest), ExitCode> {
    let home = Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))?;
    let manifest = Manifest::pinned(Platform::THIS).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let pinned = format!("steamcmd {} ({})", manifest.version, manifest.system);
    match install::install(&home, &manifest) {
        Ok(Outcome::Installed) => println!("{pinned} installed in {}", root.display()),
        Ok(Outcome::Verified) if always => {
            println!("{pinned} in {} is as pinned", root.display());
        }
        Ok(Outcome::Verified) => {}
        Err(error) => return Err(steamcmd_failed(&error)),
    }
    Ok((home, manifest))
}

fn fail(error: &dyn Display, code: u8) -> ExitCode {
    eprintln!("{error}");
    ExitCode::from(code)
}

fn steamcmd_failed(error: &install::Error) -> ExitCode {
    let code = if matches!(error, install::Error::Altered { .. }) {
        STEAMCMD
    } else {
        FAILED
    };
    fail(error, code)
}

/// The account named on the command line or in the environment, or else the one remembered.
fn account(named: Option<&str>) -> Result<Account, ExitCode> {
    if let Some(name) = named {
        return Account::parse(name).map_err(|error| fail(&error, REFUSED));
    }
    let home = Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))?;
    match Account::remembered(&home) {
        Ok(Some(account)) => Ok(account),
        Ok(None) => Err(fail(
            &"name the build account with --account or STEAMSHIP_ACCOUNT",
            REFUSED,
        )),
        Err(error) => Err(fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )),
    }
}

fn run_login(named: Option<&str>) -> ExitCode {
    let account = match account(named) {
        Ok(account) => account,
        Err(code) => return code,
    };
    let (home, manifest) = match ready(false) {
        Ok(ready) => ready,
        Err(code) => return code,
    };
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    println!(
        "steamcmd asks for the password of {account} and a Steam Guard code, or for approval in \
         the Steam Mobile app; steamship reads neither."
    );
    let environment = steamcmd::environment(&home, Platform::THIS);
    let code = match run::attached(&program, &steamcmd::login(&account), &environment, &root) {
        Ok(code) => code,
        Err(error) => return fail(&format!("{}: {error}", program.display()), FAILED),
    };
    if let Err(error) = install::verify(&home, &manifest) {
        return steamcmd_failed(&error);
    }
    if code != Some(0_i32) {
        let exited = code
            .map(|code| format!(" (it exited {code})"))
            .unwrap_or_default();
        return fail(
            &format!("steamcmd did not log {account} in{exited}"),
            FAILED,
        );
    }
    if let Err(error) = account.remember(&home) {
        return fail(&format!("{}: {error}", home.display()), FAILED);
    }
    println!("{account} is logged in; uploads use the login steamcmd keeps, until it expires");
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
