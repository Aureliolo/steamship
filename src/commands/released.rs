//! Whether the build account sees an app unreleased, which holds back what steamship does to a released one.

use std::path::Path;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{FAILED, REFUSED};
use steamship::install;
use steamship::redact::Redactor;
use steamship::show::Spinner;
use steamship::{ci, conversation, release, steamcmd};

use super::exit::fail;
use super::ready::{ready, restore, run_as, steamcmd_failed, verdict};

/// What steamcmd, run from `home` and logged in as `account`, printed of `app`'s info, redacted,
/// with the spinner that said it was `doing` so still turning. Refused when Steam did not take
/// the login.
pub(crate) fn app_info(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
    doing: &str,
) -> Result<(String, Spinner), ExitCode> {
    let before = Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?;
    let spinner = Spinner::start("steam", doing, true);
    let finished = match steamcmd::app_info(account, app).run(home) {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&error, FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?);
    let console = String::from_utf8_lossy(&redactor.redact(&finished.output)).into_owned();
    match conversation::judge_login(finished.code, &console) {
        conversation::Login::Taken => Ok((console, spinner)),
        judged @ (conversation::Login::Refused(_) | conversation::Login::Failed(_)) => {
            spinner.failed("not logged in");
            Err(verdict(judged, packed))
        }
    }
}

/// The default branch, as [`unreleased`] leaves it: a build live on a released app's default
/// branch is the one its players get, and Valve then asks for a Steam Mobile confirmation, given in
/// Steamworks.
pub(crate) const DEFAULT_RELEASED: (&str, &str) = (
    "its default branch is set live in Steamworks, where Valve asks for a Steam Mobile \
     confirmation",
    "its default branch",
);

/// Whether steamship may do what is only safe before players have the app: only while Steam
/// shows `account` that nobody can buy it yet, asked through steamcmd run from `home`. `left`
/// says what happens instead once the app is released, and what steamship leaves to Steamworks
/// when Steam shows no release state. Answers why not when it may not, and fails when Steam could
/// not be asked.
fn unreleased(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
    (released, left): (&str, &str),
) -> Result<Result<(), String>, ExitCode> {
    let (state, spinner) = release_shown(home, account, app, packed)?;
    let (judged, said) = release_judged(state.as_deref(), app, (released, left));
    if judged.is_ok() {
        spinner.done(&said);
    } else {
        spinner.failed(&said);
    }
    Ok(judged)
}

/// Whether Steam's release `state` for `app` shows it unreleased, as [`unreleased`] answers, with
/// what the step that asked says.
pub(crate) fn release_judged(
    state: Option<&str>,
    app: u32,
    (released, left): (&str, &str),
) -> (Result<(), String>, String) {
    match state {
        Some(state) if release::unreleased(state) => {
            (Ok(()), format!("unreleased, as Steam shows it ({state})"))
        }
        Some(state) => (
            Err(format!("app {app} is released, so {released}")),
            format!("released, as Steam shows it ({state})"),
        ),
        None => (
            Err(format!(
                "Steam shows no release state for app {app}, so steamship leaves {left} to \
                 Steamworks"
            )),
            "not shown".to_owned(),
        ),
    }
}

/// The release state Steam shows `account` for `app`, asked through steamcmd run from `home`, with
/// the spinner that asked, for the caller to finish as the state is good or bad for it.
pub(crate) fn release_shown(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
) -> Result<(Option<String>, Spinner), ExitCode> {
    let (console, spinner) = app_info(
        home,
        account,
        app,
        packed,
        "asking whether the app is released",
    )?;
    match release::info_from_console(&console, app, "app info") {
        Ok(info) => Ok((release::state(&info).map(str::to_owned), spinner)),
        Err(why) => {
            spinner.failed("not shown");
            Err(fail(&why.and_again(), FAILED))
        }
    }
}

/// Whether steamship may set `app`'s default branch live, as [`unreleased`] answers.
pub(crate) fn default_unreleased(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
) -> Result<Result<(), String>, ExitCode> {
    unreleased(home, account, app, packed, DEFAULT_RELEASED)
}

/// Refuses, as [`unreleased`] says why, unless Steam shows the build account named `asked_as`,
/// or the CI login's, that `app` is unreleased.
pub(crate) fn refused_once_released(
    home: &Path,
    app: u32,
    asked_as: Option<&str>,
    left: (&str, &str),
) -> Result<(), ExitCode> {
    let (packed, account) = run_as(asked_as)?;
    refused_unless(home, packed.as_ref(), || {
        unreleased(home, &account, app, packed.as_ref(), left)
    })
}

/// Refuses as `ask` says why, asked of Steam with the pinned steamcmd, held, restored to `packed`'s
/// login in `home`, and verified once it has run.
pub(crate) fn refused_unless(
    home: &Path,
    packed: Option<&ci::Login>,
    ask: impl FnOnce() -> Result<Result<(), String>, ExitCode>,
) -> Result<(), ExitCode> {
    let (held, _, manifest) = ready()?;
    restore(packed, home)?;
    let asked = ask();
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    asked?.map_err(|why| fail(&why, REFUSED))
}
