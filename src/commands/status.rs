//! `steamship status`: what is set up, and whether Steam takes the saved login.

use std::path::Path;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{FAILED, LOGIN, REFUSED};
use steamship::install::{self, State};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
#[cfg(windows)]
use steamship::windows;
use steamship::{ci, conversation, dump, keychain, steamcmd, webapi};

use super::exit::fail;
use super::key::key_variable;
use super::ready::{dump_in_actions, home, packed_login, ready, restore, steamcmd_failed, verdict};

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
            "an earlier pin, which the next login or upload replaces",
        ),
        Ok(State::Missing) => {
            show::field("steamcmd", "not installed (steamship login installs it)");
        }
        Err(error) => {
            show::failed("steamcmd", "differs from the pinned version");
            return Err(steamcmd_failed(&error));
        }
    }
    Ok(())
}

pub(crate) fn try_status(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("status");
    show::field("version", env!("CARGO_PKG_VERSION"));
    let home = home()?;
    show::field("home", &home.display().to_string());
    #[cfg(windows)]
    show::field(
        "console",
        &match windows::console() {
            Ok(()) => "Microsoft's ConPTY".to_owned(),
            Err(reason) => format!("Windows' own ({reason})"),
        },
    );
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
    let (judged, redactor) = check_saved_login(&account, packed.as_ref())?;
    if judged == conversation::Login::Taken {
        show::success("ready to upload", "");
    }
    Ok(verdict(judged, &redactor, packed.as_ref()))
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
pub(crate) fn check_saved_login(
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<(conversation::Login, Redactor), ExitCode> {
    let (held, home, manifest) = ready()?;
    restore(packed, &home)?;
    match packed.and_then(ci::Login::packed_on) {
        Some(day) => show::field("login", &format!("from {}, packed {day}", ci::VARIABLE)),
        None => show::field("login", "saved"),
    }
    let before = Redactor::for_home(&home)
        .map_err(|error| fail(&error, FAILED))?
        .and_name(account.name());
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "checking the saved login", true);
    let finished = match steamcmd::check_login(account).run(&home) {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&error, FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    // Judged as steamcmd wrote it: an account named `limit` must not turn a refusal into another.
    let judged =
        conversation::judge_login(finished.code, &String::from_utf8_lossy(&finished.output));
    let took = show::took(spinner.elapsed());
    match &judged {
        conversation::Login::Taken => spinner.done("Steam takes the saved login"),
        conversation::Login::Refused(_) => spinner.failed(&format!("refused after {took}")),
        conversation::Login::Failed(_) => {
            spinner.failed("could not check");
            dump_in_actions(
                &[],
                &[("steamcmd's console".to_owned(), console)],
                &marks.added(&steamcmd_logs),
                &redactor,
            );
        }
    }
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok((judged, redactor))
}
