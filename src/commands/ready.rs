//! What a command gets ready before it runs steamcmd.

use std::env;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{FAILED, LOGIN, REFUSED, STEAMCMD};
use steamship::install::{self, Outcome};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
use steamship::update::Installed;
use steamship::{ci, conversation, dump};

use super::exit::fail;

/// How this steamship was installed, which is how it is upgraded.
pub(crate) fn installed() -> Installed {
    // Homebrew and winget run steamship through a link to where they keep it.
    env::current_exe().map_or(Installed::Archive, |program| {
        Installed::of(&fs::canonicalize(&program).unwrap_or(program), Path::exists)
    })
}

/// The account named with `--account` or `STEAMSHIP_ACCOUNT`, where an empty name is none: a CI
/// hands a secret it does not have over as an empty variable.
pub(crate) fn named(account: Option<&str>) -> Option<&str> {
    account.filter(|name| !name.is_empty())
}

pub(crate) fn home() -> Result<PathBuf, ExitCode> {
    Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))
}

/// The home, held once any other steamship using it has finished.
pub(crate) fn hold(home: &Path) -> Result<install::Held, ExitCode> {
    let mut waiting = None;
    let held = install::hold(home, || {
        waiting = Some(Spinner::start(
            "steamcmd",
            &format!(
                "waiting for another steamship using {} to finish",
                home.display()
            ),
            true,
        ));
    });
    if let Some(spinner) = waiting {
        let waited = show::took(spinner.elapsed());
        match &held {
            Ok(_) => spinner.done(&format!("free after {waited}")),
            Err(_) => spinner.failed("not free"),
        }
    }
    held.map_err(|error| steamcmd_failed(&error))
}

/// The home, held until the returned [`install::Held`] is dropped, and the pinned steamcmd in it,
/// installed or verified.
pub(crate) fn ready() -> Result<(install::Held, PathBuf, Manifest), ExitCode> {
    let home = home()?;
    let manifest = Manifest::pinned(Platform::THIS).map_err(|error| fail(&error, FAILED))?;
    let held = hold(&home)?;
    let spinner = Spinner::start("steamcmd", "checking the pinned version", false);
    match install::install(&held, &manifest) {
        Ok(Outcome::Installed) => spinner.done(&format!(
            "{} ({}), installed as pinned",
            manifest.version, manifest.system
        )),
        Ok(Outcome::Verified) => spinner.done(&format!(
            "{} ({}), pinned and verified",
            manifest.version, manifest.system
        )),
        Err(error) => {
            spinner.failed(match error {
                install::Error::Altered { .. } => "not as pinned",
                install::Error::Withdrawn { .. } => "no longer served",
                install::Error::Download(_)
                | install::Error::Io { .. }
                | install::Error::Package { .. } => "not installed",
            });
            return Err(steamcmd_failed(&error));
        }
    }
    Ok((held, home, manifest))
}

pub(crate) fn steamcmd_failed(error: &install::Error) -> ExitCode {
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
pub(crate) enum Source {
    Named,
    Remembered,
    Typed,
}

/// The account named on the command line or in the environment, or else the one remembered.
/// With `ask`, and a person at a terminal, the account's name is asked for when there is none.
pub(crate) fn account(named: Option<&str>, ask: bool) -> Result<(Account, Source), ExitCode> {
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
            &"no build account is named. Name it with --account or STEAMSHIP_ACCOUNT",
            REFUSED,
        )),
        Err(error) => Err(fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )),
    }
}

/// The login a CI handed over in `STEAMSHIP_LOGIN`, packed by `steamship ci`, if it did.
pub(crate) fn packed_login() -> Result<Option<ci::Login>, ExitCode> {
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
pub(crate) fn restore(packed: Option<&ci::Login>, home: &Path) -> Result<(), ExitCode> {
    if let Some(packed) = packed {
        drop(
            packed
                .restore(home, Platform::THIS)
                .map_err(|error| fail(&error, FAILED))?,
        );
    }
    Ok(())
}

/// Says Steam refused the login, and how to log in again: for one handed over in
/// `STEAMSHIP_LOGIN`, with the day it was packed, since the token in it may simply have expired.
pub(crate) fn login_refused(reason: &str, packed: Option<&ci::Login>) -> ExitCode {
    let (words, log_in_again) = conversation::saved_login_words(reason);
    match packed.and_then(ci::Login::packed_on) {
        _ if !log_in_again => show::failure("not logged in", &words, None),
        None => {
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", &words, Some(again));
        }
        Some(day) => {
            let again = Hint {
                before: "log in again with ",
                command: "steamship login",
                after: ", then run steamship ci to pack the new login",
            };
            show::failure(
                "not logged in",
                &format!("{words}. The login in {} was packed on {day}", ci::VARIABLE),
                Some(again),
            );
        }
    }
    ExitCode::from(LOGIN)
}

/// The exit code for what Steam made of the saved login, or of the one handed over in `packed`.
/// It says why when Steam did not take the login.
pub(crate) fn verdict(judged: conversation::Login, packed: Option<&ci::Login>) -> ExitCode {
    match judged {
        conversation::Login::Taken => ExitCode::SUCCESS,
        conversation::Login::Refused(reason) => login_refused(&reason, packed),
        conversation::Login::Failed(reasons) => {
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            ExitCode::from(FAILED)
        }
    }
}

/// Steam's logs of a failed run, as collapsed groups in a GitHub Actions job's log: `files` as
/// they are, what was `captured` without being saved, then what steamcmd `added` to its own logs
/// this run, all of it redacted. The action deletes steamship's home when the job ends, so this
/// is the only look at them there is.
pub(crate) fn dump_in_actions(
    files: &[(String, PathBuf)],
    captured: &[(String, Vec<u8>)],
    added: &[(String, Vec<u8>)],
    redactor: &Redactor,
) {
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
    for (title, bytes) in read.chain(captured.iter().cloned()).chain(steamcmds) {
        let text = redactor.redact(&bytes);
        shown.push_str(&dump::group(
            &title,
            &String::from_utf8_lossy(&text),
            &token,
        ));
    }
    show::plain(&shown);
}
