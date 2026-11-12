//! `steamship settings`: an app's settings shown, saved and compared with a snapshot.

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{FAILED, REFUSED};
use steamship::project::Project;
use steamship::show;
use steamship::{ci, install, settings, vdf};

use super::exit::fail;
use super::game::{app_given_or, app_named, game_as};
use super::ready::{ready, restore, run_as, steamcmd_failed};
use super::released::app_info;

/// The snapshot `--save` or `--check` names: a file, or the game's own when it names none.
pub(crate) enum Snapshot {
    File(PathBuf),
    Game,
}

impl Snapshot {
    pub(crate) fn given(path: Option<PathBuf>) -> Self {
        path.map_or(Self::Game, Self::File)
    }
}

/// `settings` with whatever is not given taken from the game.
pub(crate) fn settings_in(
    app: Option<String>,
    demo: bool,
    save: Option<Snapshot>,
    check: Option<Snapshot>,
    account: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    let own = matches!(save, Some(Snapshot::Game)) || matches!(check, Some(Snapshot::Game));
    let game = (app.is_none() || own).then(|| game_as(demo)).transpose()?;
    let path = |given: Snapshot| match given {
        Snapshot::File(path) => Some(path),
        Snapshot::Game => game.as_ref().map(Project::snapshot),
    };
    let save = save.and_then(path);
    let check = check.and_then(path);
    try_settings(
        &app_given_or(app, game.as_ref()),
        save.as_deref(),
        check.as_deref(),
        account,
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
