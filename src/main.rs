//! The command line.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{self, Path, PathBuf};
use std::process::{self, ExitCode};
use std::thread;
use std::time::{Duration, SystemTime};

use clap::{CommandFactory as _, FromArgMatches as _};
use steamship::account::Account;
use steamship::cli::{Cli, Command, FAILED, LOGIN, REFUSED, STEAMCMD};
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
use steamship::{
    achievements, check, ci, conversation, dump, init, keychain, leaderboards, presence, run,
    scripts, settings, steamcmd, upload, vdf, webapi, workshop,
};
use zeroize::Zeroizing;

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
    // Completion is text for a shell to load, often at every start, so it asks nothing online.
    let update = if matches!(command, Command::Completions { .. }) {
        None
    } else {
        update_check()
    };
    let code = run(command);
    if let Some(latest) = update.and_then(update::Check::newer) {
        show::upgrade(
            &latest.to_string(),
            env!("CARGO_PKG_VERSION"),
            installed().hint(),
        );
    }
    code
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "steamship", &mut io::stdout());
            ExitCode::SUCCESS
        }
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
        Command::Init {
            app,
            depots,
            folder,
        } => run_init(app, depots, &folder),
        Command::Builds { app, count } => match try_builds(&app, count) {
            Ok(code) | Err(code) => code,
        },
        Command::Achievements { app, check } => match try_achievements(&app, check.as_deref()) {
            Ok(code) | Err(code) => code,
        },
        Command::Settings {
            app,
            save,
            check,
            account,
        } => match try_settings(
            &app,
            save.as_deref(),
            check.as_deref(),
            named(account.as_deref()),
        ) {
            Ok(code) | Err(code) => code,
        },
        Command::Leaderboards { app, check, create } => {
            match try_leaderboards(&app, check.as_deref(), create) {
                Ok(code) | Err(code) => code,
            }
        }
        Command::RichPresence {
            app,
            files,
            preview,
        } => match try_rich_presence(&app, &files, preview) {
            Ok(code) | Err(code) => code,
        },
        Command::Promote { app, build, branch } => match try_promote(&app, build, &branch) {
            Ok(code) | Err(code) => code,
        },
        Command::Branch {
            app,
            branch,
            description,
        } => match try_branch(&app, &branch, &description) {
            Ok(code) | Err(code) => code,
        },
        Command::Workshop {
            script,
            account,
            new,
        } => match try_workshop(&script, named(account.as_deref()), new) {
            Ok(code) | Err(code) => code,
        },
        Command::Login {
            web_api_key: true, ..
        } => match try_key_login() {
            Ok(code) | Err(code) => code,
        },
        Command::Login { account, .. } => match try_login(named(account.as_deref())) {
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
    }
}

