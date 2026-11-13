//! `steamship settings`: an app's settings shown, saved and compared with a snapshot.

use std::fmt::Display;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{self, FAILED, REFUSED};
use steamship::project::Project;
use steamship::show;
use steamship::{ci, install, settings, vdf};

use super::exit::fail;
use super::game::{app_named, game_for};
use super::ready::{named, ready, restore, run_as, steamcmd_failed};
use super::released::app_info;

/// `settings` with whatever is not given taken from the game: the snapshot is the file named, or
/// else `settings.vdf` beside its build scripts.
pub(crate) fn settings_in(request: &cli::Settings) -> Result<ExitCode, ExitCode> {
    let own = (request.save || request.check) && request.file.is_none();
    let (app, game) = game_for(request.app.as_deref(), own, request.demo)?;
    let snapshot = request
        .file
        .clone()
        .or_else(|| game.as_ref().map(Project::snapshot));
    try_settings(
        &app,
        snapshot.as_deref().filter(|_| request.save),
        snapshot.as_deref().filter(|_| request.check),
        named(request.account.as_deref()),
    )
}

/// The settings snapshot at `path`, refused before steamcmd is run when it is not `app`'s.
pub(crate) fn settings_file(path: &Path, app: u32) -> Result<vdf::Block, ExitCode> {
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
    let (held, home, manifest) = ready()?;
    restore(packed, &home)?;
    let asked = app_info(&home, account, app, packed, "asking for the settings");
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    let (console, spinner) = asked?;
    match settings::from_console(&console, app) {
        Ok(shown) => {
            spinner.done("shown");
            show::field("cloud", settings::cloud(&shown));
            show::field("input", &settings::controls(&shown));
            Ok(shown)
        }
        Err(why) => {
            spinner.failed("none shown");
            Err(fail(&why.and_again(), FAILED))
        }
    }
}

pub(crate) fn try_settings(
    app: &str,
    save: Option<&Path>,
    check: Option<&Path>,
    named: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    show::title("settings");
    let app_id = app_named(app)?;
    let wanted = check.map(|path| settings_file(path, app_id)).transpose()?;
    let (packed, account) = run_as(named)?;
    let shown = asked_settings(&account, app_id, packed.as_ref())?;
    let written = settings::snapshot(app_id, &shown).map_err(|why| fail(&why, FAILED))?;
    let folder = settings::install_folder(&shown).map(|why| format!("app {app_id}: {why}"));
    if let Some(path) = save {
        if let Some(why) = &folder {
            return Err(fail(why, REFUSED));
        }
        fs::write(path, &written)
            .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
        show::success(
            &format!("app {app_id}: settings saved to {}", path.display()),
            "steamship settings --check compares Steam's with them",
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(wanted) = wanted else {
        anstream::print!("{written}");
        if let Some(why) = &folder {
            show::note(why);
        }
        return Ok(ExitCode::SUCCESS);
    };
    let drift = settings::compare(&wanted, &shown);
    if drift.is_empty() && folder.is_none() {
        show::success("Steam's settings match the file", "");
        return Ok(ExitCode::SUCCESS);
    }
    for difference in &drift {
        show::failure(&difference.to_string(), "", None);
    }
    if let Some(why) = &folder {
        show::failure(why, "", None);
    }
    show::aside(
        "change the app in Steamworks, or run steamship settings --save again if the change \
         was meant",
    );
    Ok(ExitCode::from(REFUSED))
}
