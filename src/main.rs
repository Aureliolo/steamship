//! The command line.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this is the layer whose output is the interface"
)]

use std::env;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use steamship::account::{Account, Asked};
use steamship::install::{self, Outcome};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::{check, run, steamcmd, upload};

/// Something failed; what, and why, is printed.
const FAILED: u8 = 1;
/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
const REFUSED: u8 = 2;
/// steamcmd has no login it can use; `login` again.
const LOGIN: u8 = 3;
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
    /// Check the scripts, then build and upload with steamcmd, out of sight, and say the build's
    /// ID.
    Upload {
        /// The app build script, such as `steam/app_build.vdf`.
        script: PathBuf,
        /// The version being shipped, which with the commit becomes the build's description.
        #[arg(long)]
        version: String,
        /// Valve's dry run: the whole build is computed and logged, and nothing is uploaded.
        #[arg(long)]
        preview: bool,
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
        Command::Upload {
            script,
            version,
            preview,
            account,
        } => run_upload(&Upload {
            script: &script,
            version: &version,
            preview,
            account: account.as_deref(),
        }),
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
/// With `ask`, and a person at a terminal, the account's name is asked for when there is none.
fn account(named: Option<&str>, ask: bool) -> Result<Account, ExitCode> {
    if let Some(name) = named {
        return Account::parse(name).map_err(|error| fail(&error, REFUSED));
    }
    let home = Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))?;
    match Account::remembered(&home) {
        Ok(Some(account)) => Ok(account),
        Ok(None) if ask && io::stdin().is_terminal() => {
            Account::ask(io::stdin().lock(), io::stderr()).map_err(|asked| {
                let code = if matches!(asked, Asked::Invalid(_)) {
                    REFUSED
                } else {
                    FAILED
                };
                fail(&asked, code)
            })
        }
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
    let account = match account(named, true) {
        Ok(account) => account,
        Err(code) => return code,
    };
    let (home, manifest) = match ready(false) {
        Ok(ready) => ready,
        Err(code) => return code,
    };
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    // The account's name is never printed: it is half of what logs it in, and output like this
    // ends up in logs that others read.
    println!(
        "steamcmd asks for the build account's password and a Steam Guard code, or for approval \
         in the Steam Mobile app; steamship reads neither."
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
            &format!("steamcmd did not log the build account in{exited}"),
            FAILED,
        );
    }
    if let Err(error) = account.remember(&home) {
        return fail(&format!("{}: {error}", home.display()), FAILED);
    }
    println!(
        "the build account is logged in; uploads use the login steamcmd keeps, until it expires"
    );
    ExitCode::SUCCESS
}

/// Refuses, naming every problem, when `check` found any.
fn checked(script: &Path) -> Result<check::Report, ExitCode> {
    let report = check::check(script);
    if report.problems.is_empty() {
        return Ok(report);
    }
    for problem in &report.problems {
        eprintln!("refused: {problem}");
    }
    Err(ExitCode::from(REFUSED))
}

struct Upload<'command> {
    script: &'command Path,
    version: &'command str,
    preview: bool,
    account: Option<&'command str>,
}

fn run_upload(upload: &Upload<'_>) -> ExitCode {
    match try_upload(upload) {
        Ok(code) | Err(code) => code,
    }
}

fn try_upload(request: &Upload<'_>) -> Result<ExitCode, ExitCode> {
    let account = account(request.account, false)?;
    drop(checked(request.script)?);
    let description = upload::commit(request.script)
        .and_then(|commit| upload::description(request.version, &commit))
        .map_err(|error| fail(&error, REFUSED))?;
    let (home, manifest) = ready(false)?;
    let prepared = upload::prepare(&home, request.script, &description, request.preview)
        .map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let kind = if request.preview { "a preview of " } else { "" };
    println!(
        "building {kind}app {} as \"{description}\"",
        prepared.app_id
    );
    let finished = run::run(
        &program,
        &steamcmd::upload(&account, &prepared.script),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::UPLOAD_LIMIT,
    )
    .map_err(|error| fail(&format!("{}: {error}", program.display()), FAILED))?;
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    let saved = prepared.output.join("steamcmd.log");
    fs::write(&saved, &console)
        .map_err(|error| fail(&format!("{}: {error}", saved.display()), FAILED))?;
    let log = fs::read(prepared.log())
        .ok()
        .map(|log| redactor.redact(&log));
    let outcome = upload::judge(
        prepared.app_id,
        finished.code,
        &String::from_utf8_lossy(&console),
        log.as_deref().map(String::from_utf8_lossy).as_deref(),
        request.preview,
    );
    let code = report(&outcome, &prepared, &saved);
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

fn report(outcome: &upload::Outcome, prepared: &upload::Prepared, saved: &Path) -> ExitCode {
    let app = prepared.app_id;
    match outcome {
        upload::Outcome::Built { build_id } => {
            let live = prepared
                .set_live
                .as_ref()
                .map(|branch| format!(", set live on {branch}"))
                .unwrap_or_default();
            println!("app {app}: BuildID {build_id}{live}");
            ExitCode::SUCCESS
        }
        upload::Outcome::Previewed => {
            println!("app {app}: the preview finished; nothing was uploaded");
            ExitCode::SUCCESS
        }
        upload::Outcome::NotLoggedIn(line) => {
            eprintln!("{line}");
            eprintln!("log the build account in again with `steamship login`");
            ExitCode::from(LOGIN)
        }
        upload::Outcome::Failed(reasons) => {
            for reason in reasons {
                eprintln!("failed: {reason}");
            }
            eprintln!(
                "steamcmd's output is in {}, and its build log in {}",
                saved.display(),
                prepared.log().display()
            );
            ExitCode::from(FAILED)
        }
    }
}

fn run_check(script: &Path) -> ExitCode {
    let report = match checked(script) {
        Ok(report) => report,
        Err(code) => return code,
    };
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
