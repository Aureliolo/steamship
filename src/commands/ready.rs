//! What a command gets ready before it runs steamcmd.

use std::env;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{FAILED, LIMITED, LOGIN, REFUSED, STEAMCMD};
use steamship::install::{self, Outcome};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
use steamship::update::Installed;
use steamship::{ci, conversation, dump};

use super::exit::fail;
use super::key::answer_redactor;
use super::upload::redacted;

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
    held.map_err(|error| fail(&error, FAILED))
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
                install::Error::Altered { .. } => "differs from the pinned version",
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

/// Says why steamcmd cannot be used and answers its exit code. The text names files in steamcmd's
/// folder, so it goes through the redactor of this machine's login when one can be built.
pub(crate) fn steamcmd_failed(error: &install::Error) -> ExitCode {
    let text = error.to_string();
    let shown =
        answer_redactor().map_or_else(|| text.clone(), |redactor| redacted(&redactor, &text));
    show::failure(&shown, "", None);
    if let Some(remedy) = error.remedy() {
        show::note(remedy);
        show::hint(installed().hint());
    }
    ExitCode::from(STEAMCMD)
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
            &"no build account is named. Name it with --account or STEAMSHIP_ACCOUNT",
            REFUSED,
        )),
        Err(error) => Err(fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )),
    }
}

/// The account a command runs as, and the login a CI handed over, if it did. A handed-over
/// login's account is the one used unless another is named.
pub(crate) fn run_as(named: Option<&str>) -> Result<(Option<ci::Login>, Account), ExitCode> {
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    Ok((packed, account))
}

/// The account for `login` and `ci`, asked for at a terminal when none is known. Says whether it
/// was named or remembered, and nothing for one typed.
pub(crate) fn account_shown(named: Option<&str>) -> Result<Account, ExitCode> {
    let (account, source) = account(named, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    Ok(account)
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
        show::workflow(&format!("::add-mask::{}\n", login.account().name()));
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
/// `reason` is as steamcmd wrote it, and what is shown has the secrets of `redactor` taken out.
pub(crate) fn login_refused(
    reason: &str,
    redactor: &Redactor,
    packed: Option<&ci::Login>,
) -> ExitCode {
    login_refusal(reason, redactor, packed).0
}

/// As [`login_refused`], and also answers the text it printed.
pub(crate) fn login_refusal(
    reason: &str,
    redactor: &Redactor,
    packed: Option<&ci::Login>,
) -> (ExitCode, String) {
    let (words, log_in_again) = conversation::saved_login_words(reason);
    let mut said = redacted(redactor, &words);
    match packed.and_then(ci::Login::packed_on) {
        _ if !log_in_again => show::failure("not logged in", &said, None),
        None => {
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", &said, Some(again));
        }
        Some(day) => {
            let again = Hint {
                before: "log in again with ",
                command: "steamship login",
                after: ", then run steamship ci to pack the new login",
            };
            said = format!("{said}. The login in {} was packed on {day}", ci::VARIABLE);
            show::failure("not logged in", &said, Some(again));
        }
    }
    let code = ExitCode::from(if log_in_again { LOGIN } else { LIMITED });
    (code, format!("not logged in: {said}"))
}

/// The exit code for what Steam made of the saved login, or of the one handed over in `packed`.
/// It says why when Steam did not take the login.
pub(crate) fn verdict(
    judged: conversation::Login,
    redactor: &Redactor,
    packed: Option<&ci::Login>,
) -> ExitCode {
    match judged {
        conversation::Login::Taken => ExitCode::SUCCESS,
        conversation::Login::Refused(reason) => login_refused(&reason, redactor, packed),
        conversation::Login::Failed(reasons) => {
            for reason in &reasons {
                show::failure(&redacted(redactor, reason), "", None);
            }
            ExitCode::from(FAILED)
        }
    }
}

/// Steam's logs of a failed run, as collapsed groups in a GitHub Actions job's log: `files` as
/// they are, what was `captured` without being saved, then what steamcmd `added` to its own logs
/// this run, all of it redacted. The action's own home goes when the job ends, so this is the
/// only look at them there is.
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
    show::workflow(&shown);
}

#[cfg(test)]
mod tests {
    use std::io;

    use steamship::download;

    use super::*;

    #[test]
    fn every_fault_of_the_steamcmd_install_exits_4() {
        let faults = [
            install::Error::Altered {
                root: PathBuf::from("steamcmd"),
                changes: vec!["steamcmd.sh".to_owned()],
            },
            install::Error::Withdrawn {
                version: 1,
                system: "linux".to_owned(),
                file: "steamcmd_linux.zip".to_owned(),
            },
            install::Error::Io {
                path: PathBuf::from("steamcmd"),
                error: io::Error::other("no space"),
            },
            install::Error::Package {
                file: "steamcmd_linux.zip".to_owned(),
                reason: "is not a zip".to_owned(),
            },
            install::Error::Download(download::Error::Gone {
                url: "https://example.test/steamcmd_linux.zip".to_owned(),
                status: 404,
            }),
        ];
        for fault in &faults {
            assert_eq!(steamcmd_failed(fault), ExitCode::from(4), "{fault}");
        }
    }
}