/// How this steamship was installed, which is how it is upgraded.
fn installed() -> Installed {
    // Homebrew and winget run steamship through a link to where they keep it.
    env::current_exe().map_or(Installed::Archive, |program| {
        Installed::of(&fs::canonicalize(&program).unwrap_or(program), Path::exists)
    })
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

fn run_init(app: u32, mut depots: Vec<init::Depot>, folder: &Path) -> ExitCode {
    show::title("init");
    if depots.is_empty() {
        let Some(id) = app.checked_add(1) else {
            return fail(
                &format!("app {app} leaves no ID for a depot after it"),
                REFUSED,
            );
        };
        depots.push(init::Depot {
            id,
            folder: "build".to_owned(),
        });
    }
    let scripts = match init::scripts(app, &depots, folder) {
        Ok(scripts) => scripts,
        Err(why) => return fail(&why, REFUSED),
    };
    let paths: Vec<(PathBuf, String)> = scripts
        .into_iter()
        .map(|(name, text)| (folder.join(name), text))
        .collect();
    // All or nothing: a script already there is someone's, and half a set would not check.
    if let Some((there, _)) = paths.iter().find(|(path, _)| path.exists()) {
        return fail(
            &format!("{} is already there, and is left as it is", there.display()),
            REFUSED,
        );
    }
    if let Err(error) = fs::create_dir_all(folder) {
        return fail(&format!("{}: {error}", folder.display()), FAILED);
    }
    for (path, text) in &paths {
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| io::Write::write_all(&mut file, text.as_bytes()));
        if let Err(error) = written {
            return fail(&format!("{}: {error}", path.display()), FAILED);
        }
        show::field("wrote", &path.display().to_string());
    }
    let app_script = folder.join("app_build.vdf");
    show::success(
        "scripts written",
        &format!(
            "put the build in each depot's folder, then check it with steamship check {}",
            app_script.display()
        ),
    );
    ExitCode::SUCCESS
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
    show::failure(&error.to_string(), "", None);
    if let Some(remedy) = error.remedy() {
        show::note(remedy);
        show::hint(installed().hint());
    }
    ExitCode::from(code)
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
    let key = home().and_then(|home| {
        keychain::forget(&home).or_else(|error| match error {
            keychain::Error::Unavailable(_) => Ok(false),
            keychain::Error::Failed(_) | keychain::Error::NotAKey => {
                show::failed("api key", "not forgotten");
                Err(fail(&error, FAILED))
            }
        })
    });
    match key {
        Ok(true) => show::done("api key", "forgotten"),
        Ok(false) => show::field("api key", "none kept"),
        Err(code) => return code,
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
    let login = ci::Login::unpack(text).map_err(|error| fail(&error, REFUSED))?;
    // GitHub masks the secret, but not the account's name inside it, which steamcmd's logs name.
    if dump::in_actions(|name| env::var(name).ok()) {
        show::plain(&format!("::add-mask::{}\n", login.account().name()));
    }
    Ok(Some(login))
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
    status_key(&home);
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
    Ok(verdict(judged, packed.as_ref()))
}

/// Where the Web API commands would find the key, found out without unlocking anything.
fn status_key(home: &Path) {
    if key_variable().is_some() {
        show::field("api key", &format!("from {}", webapi::KEY));
        return;
    }
    match keychain::has(home) {
        Ok(true) => show::field("api key", &format!("kept in {}", keychain::STORE)),
        Ok(false) => show::field("api key", "none kept"),
        Err(keychain::Error::Unavailable(_)) => show::field("api key", "no credential store here"),
        Err(error @ (keychain::Error::Failed(_) | keychain::Error::NotAKey)) => {
            show::failed("api key", &error.to_string());
        }
    }
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
    match packed.map(ci::Login::packed_on) {
        Some(Some(day)) => show::field("login", &format!("from {}, packed {day}", ci::VARIABLE)),
        Some(None) => show::field("login", &format!("from {}", ci::VARIABLE)),
        None => show::field("login", "saved"),
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
        steamcmd::HOPELESS,
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

/// Says Steam refused the login, and how to log in again: for one handed over in
/// `STEAMSHIP_LOGIN`, with the day it was packed, since the token in it may simply have expired.
fn login_refused(reason: &str, packed: Option<&ci::Login>) -> ExitCode {
    match packed {
        None => {
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", reason, Some(again));
        }
        Some(packed) => {
            let from = packed.packed_on().map_or_else(
                || {
                    format!(
                        "the login in {} was packed by an earlier steamship",
                        ci::VARIABLE
                    )
                },
                |day| format!("the login in {} was packed on {day}", ci::VARIABLE),
            );
            let again = Hint {
                before: "log in again with ",
                command: "steamship login",
                after: ", then run steamship ci to pack the new login",
            };
            show::failure("not logged in", &format!("{reason}; {from}"), Some(again));
        }
    }
    ExitCode::from(LOGIN)
}

/// The exit code for what Steam made of the saved login, or the one handed over in `packed`,
/// saying why when it did not take it.
fn verdict(judged: upload::Login, packed: Option<&ci::Login>) -> ExitCode {
    match judged {
        upload::Login::Taken => ExitCode::SUCCESS,
        upload::Login::Refused(reason) => login_refused(&reason, packed),
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
    offer_key(&home);
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
        if let (Some(app), Some(branch)) = (report.app_id, report.set_live.as_deref()) {
            live_branch_found(script, app, branch)?;
        }
        return Ok(report);
    }
    for problem in &report.problems {
        show::failure("refused", &problem.to_string(), None);
    }
    Err(ExitCode::from(REFUSED))
}

/// Why an unchecked `SetLive` branch matters, and how to have it checked.
const UNCHECKED_BRANCH: &str = "a branch the app does not have leaves the build uploaded but not \
                                live; steamship login --web-api-key keeps a key to check it with";

/// Refuses a `SetLive` branch the app does not have, which Valve would otherwise report only
/// after the whole upload. Steam is asked only with a key already at hand, never one typed for
/// the purpose, and when it cannot answer the upload is not held up by it.
fn live_branch_found(script: &Path, app: u32, branch: &str) -> Result<(), ExitCode> {
    let Some(api) = key_at_hand()? else {
        show::field(
            "branch",
            &format!("{branch}, not checked: no Web API key at hand"),
        );
        show::aside(UNCHECKED_BRANCH);
        return Ok(());
    };
    let spinner = Spinner::start("branch", "asking Steam for the app's branches", false);
    match api.branches(app) {
        Ok(branches)
            if branches
                .iter()
                .any(|found| found.name.eq_ignore_ascii_case(branch)) =>
        {
            spinner.done(&format!("{branch}, found on Steam"));
            Ok(())
        }
        Ok(branches) => {
            spinner.failed("not on Steam");
            let problem = scripts::Problem::new(script, upload::no_branch(app, branch, &branches));
            show::failure("refused", &problem.to_string(), None);
            Err(ExitCode::from(REFUSED))
        }
        Err(error) => {
            spinner.failed("not checked");
            show::aside(&error.to_string());
            if let Some(why) = why_unanswered(&api, app, &error) {
                show::aside(&why);
            }
            Ok(())
        }
    }
}

/// The partner Web API, when a key is set in `STEAMSHIP_WEB_API_KEY` or kept; none otherwise,
/// without asking for one.
fn key_at_hand() -> Result<Option<webapi::Api>, ExitCode> {
    if let Some(text) = key_variable() {
        let key = webapi::Key::parse(&text).map_err(|_not_a_key| {
            fail(
                &format!(
                    "{} does not hold a publisher Web API key, which is 32 hexadecimal digits",
                    webapi::KEY
                ),
                REFUSED,
            )
        })?;
        return Ok(Some(webapi::Api::new(key)));
    }
    // Asked first without reading it, which no store prompts for; a store holding none is then
    // never asked to unlock, so a check stays free of prompts for anyone who kept no key.
    let kept = Platform::THIS
        .home(|name| env::var_os(name))
        .ok()
        .filter(|home| keychain::has(home).unwrap_or(false))
        .and_then(|home| keychain::kept(&home).ok().flatten());
    Ok(kept.map(webapi::Api::new))
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
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", doing, true);
    let finished = run::run(
        &program,
        &steamcmd::upload(&account, &prepared.script),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::UPLOAD_LIMIT,
        steamcmd::HOPELESS,
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
    let outcome = found_on_steam(outcome, &prepared, &description);
    let code = report(&outcome, spinner, &took, &prepared, &saved, packed.as_ref());
    if matches!(
        outcome,
        upload::Outcome::Failed(_) | upload::Outcome::BuiltThenFailed { .. }
    ) {
        let mut files = vec![
            ("steamcmd's console".to_owned(), saved),
            (format!("app_build_{}.log", prepared.app_id), prepared.log()),
        ];
        files.extend(depot_logs(&prepared.output));
        dump_in_actions(&files, &marks.added(&steamcmd_logs), &redactor);
    }
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

/// Asks Steam, when a key is at hand, whether `build_id` is what is live on `branch` now; false
/// only when Steam shows another build there, or no such branch. Asked a few times, since a
/// build just set live may not show at once; a Steam that cannot be asked holds nothing up.
fn confirmed_live(app: u32, branch: &str, build_id: u64) -> bool {
    const ASKS: u32 = 4;
    const APART: Duration = Duration::from_secs(2);
    let Ok(Some(api)) = key_at_hand() else {
        return true;
    };
    let spinner = Spinner::start("branch", "asking Steam what is live", false);
    let ask = || {
        api.branches(app)
            .map(|branches| upload::live_now(&branches, branch, build_id))
    };
    let mut seen = ask();
    for _ in 1..ASKS {
        if !matches!(seen, Ok(upload::Live::Other(_) | upload::Live::Missing)) {
            break;
        }
        thread::sleep(APART);
        seen = ask();
    }
    match seen {
        Ok(upload::Live::Confirmed) => {
            spinner.done(&format!("{branch}: BuildID {build_id}, as Steam shows it"));
            true
        }
        Ok(upload::Live::Other(other)) => {
            spinner.failed(&format!("{branch}: BuildID {other}, as Steam shows it"));
            false
        }
        Ok(upload::Live::Missing) => {
            spinner.failed(&format!("{branch}: no such branch on Steam"));
            false
        }
        Err(error) => {
            spinner.failed("not confirmed");
            show::aside(&error.to_string());
            true
        }
    }
}

/// A build Steam kept but set live nowhere, found by its description when a key is at hand, as
/// the build it is: steamcmd reports no build ID when Steam refuses to set one live.
fn found_on_steam(
    outcome: upload::Outcome,
    prepared: &upload::Prepared,
    description: &str,
) -> upload::Outcome {
    let upload::Outcome::Failed(reasons) = &outcome else {
        return outcome;
    };
    if upload::commit_refused(reasons, prepared.set_live.as_deref()).is_none() {
        return outcome;
    }
    let Ok(Some(api)) = key_at_hand() else {
        return outcome;
    };
    // Enough to reach past uploads made while this one ran.
    let found = api
        .builds(prepared.app_id, 10)
        .ok()
        .and_then(|builds| upload::uploaded_as(&builds, description));
    let Some(build_id) = found else {
        return outcome;
    };
    upload::Outcome::BuiltThenFailed {
        build_id,
        reasons: reasons.clone(),
    }
}

fn report(
    outcome: &upload::Outcome,
    spinner: Spinner,
    took: &str,
    prepared: &upload::Prepared,
    saved: &Path,
    packed: Option<&ci::Login>,
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
            hand_on(*build_id);
            let Some(branch) = &prepared.set_live else {
                show::success(&format!("app {app}: BuildID {build_id}"), &logs);
                return ExitCode::SUCCESS;
            };
            if !confirmed_live(app, branch, *build_id) {
                show::failure(
                    &format!(
                        "app {app}: BuildID {build_id} uploaded, but Steam does not show it live \
                         on {branch}"
                    ),
                    "",
                    Some(Hint {
                        before: "set it live with ",
                        command: &format!(
                            "steamship promote {app} --build {build_id} --branch {branch}"
                        ),
                        after: "",
                    }),
                );
                show::note(&logs);
                return ExitCode::from(FAILED);
            }
            show::success(
                &format!("app {app}: BuildID {build_id}, set live on {branch}"),
                &logs,
            );
            ExitCode::SUCCESS
        }
        upload::Outcome::Previewed => {
            spinner.done(&format!("preview finished in {took}"));
            show::success("nothing was uploaded, as asked", &logs);
            ExitCode::SUCCESS
        }
        upload::Outcome::NotLoggedIn(line) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(line, packed)
        }
        upload::Outcome::BuiltThenFailed { build_id, reasons } => {
            spinner.failed(&format!("failed after {took}"));
            show::failure(
                &format!("app {app}: built as BuildID {build_id}, then steamcmd failed"),
                "",
                None,
            );
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(&why);
            }
            if let Some(branch) = &prepared.set_live {
                show::hint(Hint {
                    before: "set it live with ",
                    command: &format!(
                        "steamship promote {app} --build {build_id} --branch {branch}"
                    ),
                    after: "",
                });
            }
            show::note(&logs);
            hand_on(*build_id);
            ExitCode::from(FAILED)
        }
        upload::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(upload::NOT_LIVE);
                show::note(&why);
                show::hint(Hint {
                    before: "see the app's branches with ",
                    command: &format!("steamship builds {app}"),
                    after: "",
                });
            }
            show::note(&logs);
            ExitCode::from(FAILED)
        }
    }
}

