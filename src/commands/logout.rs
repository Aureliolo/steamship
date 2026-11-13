//! `steamship logout`: the saved login, the account and the key forgotten.

use std::fmt::Display;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::FAILED;
use steamship::show::{self, Hint};
use steamship::{install, keychain, steamcmd};

use super::ready::{hold, home};

/// Each step runs even when an earlier one failed, so one file that cannot be removed does not
/// leave the rest for the next job. Any failure gives exit code 1 once every step has run.
pub(crate) fn run_logout() -> ExitCode {
    show::title("logout");
    let home = match home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    let mut failed = false;
    {
        // Not while another steamship's steamcmd may be signed in with what is about to go.
        let _held = if home.exists() {
            match hold(&home) {
                Ok(held) => Some(held),
                Err(code) => return code,
            }
        } else {
            None
        };
        failed |= forget_login(&home);
        failed |= forget_account(&home);
        if let Err(error) = install::clear_working(&home) {
            show::failed("install folders", "not cleared");
            report(&error, BY_HAND);
            failed = true;
        }
    }
    failed |= forget_key(&home);
    if failed {
        return ExitCode::from(FAILED);
    }
    show::success("logged out", "the next upload needs steamship login first");
    ExitCode::SUCCESS
}

/// What to do about a failure here: a file steamship cannot remove stays until a person does.
const BY_HAND: Hint<'_> = Hint {
    before: "remove what is named by hand, then run ",
    command: "steamship logout",
    after: " again",
};

fn report(error: &dyn Display, hint: Hint<'_>) {
    show::failure(&error.to_string(), "", Some(hint));
}

/// Removes steamcmd's saved login, a stopped install's carried one included. Returns true when it
/// failed.
fn forget_login(home: &Path) -> bool {
    let mut problems = Vec::new();
    let mut found = false;
    for listing in [
        steamcmd::list_login_files(home),
        steamcmd::carried_login_files(home),
    ] {
        found |= !listing.files.is_empty();
        for file in listing.files {
            // The hold means no other steamship removes it first.
            if let Err(error) = fs::remove_file(&file) {
                problems.push(format!("{}: {error}", file.display()));
            }
        }
        problems.extend(listing.unreadable.iter().map(ToString::to_string));
    }
    if !problems.is_empty() {
        show::failed("login", "not forgotten");
        for problem in &problems {
            report(problem, BY_HAND);
        }
        return true;
    }
    if found {
        show::done("login", "forgotten");
    } else {
        show::field("login", "none saved");
    }
    false
}

/// Forgets the remembered account name. Returns true when it failed.
fn forget_account(home: &Path) -> bool {
    match Account::forget(home) {
        Ok(true) => show::done("account", "forgotten"),
        Ok(false) => show::field("account", "none remembered"),
        Err(error) => {
            show::failed("account", "not forgotten");
            report(&format!("{}: {error}", home.display()), BY_HAND);
            return true;
        }
    }
    false
}

/// Forgets the Web API key kept for `home`. Returns true when it failed.
fn forget_key(home: &Path) -> bool {
    match keychain::forget(home) {
        Ok(true) => show::done("api key", "forgotten"),
        Ok(false) | Err(keychain::Error::Unavailable(_)) => show::field("api key", "none kept"),
        Err(
            error @ (keychain::Error::Failed(_)
            | keychain::Error::Unsettled(_)
            | keychain::Error::NotAKey),
        ) => {
            show::failed("api key", "not forgotten");
            report(
                &error,
                Hint {
                    before: "remove the key from the credential store by hand, then run ",
                    command: "steamship logout",
                    after: " again",
                },
            );
            return true;
        }
    }
    false
}
