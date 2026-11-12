//! `steamship login`: the build account logged in at a terminal, and the Web API key kept.

use std::io::{self, IsTerminal as _};
use std::path::Path;
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED};
use steamship::install;
use steamship::login::{self, Ending, Person};
use steamship::platform::Platform;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
#[cfg(unix)]
use steamship::unix::Terminal;
#[cfg(windows)]
use steamship::windows::Terminal;
use steamship::{conversation, keychain, steamcmd, webapi};
use zeroize::Zeroizing;

use super::exit::fail;
use super::key::{WHERE_KEY, check_key, keep_key, key_variable, typed_key};
use super::ready::{Source, account, home, ready, steamcmd_failed};

/// What steamship shows while it waits for the login to be approved.
const APPROVING: &str = "waiting for you in the Steam Mobile app";

/// The person at the terminal, who answers steamcmd through steamship's own prompts.
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

pub(crate) fn try_login(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("login");
    let (account, source) = account(named, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    let (held, home, manifest) = ready()?;
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
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
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
            "uploads use the saved login until it expires. steamship logout forgets it",
        );
    } else {
        show::success("logged in", "uploads use this login until it expires");
    }
    Ok(ExitCode::SUCCESS)
}

/// `login --web-api-key`: asks for the key, has Steam check it, and keeps it.
pub(crate) fn try_key_login() -> Result<ExitCode, ExitCode> {
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
        "steamship builds and the other Web API commands use it. steamship logout forgets it",
    );
    Ok(ExitCode::SUCCESS)
}

/// At the end of `login`, at a terminal: offers to keep the Web API key too, when none is set or
/// kept and there is a store to keep it in. Only Enter skips it. Nothing here fails the login.
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
    show::aside("steamship builds and the other Web API commands need the publisher key");
    show::aside(&format!("{WHERE_KEY}. Enter skips it"));
    let key = match typed_key() {
        Ok(Some(key)) => key,
        Ok(None) => {
            show::field(
                "api key",
                "skipped (steamship login --web-api-key keeps one)",
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