/// Steam's logs of a failed run, as collapsed groups in a GitHub Actions job's log: `files` as
/// they are, then what steamcmd added to its own logs this run, all of it redacted. The action
/// deletes steamship's home when the job ends, so this is the only look at them there is.
fn dump_in_actions(files: &[(String, PathBuf)], added: &[(String, Vec<u8>)], redactor: &Redactor) {
    if !dump::in_actions(|name| env::var(name).ok()) {
        return;
    }
    let token = dump::token();
    let mut shown = String::new();
    let read = files
        .iter()
        .filter_map(|(title, path)| Some((title.clone(), fs::read(path).ok()?)));
    let steamcmds = added
        .iter()
        .map(|(name, bytes)| (format!("steamcmd's {name}, from this run"), bytes.clone()));
    for (title, bytes) in read.chain(steamcmds) {
        let text = redactor.redact(&bytes);
        shown.push_str(&dump::group(
            &title,
            &String::from_utf8_lossy(&text),
            &token,
        ));
    }
    show::plain(&shown);
}

/// Each depot's build log in `output`, by name, in name order.
fn depot_logs(output: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(output)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let log = Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
            (name.starts_with("depot_build_") && log).then(|| (name, entry.path()))
        })
        .collect();
    found.sort();
    found
}

fn run_check(script: &Path) -> ExitCode {
    show::title("check");
    show::field("script", &script.display().to_string());
    match checked(script) {
        Ok(report) => {
            for depot in &report.depots {
                let symbols = match depot.debug_symbols {
                    0 => String::new(),
                    count => format!(", {count} of them debug symbols"),
                };
                show::field(
                    "depot",
                    &format!(
                        "{}, {}{symbols}",
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

/// The key set in `STEAMSHIP_WEB_API_KEY`, where an empty one is none, as for the account.
fn key_variable() -> Option<Zeroizing<String>> {
    env::var(webapi::KEY)
        .ok()
        .filter(|text| !text.is_empty())
        .map(Zeroizing::new)
}

/// Where Steamworks shows the key, said before it is asked for.
const WHERE_KEY: &str = "the publisher Web API key is in Steamworks under Users & Permissions, \
                         Manage Groups: the group that holds the app";

/// How the Web API key for a command was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyFrom {
    Variable,
    Kept,
    Typed,
}

/// The partner Web API, with the key from `STEAMSHIP_WEB_API_KEY`, or else the one kept in the
/// credential store, or else, at a terminal, one typed in.
fn web_api(home: &Path) -> Result<(webapi::Api, KeyFrom), ExitCode> {
    if let Some(text) = key_variable() {
        let key = webapi::Key::parse(&text).map_err(|_not_a_key| {
            fail(
                &format!(
                    "{} does not hold a publisher Web API key, which is 32 hexadecimal digits",
                    webapi::KEY
                ),
                REFUSED,
            )
        })?;
        show::field("api key", &format!("from {}", webapi::KEY));
        return Ok((webapi::Api::new(key), KeyFrom::Variable));
    }
    match keychain::kept(home) {
        Ok(Some(key)) => {
            show::field("api key", &format!("kept in {}", keychain::STORE));
            return Ok((webapi::Api::new(key), KeyFrom::Kept));
        }
        Ok(None) | Err(keychain::Error::Unavailable(_)) => {}
        Err(error @ (keychain::Error::Failed(_) | keychain::Error::NotAKey)) => {
            show::failed("api key", "not read");
            return Err(fail(&error, FAILED));
        }
    }
    if !io::stdin().is_terminal() {
        let keep = Hint {
            before: "run ",
            command: "steamship login --web-api-key",
            after: " at a terminal to keep one, or set STEAMSHIP_WEB_API_KEY",
        };
        show::failure("no Web API key", "", Some(keep));
        return Err(ExitCode::from(REFUSED));
    }
    show::aside(WHERE_KEY);
    let key = typed_key()?.ok_or_else(|| fail(&"no Web API key typed", REFUSED))?;
    Ok((webapi::Api::new(key), KeyFrom::Typed))
}

/// The key typed at the prompt, shown as dots; none when only Enter was pressed.
fn typed_key() -> Result<Option<webapi::Key>, ExitCode> {
    let typed = typing::ask("api key", Echo::Dots)
        .map_err(|error| fail(&format!("the Web API key: {error}"), FAILED))?;
    if typed.trim().is_empty() {
        return Ok(None);
    }
    webapi::Key::parse(&typed).map(Some).map_err(|_not_a_key| {
        fail(
            &"that is not a publisher Web API key, which is 32 hexadecimal digits",
            REFUSED,
        )
    })
}

/// Has Steam list the apps the key reaches, the one check a key can be given, and shows them.
fn check_key(api: &webapi::Api) -> Result<(), ExitCode> {
    let spinner = Spinner::start("steam", "checking the Web API key", false);
    match api.apps() {
        Ok(apps) => {
            spinner.done(&format!(
                "Steam takes the key, for {}",
                show::counted(apps.len(), "app")
            ));
            for app in &apps {
                show::field(&app.app_id.to_string(), &app.name);
            }
            Ok(())
        }
        Err(error) => Err(web_api_failed(spinner, &error)),
    }
}

/// Keeps `key` for `home` in the credential store, and says whether it did; why not is shown.
fn keep_key(home: &Path, key: &webapi::Key) -> bool {
    match keychain::keep(home, key) {
        Ok(()) => {
            show::done("api key", &format!("kept in {}", keychain::STORE));
            for line in keychain::KEPT_NOTE {
                show::aside(line);
            }
            true
        }
        Err(error) => {
            show::failed("api key", "not kept");
            show::failure(&error.to_string(), "", None);
            false
        }
    }
}

/// `login --web-api-key`: asks for the key, has Steam check it, and keeps it.
fn try_key_login() -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("login");
    let home = home()?;
    show::aside(WHERE_KEY);
    let key = typed_key()?.ok_or_else(|| fail(&"no Web API key typed", REFUSED))?;
    let api = webapi::Api::new(key);
    check_key(&api)?;
    if !keep_key(&home, api.key()) {
        return Err(ExitCode::from(FAILED));
    }
    show::success(
        "Web API key kept",
        "`steamship builds` and the other Web API commands use it; `steamship logout` forgets it",
    );
    Ok(ExitCode::SUCCESS)
}

/// At the end of `login`, at a terminal: offers to keep the Web API key too, when none is set or
/// kept and there is a store to keep it in. Only Enter skips it; nothing here fails the login.
fn offer_key(home: &Path) {
    if !io::stdin().is_terminal() || key_variable().is_some() {
        return;
    }
    match keychain::has(home) {
        Ok(false) => {}
        Ok(true) => {
            show::field("api key", &format!("kept in {}", keychain::STORE));
            return;
        }
        Err(_) => return,
    }
    show::aside("`steamship builds` and the other Web API commands need the publisher key;");
    show::aside(&format!("{WHERE_KEY}. Enter skips it"));
    let key = match typed_key() {
        Ok(Some(key)) => key,
        Ok(None) => {
            show::field(
                "api key",
                "skipped; `steamship login --web-api-key` keeps one",
            );
            return;
        }
        Err(_) => return,
    };
    let api = webapi::Api::new(key);
    if check_key(&api).is_ok() {
        let _kept = keep_key(home, api.key());
    }
}

/// After a command that worked with a key typed at its prompt, offers to keep that key.
fn offer_to_keep(home: &Path, api: &webapi::Api, from: KeyFrom) {
    if from != KeyFrom::Typed {
        return;
    }
    show::aside(&format!(
        "Enter keeps the key in {} for next time; n does not",
        keychain::STORE
    ));
    let Ok(answer) = typing::ask("keep it", Echo::Typed) else {
        return;
    };
    if matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ) {
        let _kept = keep_key(home, api.key());
    } else {
        show::field("api key", "not kept");
    }
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

/// Why Steam gave `error` about `app`, when the apps the key holds can say: asked only for the
/// one error that needs them.
fn why_unanswered(api: &webapi::Api, app: u32, error: &webapi::Error) -> Option<String> {
    if !matches!(error, webapi::Error::Status(500, _)) {
        return None;
    }
    webapi::no_build_yet(error, app, &api.apps().ok()?)
}

/// Says why the Web API did not answer, and the exit code for it.
fn web_api_failed(spinner: Spinner, error: &webapi::Error) -> ExitCode {
    spinner.failed("no answer");
    fail(error, FAILED)
}

fn try_builds(app: &str, count: u32) -> Result<ExitCode, ExitCode> {
    show::title("builds");
    let app_id = app_named(app)?;
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "asking for the branches and builds", false);
    let overview = match api.overview(app_id, count) {
        Ok(overview) => overview,
        Err(error) => {
            let why = why_unanswered(&api, app_id, &error);
            let code = web_api_failed(spinner, &error);
            if let Some(why) = why {
                show::note(&why);
            }
            return Err(code);
        }
    };
    spinner.done(&overview.summary());
    for (label, line) in overview.lines() {
        show::field(&label, &line);
    }
    offer_to_keep(&home, &api, from);
    Ok(ExitCode::SUCCESS)
}

/// The achievements file at `path`, read, and refused when it is not one or names another app
/// than `app`: before Steam is asked anything.
fn achievements_file(path: &Path, app: u32) -> Result<achievements::Listed, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let listed = achievements::read(&text).map_err(|why| refused(&why))?;
    if let Some(other) = listed.app.filter(|named| *named != u64::from(app)) {
        return Err(refused(&format!("is for app {other}, not app {app}")));
    }
    let mut holds = show::counted(listed.achievements.len(), "achievement");
    if let Some(stats) = &listed.stats {
        holds = format!("{holds} and {}", show::counted(stats.len(), "stat"));
    }
    show::field("file", &format!("{}, {holds}", path.display()));
    Ok(listed)
}

fn try_achievements(app: &str, check: Option<&Path>) -> Result<ExitCode, ExitCode> {
    show::title("achievements");
    let app_id = app_named(app)?;
    let listed = check
        .map(|path| achievements_file(path, app_id))
        .transpose()?;
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "asking for the achievements", false);
    let held = match api.schema(app_id) {
        Ok(held) => held,
        Err(error) => return Err(web_api_failed(spinner, &error)),
    };
    let mut holds = show::counted(held.achievements.len(), "achievement");
    if !held.stats.is_empty() {
        holds = format!("{holds} and {}", show::counted(held.stats.len(), "stat"));
    }
    spinner.done(&format!("{holds} on Steam"));
    offer_to_keep(&home, &api, from);
    let Some(listed) = listed else {
        for achievement in &held.achievements {
            show::field(&achievement.api_name, &achievements::line(achievement));
        }
        for stat in &held.stats {
            show::field(
                &format!("stat {}", stat.api_name),
                &achievements::stat_line(stat),
            );
        }
        return Ok(ExitCode::SUCCESS);
    };
    let mut drift = achievements::compare(&listed.achievements, &held.achievements);
    if let Some(stats) = &listed.stats {
        drift.extend(achievements::compare_stats(stats, &held.stats));
    }
    if drift.is_empty() {
        let what = if listed.stats.is_some() {
            "achievements and stats"
        } else {
            "achievements"
        };
        show::success(&format!("Steam's {what} match the file"), "");
        return Ok(ExitCode::SUCCESS);
    }
    for difference in &drift {
        show::failure(&difference.to_string(), "", None);
    }
    Ok(ExitCode::from(REFUSED))
}

/// The rich presence files, each read and checked, refused before Steam is asked anything when
/// one is wrong or two are for the same language.
fn presence_files(files: &[PathBuf]) -> Result<Vec<presence::Language>, ExitCode> {
    let mut languages: Vec<presence::Language> = Vec::with_capacity(files.len());
    for path in files {
        let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
        let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
        let language = presence::read(&text).map_err(|why| refused(&why))?;
        if languages
            .iter()
            .any(|other| other.language == language.language)
        {
            return Err(refused(&format!(
                "is a second file for {}",
                language.language
            )));
        }
        show::field(
            &language.language,
            &format!(
                "{}, from {}",
                show::counted(language.tokens.len(), "token"),
                path.display()
            ),
        );
        languages.push(language);
    }
    Ok(languages)
}

fn try_rich_presence(app: &str, files: &[PathBuf], preview: bool) -> Result<ExitCode, ExitCode> {
    show::title(if preview {
        "rich-presence, preview"
    } else {
        "rich-presence"
    });
    let app_id = app_named(app)?;
    let languages = presence_files(files)?;
    if preview {
        show::success("the files are sound; nothing was sent, as asked", "");
        return Ok(ExitCode::SUCCESS);
    }
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "sending the rich presence", false);
    if let Err(error) = api.set_rich_presence(&presence::request(app_id, &languages)) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("taken");
    offer_to_keep(&home, &api, from);
    let names: Vec<&str> = languages
        .iter()
        .map(|language| language.language.as_str())
        .collect();
    show::success(
        &format!(
            "app {app_id}: rich presence replaced for {}",
            names.join(", ")
        ),
        "each language's tokens on Steam are now exactly its file's",
    );
    Ok(ExitCode::SUCCESS)
}

