//! The command line.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use steamship::account::Account;
use steamship::install::{self, Outcome, State};
use steamship::login::{self, Ending, Person};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
#[cfg(unix)]
use steamship::unix::Terminal;
use steamship::update::{self, Installed};
#[cfg(windows)]
use steamship::windows::Terminal;
use steamship::{check, conversation, run, steamcmd, upload};
use zeroize::Zeroizing;

/// Something failed; what, and why, is printed.
const FAILED: u8 = 1;
/// `check` refused the scripts, or the command line was wrong (clap uses 2 for the latter too).
const REFUSED: u8 = 2;
/// steamcmd has no login it can use; `login` again.
const LOGIN: u8 = 3;
/// steamcmd is missing or has been altered.
const STEAMCMD: u8 = 4;

#[derive(Debug, Parser)]
#[command(
    name = "steamship",
    bin_name = "steamship",
    version,
    about = "Uploads game builds to Steam with Valve's steamcmd.",
    styles = show::HELP
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Log in to Steam, once, for uploads.
    ///
    /// Asks for the account's name when none is given or remembered, then the password, and a
    /// Steam Guard code or approval in the Steam Mobile app. What you type is passed directly to
    /// steamcmd, never logged or saved.
    Login {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = "STEAMSHIP_ACCOUNT")]
        account: Option<String>,
    },
    /// Show the login and steamcmd, checking the login with Steam.
    ///
    /// Shows the home, the build account and steamcmd, then logs in with the login steamcmd
    /// saved, as an upload does, and says whether Steam takes it. Nothing is asked for.
    Status {
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = "STEAMSHIP_ACCOUNT")]
        account: Option<String>,
    },
    /// Check the build scripts, without logging in.
    ///
    /// Reads an app build script and every depot script it names, and refuses anything that
    /// would upload the wrong thing.
    Check {
        /// The app build script, such as `steam/app_build.vdf`.
        script: PathBuf,
    },
    /// Check, build and upload, then print the build ID.
    ///
    /// Runs `check`, then the build, with steamcmd out of sight, and says the build's ID and
    /// the branch it was set live on.
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
    /// Forget the saved login.
    ///
    /// Removes the login steamcmd saved and the account steamship remembered; the next upload
    /// needs `steamship login` first.
    Logout,
    /// Install or verify the pinned steamcmd.
    ///
    /// `login` and `upload` do this themselves; this does it ahead of time, as in CI.
    Install,
}

fn main() -> ExitCode {
    let mut cli = Cli::command();
    if io::stdout().is_terminal() {
        cli = cli.before_help(show::banner());
    }
    let parsed = cli
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let command = match parsed {
        Ok(parsed) => parsed.command,
        Err(error) => error.exit(),
    };
    let update = update_check();
    let code = match command {
        Command::Check { script } => run_check(&script),
        Command::Install => run_install(),
        Command::Login { account } => match try_login(account.as_deref()) {
            Ok(code) | Err(code) => code,
        },
        Command::Logout => run_logout(),
        Command::Status { account } => match try_status(account.as_deref()) {
            Ok(code) | Err(code) => code,
        },
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
    };
    if let Some(latest) = update.and_then(update::Check::newer) {
        let installed = env::current_exe().map_or(Installed::Archive, |program| {
            Installed::of(&program, Path::exists)
        });
        show::upgrade(
            &latest.to_string(),
            env!("CARGO_PKG_VERSION"),
            installed.hint(),
        );
    }
    code
}

/// Whether a newer steamship is out, found out while the command runs, when a person at a
/// terminal would be told.
fn update_check() -> Option<update::Check> {
    if !update::wanted(|name| env::var_os(name), io::stderr().is_terminal()) {
        return None;
    }
    let home = Platform::THIS.home(|name| env::var_os(name)).ok()?;
    Some(update::Check::start(home))
}

fn home() -> Result<PathBuf, ExitCode> {
    Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))
}

