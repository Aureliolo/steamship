//! `steamship logout`: the saved login, the account and the key forgotten.

use std::fs;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::FAILED;
use steamship::show;
use steamship::{keychain, steamcmd};

use super::exit::fail;
use super::ready::{hold, home};

pub(crate) fn run_logout() -> ExitCode {
    show::title("logout");
    let forgot = home().and_then(|home| {
        // Not while another steamship's steamcmd may be signed in with what is about to go.
        let _held = if home.exists() {
            Some(hold(&home)?)
        } else {
            None
        };
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
    show::success("logged out", "the next upload needs steamship login first");
    ExitCode::SUCCESS
}