/// The settings snapshot at `path`, refused before steamcmd is run when it is not `app`'s.
fn settings_file(path: &Path, app: u32) -> Result<vdf::Block, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let wanted = settings::read(&text, app).map_err(|why| refused(&why))?;
    show::field("file", &path.display().to_string());
    Ok(wanted)
}

/// `app`'s settings, as Steam shows them to `account` through steamcmd.
fn asked_settings(
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
) -> Result<vdf::Block, ExitCode> {
    let (home, manifest) = ready()?;
    restore(packed, &home)?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let spinner = Spinner::start("steam", "asking for the settings", true);
    let finished = match run::run(
        &program,
        &steamcmd::app_info(account, app),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::CHECK_LIMIT,
        steamcmd::HOPELESS,
    ) {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    let console = String::from_utf8_lossy(&console);
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    match upload::judge_login(finished.code, &console) {
        upload::Login::Taken => {}
        judged @ (upload::Login::Refused(_) | upload::Login::Failed(_)) => {
            spinner.failed("not logged in");
            return Err(verdict(judged, packed));
        }
    }
    match settings::from_console(&console, app) {
        Ok(shown) => {
            spinner.done("shown");
            Ok(shown)
        }
        Err(why) => {
            spinner.failed("none shown");
            Err(fail(&why, FAILED))
        }
    }
}

fn try_settings(
    app: &str,
    save: Option<&Path>,
    check: Option<&Path>,
    named: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    show::title("settings");
    let app_id = app_named(app)?;
    let wanted = check.map(|path| settings_file(path, app_id)).transpose()?;
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let shown = asked_settings(&account, app_id, packed.as_ref())?;
    let written = settings::snapshot(app_id, &shown).map_err(|why| fail(&why, FAILED))?;
    if let Some(path) = save {
        fs::write(path, &written)
            .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
        show::success(
            &format!("app {app_id}: settings saved to {}", path.display()),
            "`steamship settings --check` compares Steam's with them",
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(wanted) = wanted else {
        anstream::print!("{written}");
        return Ok(ExitCode::SUCCESS);
    };
    let drift = settings::compare(&wanted, &shown);
    if drift.is_empty() {
        show::success("Steam's settings match the file", "");
        return Ok(ExitCode::SUCCESS);
    }
    for difference in &drift {
        show::failure(&difference.to_string(), "", None);
    }
    show::aside(
        "change the app in Steamworks, or `steamship settings --save` the file again if the \
         change was meant",
    );
    Ok(ExitCode::from(REFUSED))
}

fn leaderboards_file(path: &Path, app: u32) -> Result<leaderboards::Listed, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let listed = leaderboards::read(&text).map_err(|why| refused(&why))?;
    if let Some(other) = listed.app.filter(|named| *named != u64::from(app)) {
        return Err(refused(&format!("is for app {other}, not app {app}")));
    }
    show::field(
        "file",
        &format!(
            "{}, {}",
            path.display(),
            show::counted(listed.leaderboards.len(), "leaderboard")
        ),
    );
    Ok(listed)
}

fn try_leaderboards(app: &str, check: Option<&Path>, create: bool) -> Result<ExitCode, ExitCode> {
    show::title("leaderboards");
    let app_id = app_named(app)?;
    let listed = check
        .map(|path| leaderboards_file(path, app_id))
        .transpose()?;
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let asking = Spinner::start("steam", "asking for the leaderboards", false);
    let mut held = match api.leaderboards(app_id) {
        Ok(held) => held,
        Err(error) => return Err(web_api_failed(asking, &error)),
    };
    asking.done(&format!(
        "{} on Steam",
        show::counted(held.len(), "leaderboard")
    ));
    offer_to_keep(&home, &api, from);
    let Some(listed) = listed else {
        for leaderboard in &held {
            show::field(&leaderboard.name, &leaderboards::line(leaderboard));
        }
        return Ok(ExitCode::SUCCESS);
    };
    if create {
        for wanted in &listed.leaderboards {
            if held
                .iter()
                .any(|leaderboard| leaderboard.name == wanted.name)
            {
                continue;
            }
            let spinner = Spinner::start("steam", &format!("making {}", wanted.name), false);
            match api.create_leaderboard(app_id, wanted) {
                Ok(made) => {
                    spinner.done(&format!("made {}", made.name));
                    held.push(made);
                }
                Err(error) => return Err(web_api_failed(spinner, &error)),
            }
        }
    }
    let drift = leaderboards::compare(&listed.leaderboards, &held);
    if drift.is_empty() {
        show::success("Steam's leaderboards match the file", "");
        return Ok(ExitCode::SUCCESS);
    }
    for difference in &drift {
        show::failure(&difference.to_string(), "", None);
    }
    if drift
        .iter()
        .any(|difference| matches!(difference, leaderboards::Drift::Differs { .. }))
    {
        show::aside(
            "Steam keeps a leaderboard's settings: change them in Steamworks, under Stats & \
             Achievements, Leaderboards",
        );
    }
    Ok(ExitCode::from(REFUSED))
}

fn try_branch(app: &str, branch: &str, description: &str) -> Result<ExitCode, ExitCode> {
    show::title("branch");
    let app_id = app_named(app)?;
    if webapi::is_default(branch) {
        return Err(fail(
            &"the default branch's description is set in Steamworks, not by steamship",
            REFUSED,
        ));
    }
    show::field("branch", branch);
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let asking = Spinner::start("steam", "asking for the branches", false);
    let branches = match api.branches(app_id) {
        Ok(branches) => branches,
        Err(error) => {
            let why = why_unanswered(&api, app_id, &error);
            let code = web_api_failed(asking, &error);
            if let Some(why) = why {
                show::note(&why);
            }
            return Err(code);
        }
    };
    let Some(found) = branches.iter().find(|found| found.name == branch) else {
        asking.failed("not on Steam");
        return Err(fail(
            &format!(
                "app {app_id} has no branch \"{branch}\"; {}",
                upload::create_first(&branches)
            ),
            REFUSED,
        ));
    };
    if found.description.is_empty() {
        asking.done("no description");
    } else {
        asking.done(&format!("described as \"{}\"", found.description));
    }
    offer_to_keep(&home, &api, from);
    if found.description == description {
        show::success(
            &format!("app {app_id}: {branch} is already described so"),
            "",
        );
        return Ok(ExitCode::SUCCESS);
    }
    let setting = Spinner::start("steam", "setting the description", false);
    if let Err(error) = api.describe_branch(app_id, branch, description) {
        return Err(web_api_failed(setting, &error));
    }
    setting.done("set");
    show::success(
        &format!("app {app_id}: {branch} described as \"{description}\""),
        "",
    );
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
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "setting it live", false);
    if let Err(error) = api.set_live(app_id, build, branch) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("set live");
    offer_to_keep(&home, &api, from);
    show::success(
        &format!("app {app_id}: BuildID {build} live on {branch}"),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

fn try_workshop(script: &Path, named: Option<&str>, new: bool) -> Result<ExitCode, ExitCode> {
    show::title("workshop");
    show::field("script", &script.display().to_string());
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let item = checked_item(script, new)?;
    let (home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let copy =
        workshop::prepare(&home, script, item.app_id).map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "uploading", true);
    let finished = run::run(
        &program,
        &steamcmd::workshop(&account, &copy),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::UPLOAD_LIMIT,
        steamcmd::HOPELESS,
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
            output("published-file-id", published, "the item's ID");
            ExitCode::SUCCESS
        }
        workshop::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(&reason, packed.as_ref())
        }
        workshop::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            // Valve gives the real reason for a Workshop failure there, not on the console.
            show::note(&format!(
                "{log}, and steamcmd's workshop log in {}",
                steamcmd_logs.join("workshop_log.txt").display()
            ));
            dump_in_actions(
                &[("steamcmd's console".to_owned(), saved)],
                &marks.added(&steamcmd_logs),
                &redactor,
            );
            ExitCode::from(FAILED)
        }
    };
    install::verify(&home, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

/// The Workshop item `script` describes, checked, and said.
fn checked_item(script: &Path, new: bool) -> Result<workshop::Item, ExitCode> {
    let item = workshop::check(script).map_err(|problems| {
        for problem in &problems {
            show::failure("refused", &problem.to_string(), None);
        }
        ExitCode::from(REFUSED)
    })?;
    // Nobody adds the new item's ID back to the script from CI, so every run would make another.
    if item.published.is_none() && !new && dump::in_ci(|name| env::var(name).ok()) {
        show::failure(
            "refused",
            &format!(
                "{}: there is no \"publishedfileid\", so every CI run would make a new Workshop \
                 item; add the ID of the item to update, or pass --new to make one",
                script.display()
            ),
            None,
        );
        return Err(ExitCode::from(REFUSED));
    }
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
    Ok(item)
}

/// Hands `build_id` on as the `build-id` output of the GitHub Actions step steamship runs in.
fn hand_on(build_id: u64) {
    output("build-id", build_id, "the BuildID");
}

/// Sets the output `name` of the GitHub Actions step steamship runs in to `value`, through the
/// file GitHub names for a step's outputs. The upload is done by then, so a file that cannot be
/// written is said, as `what`, and does not fail it.
fn output(name: &str, value: u64, what: &str) {
    let Some(outputs) = env::var_os("GITHUB_OUTPUT").filter(|path| !path.is_empty()) else {
        return;
    };
    let written = fs::OpenOptions::new()
        .append(true)
        .open(&outputs)
        .and_then(|mut file| {
            io::Write::write_all(&mut file, format!("{name}={value}\n").as_bytes())
        });
    if let Err(error) = written {
        show::note(&format!(
            "{what} could not be handed to the workflow: {error}"
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
        return Ok(verdict(judged, None));
    }
    let today = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let packed = packed.packed(&ci::day(today));
    let again = "when the login expires, run `steamship login` and `steamship ci` again";
    match (setup.output, repository) {
        (Some(output), _) => {
            ci::write_private(output, packed.as_bytes())
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
            fits_a_secret(&packed)?;
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

/// Refuses a packed login GitHub would not keep in a secret, before `gh` is asked to.
fn fits_a_secret(packed: &str) -> Result<(), ExitCode> {
    if packed.len() <= ci::SECRET_LIMIT {
        return Ok(());
    }
    let output = Hint {
        before: "write it to a file for another CI with ",
        command: "steamship ci --output login.txt",
        after: "",
    };
    show::failure(
        "the packed login is too large for a GitHub secret",
        &format!(
            "{} KB, where a secret holds 48 KB",
            packed.len().div_ceil(1024)
        ),
        Some(output),
    );
    Err(ExitCode::from(FAILED))
}

/// Sets `packed` as the secret `secret` of `repository` with the GitHub command line, which
/// reads it from its input, so that it is never on a command line or on the screen.
fn set_secret(repository: &str, secret: &str, packed: &str) -> Result<(), ExitCode> {
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
        io::Write::write_all(&mut input, packed.as_bytes())
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
