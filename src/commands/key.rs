//! The publisher Web API key: where a command gets it, and what it says when Steam does not answer.

use std::env;
use std::io::{self, IsTerminal as _};
use std::path::Path;
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED};
use steamship::keychain;
use steamship::platform::Platform;
use steamship::redact::{self, Redactor};
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
use steamship::{builds, webapi};
use zeroize::Zeroizing;

use super::exit::fail;

/// The key `STEAMSHIP_WEB_API_KEY` holds, none when it is unset, and a refusal when it holds no key.
fn key_given() -> Result<Option<webapi::Key>, ExitCode> {
    let Some(text) = key_variable() else {
        return Ok(None);
    };
    webapi::Key::parse(&text).map(Some).map_err(|_not_a_key| {
        fail(
            &format!(
                "{} does not hold a publisher Web API key, which is 32 hexadecimal digits",
                webapi::KEY
            ),
            REFUSED,
        )
    })
}

/// The partner Web API, when a key is set in `STEAMSHIP_WEB_API_KEY` or kept. None otherwise,
/// and none is asked for.
pub(crate) fn key_at_hand() -> Result<Option<webapi::Api>, ExitCode> {
    if let Some(key) = key_given()? {
        return Ok(Some(webapi::Api::new(key)));
    }
    // Asked first without reading it, which no store prompts for. A store holding none is then
    // never asked to unlock, so a check stays free of prompts for anyone who kept no key.
    let kept = Platform::THIS
        .home(|name| env::var_os(name))
        .ok()
        .filter(|home| keychain::has(home).unwrap_or(false))
        .and_then(|home| keychain::kept(&home).ok().flatten());
    Ok(kept.map(webapi::Api::new))
}

/// The key set in `STEAMSHIP_WEB_API_KEY`, where an empty one is none, as for the account.
pub(crate) fn key_variable() -> Option<Zeroizing<String>> {
    env::var(webapi::KEY)
        .ok()
        .filter(|text| !text.is_empty())
        .map(Zeroizing::new)
}

/// What a refused empty answer to the key prompt says.
pub(crate) const NO_KEY_TYPED: &str = "no Web API key was typed. Run `steamship login --web-api-key-only` and paste the publisher key";

/// Where Steamworks shows the key, said before it is asked for.
pub(crate) const WHERE_KEY: &str = "the publisher Web API key is in Steamworks under Users & Permissions, \
                         Manage Groups, in the group that holds the app";

/// How the Web API key for a command was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyFrom {
    Variable,
    Kept,
    Typed,
}

/// The partner Web API, with the key from `STEAMSHIP_WEB_API_KEY`, or else the one kept in the
/// credential store, or else, at a terminal, one typed in.
pub(crate) fn web_api(home: &Path) -> Result<(webapi::Api, KeyFrom), ExitCode> {
    if let Some(key) = key_given()? {
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
    // A program reading standard output cannot be asked for a key on it.
    if !io::stdin().is_terminal() || show::diverted() {
        let keep = Hint {
            before: "run ",
            command: "steamship login --web-api-key-only",
            after: " at a terminal to keep one, or set STEAMSHIP_WEB_API_KEY",
        };
        show::failure("no Web API key", "", Some(keep));
        return Err(ExitCode::from(REFUSED));
    }
    show::aside(WHERE_KEY);
    let key = typed_key()?.ok_or_else(|| fail(&NO_KEY_TYPED, REFUSED))?;
    Ok((webapi::Api::new(key), KeyFrom::Typed))
}

/// The key typed at the prompt, shown as dots. None when only Enter was pressed.
pub(crate) fn typed_key() -> Result<Option<webapi::Key>, ExitCode> {
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
pub(crate) fn check_key(api: &webapi::Api) -> Result<(), ExitCode> {
    let spinner = Spinner::start("steam", "checking the Web API key", false);
    match api.apps() {
        Ok(apps) => {
            spinner.done(&format!(
                "Steam takes the key, for {}",
                show::counted(apps.len(), "app")
            ));
            show::fields(
                &apps
                    .iter()
                    .map(|app| (app.app_id.to_string(), app.name.clone()))
                    .collect::<Vec<_>>(),
            );
            Ok(())
        }
        Err(error) => Err(web_api_failed(spinner, &error)),
    }
}

/// Keeps `key` for `home` in the credential store, and answers whether it did. A failure is shown
/// with its reason.
pub(crate) fn keep_key(home: &Path, key: &webapi::Key) -> bool {
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

/// After a command that worked with a key typed at its prompt, offers to keep that key.
pub(crate) fn offer_to_keep(home: &Path, api: &webapi::Api, from: KeyFrom) {
    if from != KeyFrom::Typed {
        return;
    }
    show::aside(&format!(
        "Enter keeps the key in {} for next time, and n does not",
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

/// Why Steam gave `error` about `app`, when the apps the key holds can say: asked only for the one
/// error that needs them.
pub(crate) fn why_unanswered(api: &webapi::Api, app: u32, error: &webapi::Error) -> Option<String> {
    if !matches!(error, webapi::Error::Status(500, _)) {
        return None;
    }
    webapi::no_build_yet(error, app, &api.apps().ok()?)
}

/// `error` as a person reads it, with only the words Steam gave redacted.
pub(crate) fn answered(error: &webapi::Error) -> String {
    let redactor = answer_redactor();
    error.said_with(&|steam| redact::answer(redactor.as_ref(), steam))
}

/// The redactor for this machine's steamcmd login secrets, or none when its files cannot be read.
/// Steam's words are shown only after it has run over them.
pub(crate) fn answer_redactor() -> Option<Redactor> {
    let home = Platform::THIS.home(|name| env::var_os(name)).ok()?;
    Redactor::for_home(&home).ok()
}

/// Prints the `--json` line for Steam not answering about `app`, its words redacted.
pub(crate) fn print_unanswered(app: u32, error: &webapi::Error, why: Option<&str>) {
    let line = builds::error_json(app, error, why, answer_redactor().as_ref());
    show::plain(&format!("{line}\n"));
}

/// Says why the Web API did not answer, and the exit code for it.
pub(crate) fn web_api_failed(spinner: Spinner, error: &webapi::Error) -> ExitCode {
    spinner.failed(&match error {
        webapi::Error::NotAKey => "not a key".to_owned(),
        webapi::Error::Refused => "key refused".to_owned(),
        webapi::Error::Status(status, _) => format!("HTTP {status}"),
        webapi::Error::Unreachable(_) => "no answer".to_owned(),
        webapi::Error::Unreadable(_) => "unreadable".to_owned(),
        webapi::Error::Failed { .. } => "failed".to_owned(),
    });
    fail(&answered(error), FAILED)
}