/// The home, and the pinned steamcmd in it, installed or verified.
fn ready() -> Result<(PathBuf, Manifest), ExitCode> {
    let home = home()?;
    let manifest = Manifest::pinned(Platform::THIS).map_err(|error| fail(&error, FAILED))?;
    let spinner = Spinner::start("steamcmd", "checking the pinned version", false);
    match install::install(&home, &manifest) {
        Ok(Outcome::Installed) => spinner.done(&format!(
            "{} ({}), installed as pinned",
            manifest.version, manifest.system
        )),
        Ok(Outcome::Verified) => spinner.done(&format!(
            "{} ({}), pinned and verified",
            manifest.version, manifest.system
        )),
        Err(error) => {
            spinner.failed("not as pinned");
            return Err(steamcmd_failed(&error));
        }
    }
    Ok((home, manifest))
}

fn run_install() -> ExitCode {
    show::title("install");
    match ready() {
        Ok((home, _)) => {
            show::field("folder", &home.join(install::FOLDER).display().to_string());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn fail(error: &dyn Display, code: u8) -> ExitCode {
    show::failure(&error.to_string(), "", None);
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

/// Where the account came from, which is what `login` shows of it: its name is never printed,
/// being half of what logs it in, and output like this ends up in logs that others read.
#[derive(Debug, Clone, Copy)]
enum Source {
    Named,
    Remembered,
    Typed,
}

/// The account named on the command line or in the environment, or else the one remembered.
/// With `ask`, and a person at a terminal, the account's name is asked for when there is none.
fn account(named: Option<&str>, ask: bool) -> Result<(Account, Source), ExitCode> {
    if let Some(name) = named {
        let account = Account::parse(name).map_err(|error| fail(&error, REFUSED))?;
        return Ok((account, Source::Named));
    }
    let home = home()?;
    match Account::remembered(&home) {
        Ok(Some(account)) => Ok((account, Source::Remembered)),
        Ok(None) if ask && io::stdin().is_terminal() => typing::ask("account", Echo::Typed)
            .map_err(|error| fail(&format!("the account name: {error}"), FAILED))
            .and_then(|name| Account::parse(&name).map_err(|error| fail(&error, REFUSED)))
            .map(|account| (account, Source::Typed)),
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

/// What steamship shows while it waits for the login to be approved.
const APPROVING: &str = "waiting for you in the Steam Mobile app";

/// The person at the terminal, answering steamcmd through steamship's own prompts.
#[derive(Debug, Default)]
struct Typist {
    approving: Option<Spinner>,
    /// steamcmd used the login it saved, so nothing was asked.
    saved: bool,
}

impl Typist {
    fn ask(&mut self, label: &str, echo: Echo) -> io::Result<Zeroizing<String>> {
        drop(self.approving.take());
        typing::ask(label, echo)
    }
}

impl Person for Typist {
    fn password(&mut self) -> io::Result<Zeroizing<String>> {
        self.ask("password", Echo::Dots)
    }

    fn code(&mut self) -> io::Result<Zeroizing<String>> {
        self.ask("code", Echo::Typed)
    }

    fn approving(&mut self) {
        if self.approving.is_none() {
            self.approving = Some(Spinner::start("approve", APPROVING, true));
        }
    }

    fn approved(&mut self) {
        match self.approving.take() {
            Some(spinner) => spinner.done("approved"),
            None => show::done("approve", "approved"),
        }
    }

    fn saved(&mut self) {
        self.saved = true;
    }

    fn said(&mut self, line: &str) {
        if let Some(spinner) = self.approving.take() {
            self.approving = Some(spinner.around(|| show::aside(line), APPROVING, true));
        } else {
            show::aside(line);
        }
    }
}

/// `login` failed, for `reason`, and can be tried again.
fn not_logged_in(reason: &str) -> ExitCode {
    let again = Hint {
        before: "run ",
        command: "steamship login",
        after: " to try again",
    };
    show::failure("not logged in", reason, Some(again));
    ExitCode::from(FAILED)
}

fn run_logout() -> ExitCode {
    show::title("logout");
    let forgot = home().and_then(|home| {
        let files = steamcmd::login_files(&home)
            .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?;
        for file in &files {
            fs::remove_file(file)
                .map_err(|error| fail(&format!("{}: {error}", file.display()), FAILED))?;
        }
        let account = Account::forget(&home)
            .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?;
        Ok((!files.is_empty(), account))
    });
    let (login, account) = match forgot {
        Ok(forgot) => forgot,
        Err(code) => return code,
    };
    if login {
        show::done("login", "forgotten");
    } else {
        show::field("login", "none saved");
    }
    if account {
        show::done("account", "forgotten");
    } else {
        show::field("account", "none remembered");
    }
    show::success(
        "logged out",
        "the next upload needs `steamship login` first",
    );
    ExitCode::SUCCESS
}

/// What the steamcmd in `home` is, shown without installing or changing anything.
fn steamcmd_state(home: &Path, manifest: &Manifest) -> Result<(), ExitCode> {
    match install::state(home, manifest) {
        Ok(State::Pinned) => show::done(
            "steamcmd",
            &format!(
                "{} ({}), pinned and verified",
                manifest.version, manifest.system
            ),
        ),
        Ok(State::Earlier) => show::field(
            "steamcmd",
            "an earlier pin; the next login or upload replaces it",
        ),
        Ok(State::Missing) => {
            show::field("steamcmd", "not installed; `steamship login` installs it");
        }
        Err(error) => {
            show::failed("steamcmd", "not as pinned");
            return Err(steamcmd_failed(&error));
        }
    }
    Ok(())
}

fn try_status(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::title("status");
    show::field("version", env!("CARGO_PKG_VERSION"));
    let home = home()?;
    show::field("home", &home.display().to_string());
    let account = status_account(named, &home)?;
    let saved = !steamcmd::login_files(&home)
        .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?
        .is_empty();
    let login = Hint {
        before: "run ",
        command: "steamship login",
        after: "",
    };
    let Some(account) = account.filter(|_| saved) else {
        let manifest = Manifest::pinned(Platform::THIS).map_err(|error| fail(&error, FAILED))?;
        steamcmd_state(&home, &manifest)?;
        if saved {
            show::field("login", "saved");
            show::failure("no build account", "", Some(login));
        } else {
            show::field("login", "none saved");
            show::failure("not logged in", "", Some(login));
        }
        return Ok(ExitCode::from(LOGIN));
    };
    check_saved_login(&account)
}

/// The account named, or else the one remembered, shown by where it came from and never by
/// name, as `login` shows it.
fn status_account(named: Option<&str>, home: &Path) -> Result<Option<Account>, ExitCode> {
    if let Some(name) = named {
        let account = Account::parse(name).map_err(|error| fail(&error, REFUSED))?;
        show::field("account", "as named");
        return Ok(Some(account));
    }
    let remembered = Account::remembered(home).map_err(|error| {
        fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )
    })?;
    show::field(
        "account",
        if remembered.is_some() {
            "remembered"
        } else {
            "none remembered"
        },
    );
    Ok(remembered)
}

/// Logs `account` in with the login steamcmd saved, as an upload does, and says what Steam made
/// of it.
fn check_saved_login(account: &Account) -> Result<ExitCode, ExitCode> {
    let (home, manifest) = ready()?;
    show::field("login", "saved");
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let spinner = Spinner::start("steam", "checking the saved login", true);
    let finished = run::run(
        &program,
        &steamcmd::check_login(account),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::CHECK_LIMIT,
    );
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    let judged = upload::judge_login(finished.code, &String::from_utf8_lossy(&console));
    match &judged {
        upload::Login::Taken => spinner.done("Steam takes the saved login"),
        upload::Login::Refused(_) => spinner.failed("Steam refused the saved login"),
        upload::Login::Failed(_) => spinner.failed("could not check"),
    }
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(match judged {
        upload::Login::Taken => {
            show::success("ready to upload", "");
            ExitCode::SUCCESS
        }
        upload::Login::Refused(reason) => {
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", &reason, Some(again));
            ExitCode::from(LOGIN)
        }
        upload::Login::Failed(reasons) => {
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            ExitCode::from(FAILED)
        }
    })
}

fn try_login(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("login");
    let (account, source) = account(named, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    let (home, manifest) = ready()?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let environment = steamcmd::environment(&home, Platform::THIS);
    let (terminal, mut output, mut input) =
        Terminal::start(&program, &steamcmd::login(&account), &environment, &root)
            .map_err(|error| not_logged_in(&format!("{}: {error}", program.display())))?;
    let mut typist = Typist::default();
    let conversed = login::converse(&mut output, &mut input, &mut typist);
    if let Some(spinner) = typist.approving.take() {
        spinner.failed("not approved");
    }
    // Dropping the terminal unwaited ends steamcmd, which is what giving up should do.
    let ending = conversed.map_err(|error| {
        let kind = error.kind();
        not_logged_in(&if kind == io::ErrorKind::Interrupted {
            "given up".to_owned()
        } else if kind == io::ErrorKind::UnexpectedEof {
            "input ended before steamcmd had an answer".to_owned()
        } else {
            error.to_string()
        })
    })?;
    drop(input);
    let code = terminal
        .wait()
        .map_err(|error| not_logged_in(&format!("{}: {error}", program.display())))?;
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    if let Ending::Refused(reason) = ending {
        return Err(not_logged_in(&conversation::explain(&reason)));
    }
    if code != Some(0_i32) {
        let exited = code.map_or_else(
            || "steamcmd was stopped".to_owned(),
            |code| format!("steamcmd exited {code}"),
        );
        return Err(not_logged_in(&exited));
    }
    account
        .remember(&home)
        .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?;
    if typist.saved {
        show::success(
            "already logged in",
            "uploads use the saved login until it expires; `steamship logout` forgets it",
        );
    } else {
        show::success("logged in", "uploads use this login until it expires");
    }
    Ok(ExitCode::SUCCESS)
}

/// Refuses, naming every problem, when `check` found any; otherwise shows what was checked.
fn checked(script: &Path) -> Result<check::Report, ExitCode> {
    let report = check::check(script);
    if report.problems.is_empty() {
        let files = report.depots.iter().map(|depot| depot.files.len()).sum();
        show::done(
            "app",
            &format!(
                "{}, {}, {}, checked",
                report.app_id.unwrap_or_default(),
                show::counted(report.depots.len(), "depot"),
                show::counted(files, "file"),
            ),
        );
        return Ok(report);
    }
    for problem in &report.problems {
        show::failure("refused", &problem.to_string(), None);
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
    show::title(if request.preview {
        "upload, preview"
    } else {
        "upload"
    });
    let (account, _) = account(request.account, false)?;
    drop(checked(request.script)?);
    let description = upload::commit(request.script)
        .and_then(|commit| upload::description(request.version, &commit))
        .map_err(|error| fail(&error, REFUSED))?;
    show::field("build", &description);
    let (home, manifest) = ready()?;
    let prepared = upload::prepare(&home, request.script, &description, request.preview)
        .map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let doing = if request.preview {
        "computing the build"
    } else {
        "uploading"
    };
    let spinner = Spinner::start("steam", doing, true);
    let finished = run::run(
        &program,
        &steamcmd::upload(&account, &prepared.script),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::UPLOAD_LIMIT,
    );
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
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
    let code = report(&outcome, spinner, &took, &prepared, &saved);
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

fn report(
    outcome: &upload::Outcome,
    spinner: Spinner,
    took: &str,
    prepared: &upload::Prepared,
    saved: &Path,
) -> ExitCode {
    let app = prepared.app_id;
    let logs = format!(
        "log  {}, and steamcmd's own in {}",
        saved.display(),
        prepared.log().display()
    );
    match outcome {
        upload::Outcome::Built { build_id } => {
            spinner.done(&format!("uploaded in {took}"));
            let live = prepared
                .set_live
                .as_ref()
                .map(|branch| format!(", set live on {branch}"))
                .unwrap_or_default();
            show::success(&format!("app {app}: BuildID {build_id}{live}"), &logs);
            ExitCode::SUCCESS
        }
        upload::Outcome::Previewed => {
            spinner.done(&format!("preview finished in {took}"));
            show::success("nothing was uploaded, as asked", &logs);
            ExitCode::SUCCESS
        }
        upload::Outcome::NotLoggedIn(line) => {
            spinner.failed(&format!("refused after {took}"));
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", line, Some(again));
            ExitCode::from(LOGIN)
        }
        upload::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in reasons {
                show::failure(reason, "", None);
            }
            show::note(&logs);
            ExitCode::from(FAILED)
        }
    }
}

fn run_check(script: &Path) -> ExitCode {
    show::title("check");
    show::field("script", &script.display().to_string());
    match checked(script) {
        Ok(report) => {
            for depot in &report.depots {
                show::field(
                    "depot",
                    &format!(
                        "{}, {}",
                        depot.depot_id,
                        show::counted(depot.files.len(), "file")
                    ),
                );
            }
            show::success("nothing refused", "");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}
