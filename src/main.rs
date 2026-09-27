//! The command line.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{self, Path, PathBuf};
use std::process::{self, ExitCode};

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
use steamship::{check, ci, conversation, run, scripts, steamcmd, upload, webapi, workshop};
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
    /// Upload a Workshop item, then print its ID.
    ///
    /// Checks a `workshopitem` script and what it names, then has steamcmd upload the item with
    /// the saved login. A script with no `publishedfileid` makes a new item, whose ID is printed
    /// to add to the script so that later uploads update the same item.
    Workshop {
        /// The item script, such as `workshop/item.vdf`.
        script: PathBuf,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = "STEAMSHIP_ACCOUNT")]
        account: Option<String>,
    },
    /// Show an app's branches and last builds.
    ///
    /// Lists each branch with the build live on it, then the last builds uploaded, through
    /// Steam's partner Web API, with the publisher key in `STEAMSHIP_WEB_API_KEY`.
    Builds {
        /// The app, by its ID or its app build script.
        app: String,
        /// How many of the last builds to list.
        #[arg(long, default_value_t = 10)]
        count: u32,
    },
    /// Set an uploaded build live on a branch.
    ///
    /// Sets a build live on a beta branch without uploading it again, through Steam's partner
    /// Web API, with the publisher key in `STEAMSHIP_WEB_API_KEY`. The default branch is set
    /// live in Steamworks only.
    Promote {
        /// The app, by its ID or its app build script.
        app: String,
        /// The build, by its build ID.
        #[arg(long)]
        build: u64,
        /// The branch to set it live on.
        #[arg(long)]
        branch: String,
    },
    /// Set up uploads from CI, the login kept as a secret.
    ///
    /// Finds the GitHub repository, its app build script and the saved login, checks the script,
    /// and the login with Steam, then sets the login as a secret with the GitHub command line,
    /// `gh`, and shows the workflow step that uploads. What it cannot find it asks for, and each
    /// option answers ahead. The secret holds the token steamcmd saved, never a password.
    Ci {
        /// The app build script, if not the one found in the repository.
        #[arg(long)]
        script: Option<PathBuf>,
        /// The GitHub repository as owner/name, if not the one `origin` points at.
        #[arg(long)]
        repo: Option<String>,
        /// The build account, if not the one the last login remembered.
        #[arg(long, env = "STEAMSHIP_ACCOUNT")]
        account: Option<String>,
        /// The name of the secret the login is kept in.
        #[arg(long, default_value = ci::VARIABLE)]
        secret: String,
        /// Write the login to this file instead, for a CI other than GitHub Actions.
        #[arg(long)]
        output: Option<PathBuf>,
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
        Command::Ci {
            script,
            repo,
            account,
            secret,
            output,
        } => match try_ci(&Setup {
            script: script.as_deref(),
            repo: repo.as_deref(),
            account: named(account.as_deref()),
            secret: &secret,
            output: output.as_deref(),
        }) {
            Ok(code) | Err(code) => code,
        },
        Command::Install => run_install(),
        Command::Builds { app, count } => match try_builds(&app, count) {
            Ok(code) | Err(code) => code,
        },
        Command::Promote { app, build, branch } => match try_promote(&app, build, &branch) {
            Ok(code) | Err(code) => code,
        },
        Command::Workshop { script, account } => {
            match try_workshop(&script, named(account.as_deref())) {
                Ok(code) | Err(code) => code,
            }
        }
        Command::Login { account } => match try_login(named(account.as_deref())) {
            Ok(code) | Err(code) => code,
        },
        Command::Logout => run_logout(),
        Command::Status { account } => match try_status(named(account.as_deref())) {
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
            account: named(account.as_deref()),
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

/// The account named with `--account` or `STEAMSHIP_ACCOUNT`, where an empty name is none: a CI
/// hands a secret it does not have over as an empty variable.
fn named(account: Option<&str>) -> Option<&str> {
    account.filter(|name| !name.is_empty())
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

/// The login a CI handed over in `STEAMSHIP_LOGIN`, packed by `steamship ci`, if it did.
fn packed_login() -> Result<Option<ci::Login>, ExitCode> {
    let Some(value) = env::var_os(ci::VARIABLE).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let text = value
        .to_str()
        .ok_or_else(|| fail(&ci::Error::Unreadable, REFUSED))?;
    ci::Login::unpack(text)
        .map(Some)
        .map_err(|error| fail(&error, REFUSED))
}

/// Puts a login handed over in `STEAMSHIP_LOGIN` where steamcmd looks for it in `home`.
fn restore(packed: Option<&ci::Login>, home: &Path) -> Result<(), ExitCode> {
    if let Some(packed) = packed {
        drop(
            packed
                .restore(home, Platform::THIS)
                .map_err(|error| fail(&error, FAILED))?,
        );
    }
    Ok(())
}

fn try_status(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("status");
    show::field("version", env!("CARGO_PKG_VERSION"));
    let home = home()?;
    show::field("home", &home.display().to_string());
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => {
            show::field("account", &format!("from {}", ci::VARIABLE));
            Some(packed.account().clone())
        }
        _ => status_account(named, &home)?,
    };
    let saved = packed.is_some()
        || !steamcmd::login_files(&home)
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
    let judged = check_saved_login(&account, packed.as_ref())?;
    if judged == upload::Login::Taken {
        show::success("ready to upload", "");
    }
    Ok(verdict(judged))
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

/// Logs `account` in with the login steamcmd saved, or the one handed over in `packed`, as an
/// upload does, and shows what Steam made of it.
fn check_saved_login(
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<upload::Login, ExitCode> {
    let (home, manifest) = ready()?;
    restore(packed, &home)?;
    if packed.is_some() {
        show::field("login", &format!("from {}", ci::VARIABLE));
    } else {
        show::field("login", "saved");
    }
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
    Ok(judged)
}

/// The exit code for what Steam made of the saved login, saying why when it did not take it.
fn verdict(judged: upload::Login) -> ExitCode {
    match judged {
        upload::Login::Taken => ExitCode::SUCCESS,
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
    }
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
    let packed = packed_login()?;
    let account = match (&packed, request.account) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(request.account, false)?.0,
    };
    drop(checked(request.script)?);
    let description = upload::commit(request.script)
        .and_then(|commit| upload::description(request.version, &commit))
        .map_err(|error| fail(&error, REFUSED))?;
    show::field("build", &description);
    let (home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
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
            hand_on(*build_id);
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

/// The partner Web API, with the publisher key from `STEAMSHIP_WEB_API_KEY`.
fn web_api() -> Result<webapi::Api, ExitCode> {
    let Some(text) = env::var(webapi::KEY).ok().filter(|text| !text.is_empty()) else {
        let key = Hint {
            before: "set ",
            command: webapi::KEY,
            after: " to the publisher Web API key from Steamworks, under Users & Permissions, \
                    Manage Groups",
        };
        show::failure("no Web API key", "", Some(key));
        return Err(ExitCode::from(REFUSED));
    };
    let key = webapi::Key::parse(&text).map_err(|error| fail(&error, REFUSED))?;
    Ok(webapi::Api::new(key))
}

/// The app named by its ID, or by the app build script that names it.
fn app_named(app: &str) -> Result<u32, ExitCode> {
    let app_id = match app.parse::<u32>() {
        Ok(app_id) if app_id > 0 => app_id,
        Ok(_) => return Err(fail(&"0 is not an app's ID", REFUSED)),
        Err(_) => scripts::load(Path::new(app))
            .map(|script| script.app_id)
            .map_err(|problems| {
                for problem in &problems {
                    show::failure("refused", &problem.to_string(), None);
                }
                ExitCode::from(REFUSED)
            })?,
    };
    show::field("app", &app_id.to_string());
    Ok(app_id)
}

/// Says why the Web API did not answer, and the exit code for it.
fn web_api_failed(spinner: Spinner, error: &webapi::Error) -> ExitCode {
    spinner.failed("no answer");
    fail(error, FAILED)
}

fn try_builds(app: &str, count: u32) -> Result<ExitCode, ExitCode> {
    show::title("builds");
    let app_id = app_named(app)?;
    let api = web_api()?;
    let spinner = Spinner::start("steam", "asking for the branches and builds", false);
    let overview = match api.overview(app_id, count) {
        Ok(overview) => overview,
        Err(error) => return Err(web_api_failed(spinner, &error)),
    };
    spinner.done(&overview.summary());
    for (label, line) in overview.lines() {
        show::field(&label, &line);
    }
    Ok(ExitCode::SUCCESS)
}

fn try_promote(app: &str, build: u64, branch: &str) -> Result<ExitCode, ExitCode> {
    show::title("promote");
    let app_id = app_named(app)?;
    if webapi::is_default(branch) {
        return Err(fail(
            &"the default branch is set live in Steamworks, not by steamship",
            REFUSED,
        ));
    }
    show::field("build", &build.to_string());
    show::field("branch", branch);
    let api = web_api()?;
    let spinner = Spinner::start("steam", "setting it live", false);
    if let Err(error) = api.set_live(app_id, build, branch) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("set live");
    show::success(
        &format!("app {app_id}: BuildID {build} live on {branch}"),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

fn try_workshop(script: &Path, named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::title("workshop");
    show::field("script", &script.display().to_string());
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let item = workshop::check(script).map_err(|problems| {
        for problem in &problems {
            show::failure("refused", &problem.to_string(), None);
        }
        ExitCode::from(REFUSED)
    })?;
    let which = item.published.map_or_else(
        || "a new item".to_owned(),
        |published| format!("item {published}"),
    );
    show::done(
        "item",
        &format!(
            "app {}, {which}, {}, checked",
            item.app_id,
            show::counted(item.files, "file")
        ),
    );
    let (home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let copy =
        workshop::prepare(&home, script, item.app_id).map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let spinner = Spinner::start("steam", "uploading", true);
    let finished = run::run(
        &program,
        &steamcmd::workshop(&account, &copy),
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
    let saved = copy.with_file_name("steamcmd.log");
    fs::write(&saved, &console)
        .map_err(|error| fail(&format!("{}: {error}", saved.display()), FAILED))?;
    let outcome = workshop::judge(
        finished.code,
        &String::from_utf8_lossy(&console),
        workshop::published(&copy),
    );
    let log = format!("log  {}", saved.display());
    let code = match outcome {
        workshop::Outcome::Published { published } => {
            spinner.done(&format!("uploaded in {took}"));
            let detail = if item.published.is_none() {
                format!(
                    "a new item: add \"publishedfileid\" \"{published}\" to {} so that later \
                     uploads update it",
                    script.display()
                )
            } else {
                log
            };
            show::success(
                &format!("app {}: Workshop item {published}", item.app_id),
                &detail,
            );
            ExitCode::SUCCESS
        }
        workshop::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", &reason, Some(again));
            ExitCode::from(LOGIN)
        }
        workshop::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            show::note(&log);
            ExitCode::from(FAILED)
        }
    };
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

/// Hands `build_id` on as the `build-id` output of the GitHub Actions step steamship runs in,
/// through the file GitHub names for a step's outputs. The upload is done by then, so a file that
/// cannot be written is said and does not fail it.
fn hand_on(build_id: u64) {
    let Some(outputs) = env::var_os("GITHUB_OUTPUT").filter(|path| !path.is_empty()) else {
        return;
    };
    let written = fs::OpenOptions::new()
        .append(true)
        .open(&outputs)
        .and_then(|mut file| {
            io::Write::write_all(&mut file, format!("build-id={build_id}\n").as_bytes())
        });
    if let Err(error) = written {
        show::note(&format!(
            "the BuildID could not be handed to the workflow: {error}"
        ));
    }
}

struct Setup<'command> {
    script: Option<&'command Path>,
    repo: Option<&'command str>,
    account: Option<&'command str>,
    secret: &'command str,
    output: Option<&'command Path>,
}

fn try_ci(setup: &Setup<'_>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("ci");
    if !ci::secret_name(setup.secret) {
        return Err(fail(
            &format!(
                "{} cannot name a GitHub secret: letters, digits and underscores, not starting \
                 with a digit or GITHUB_",
                setup.secret
            ),
            REFUSED,
        ));
    }
    let root = repository_root(Path::new("."));
    let script = ci_script(setup.script, root.as_deref())?;
    drop(checked(&script)?);
    let repository = match setup.output {
        Some(_) => None,
        None => Some(ci_repository(setup.repo, &script)?),
    };
    let (account, source) = account(setup.account, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    let home = home()?;
    let packed = ci::Login::saved(&home, Platform::THIS, account.clone()).map_err(|error| {
        if matches!(error, ci::Error::NotSaved { .. }) {
            let login = Hint {
                before: "log in here first with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", "", Some(login));
            ExitCode::from(LOGIN)
        } else {
            fail(&error, FAILED)
        }
    })?;
    let judged = check_saved_login(&account, None)?;
    if judged != upload::Login::Taken {
        return Ok(verdict(judged));
    }
    let again = "when the login expires, run `steamship login` and `steamship ci` again";
    match (setup.output, repository) {
        (Some(output), _) => {
            ci::write_private(output, packed.packed().as_bytes())
                .map_err(|error| fail(&format!("{}: {error}", output.display()), FAILED))?;
            show::done("secret", &format!("written to {}", output.display()));
            show::success(
                "ready for CI",
                &format!(
                    "give your CI the file's contents as {}, then delete the file; {again}",
                    ci::VARIABLE
                ),
            );
        }
        (None, Some(repository)) => {
            set_secret(&repository, setup.secret, &packed)?;
            let relative = root
                .as_deref()
                .and_then(|root| {
                    path::absolute(&script)
                        .ok()?
                        .strip_prefix(root)
                        .ok()
                        .map(Path::to_path_buf)
                })
                .unwrap_or_else(|| script.clone());
            let (pinned, release) = pinned_action();
            show::success(
                "ready for CI",
                &format!("add this step after the one that builds the content; {again}"),
            );
            show::verbatim(&ci::step(
                &relative.to_string_lossy().replace('\\', "/"),
                &pinned,
                &release,
                setup.secret,
            ));
        }
        (None, None) => {}
    }
    Ok(ExitCode::SUCCESS)
}

/// The top of the Git repository `folder` is in, if it is in one.
fn repository_root(folder: &Path) -> Option<PathBuf> {
    let output = process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(folder)
        .stderr(process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let root = String::from_utf8(output.stdout).ok()?;
    Some(Path::new(root.trim()).components().collect())
}

/// The app build script named, or the one found in the repository, asked for when there are
/// several and a person to ask.
fn ci_script(named: Option<&Path>, root: Option<&Path>) -> Result<PathBuf, ExitCode> {
    if let Some(script) = named {
        show::field("script", &script.display().to_string());
        return Ok(script.to_path_buf());
    }
    let root = root.ok_or_else(|| {
        fail(
            &"this is not a Git repository; run it in the game's, or name the script with --script",
            REFUSED,
        )
    })?;
    let found = ci::app_scripts(root)
        .map_err(|error| fail(&format!("{}: {error}", root.display()), FAILED))?;
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    match found.as_slice() {
        [] => Err(fail(
            &format!(
                "no app build script in {}; name one with --script",
                root.display()
            ),
            REFUSED,
        )),
        [only] => {
            show::field("script", &format!("{}, found", shown(only)));
            Ok(only.clone())
        }
        several if io::stdin().is_terminal() => {
            for (number, script) in several.iter().enumerate() {
                show::field(&number.saturating_add(1).to_string(), &shown(script));
            }
            let picked = typing::ask("script", Echo::Typed)
                .map_err(|error| fail(&format!("the script: {error}"), FAILED))?;
            picked
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|number| several.get(number.checked_sub(1)?))
                .cloned()
                .ok_or_else(|| fail(&"pick a script by its number", REFUSED))
        }
        several => {
            let listed: Vec<String> = several.iter().map(|script| shown(script)).collect();
            Err(fail(
                &format!(
                    "several app build scripts, {}; name one with --script",
                    listed.join(", ")
                ),
                REFUSED,
            ))
        }
    }
}

/// The GitHub repository named, or the one the script's repository's `origin` points at.
fn ci_repository(named: Option<&str>, script: &Path) -> Result<String, ExitCode> {
    if let Some(named) = named {
        let repository =
            ci::github_repository(&format!("https://github.com/{named}")).ok_or_else(|| {
                fail(
                    &format!("{named} is not a repository's owner/name"),
                    REFUSED,
                )
            })?;
        show::field("repo", &format!("{repository}, as named"));
        return Ok(repository);
    }
    let folder = script
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let origin = process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(folder)
        .stderr(process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    let repository = origin
        .as_deref()
        .and_then(ci::github_repository)
        .ok_or_else(|| {
            fail(
                &"the script's repository has no GitHub origin; name one with --repo owner/name, \
                  or write the login to a file with --output",
                REFUSED,
            )
        })?;
    show::field("repo", &format!("{repository}, from origin"));
    Ok(repository)
}

/// Sets `packed` as the secret `secret` of `repository` with the GitHub command line, which
/// reads it from its input, so that it is never on a command line or on the screen.
fn set_secret(repository: &str, secret: &str, packed: &ci::Login) -> Result<(), ExitCode> {
    let spinner = Spinner::start(
        "secret",
        &format!("setting {secret} on {repository}"),
        false,
    );
    let started = process::Command::new("gh")
        .args(["secret", "set", secret, "--repo", repository])
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::piped())
        .spawn();
    let mut child = match started {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            spinner.failed("gh is not installed");
            let install = Hint {
                before: "install it from ",
                command: "https://cli.github.com",
                after: ", or write the login to a file with --output",
            };
            show::failure("the GitHub command line is needed", "", Some(install));
            return Err(ExitCode::from(FAILED));
        }
        Err(error) => {
            spinner.failed("gh could not start");
            return Err(fail(&format!("gh: {error}"), FAILED));
        }
    };
    let written = child.stdin.take().map_or(Ok(()), |mut input| {
        io::Write::write_all(&mut input, packed.packed().as_bytes())
    });
    let finished = child.wait_with_output();
    match (written, finished) {
        (Ok(()), Ok(output)) if output.status.success() => {
            spinner.done(&format!("{secret} set on {repository}"));
            Ok(())
        }
        (_, Ok(output)) => {
            spinner.failed("not set");
            let said = String::from_utf8_lossy(&output.stderr);
            let login = Hint {
                before: "if gh is not logged in, run ",
                command: "gh auth login",
                after: "",
            };
            show::failure("gh could not set the secret", said.trim(), Some(login));
            Err(ExitCode::from(FAILED))
        }
        (_, Err(error)) => {
            spinner.failed("not set");
            Err(fail(&format!("gh: {error}"), FAILED))
        }
    }
}

/// The commit this release of steamship's action is at, and the release, for a step pinned by
/// commit; the release's tag alone when GitHub cannot be asked.
fn pinned_action() -> (String, String) {
    let release = format!("v{}", env!("CARGO_PKG_VERSION"));
    let commit = process::Command::new("gh")
        .args([
            "api",
            &format!("repos/Aureliolo/steamship/commits/{release}"),
            "--jq",
            ".sha",
        ])
        .stderr(process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    match commit {
        Some(commit) => (commit, release),
        None => (release.clone(), release),
    }
}
