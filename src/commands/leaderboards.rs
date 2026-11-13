//! `steamship leaderboards`: Steam's leaderboards listed, saved, compared with the files and made.

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::cli::{self, FAILED, REFUSED};
use steamship::project;
use steamship::show::{self, Spinner};
use steamship::{builds, leaderboards, webapi};

use super::achievements::written_over;
use super::exit::fail;
use super::game::{app_named, game_for};
use super::key::{offer_to_keep, web_api, web_api_failed};
use super::ready::{home, named};
use super::released::refused_once_released;

/// What `leaderboards` changes on Steam before it compares.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Changes<'command> {
    create: bool,
    delete_missing: bool,
    account: Option<&'command str>,
}

/// How `leaderboards` with no files to check gives what Steam holds.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Listing<'command> {
    /// A line for each.
    Lines,
    /// One line of JSON on standard output.
    Json,
    /// Written to this file, as `--check` reads it.
    Saved(&'command Path),
}

/// `leaderboards` with whatever is not given taken from the game.
pub(crate) fn leaderboards_in(request: cli::Leaderboards) -> Result<ExitCode, ExitCode> {
    let cli::Leaderboards {
        app,
        mut files,
        check,
        save,
        create,
        delete_missing,
        json,
        account,
    } = request;
    let listing = match (save.as_deref(), json) {
        (Some(path), _) => Listing::Saved(path),
        (None, true) => Listing::Json,
        (None, false) => Listing::Lines,
    };
    let changes = Changes {
        create,
        delete_missing,
        account: named(account.as_deref()),
    };
    let own = files.is_empty() && (check || create || delete_missing);
    let (app, game) = game_for(app.as_deref(), own, false)?;
    if let Some(game) = game.as_ref().filter(|_| own) {
        files = game
            .records(project::LEADERBOARDS)
            .map_err(|why| fail(&why, REFUSED))?;
        if files.is_empty() {
            return Err(fail(
                &"the game holds no leaderboard records to check or make",
                REFUSED,
            ));
        }
    }
    try_leaderboards(&app, &files, listing, changes)
}

fn leaderboards_file(path: &Path, app: u32) -> Result<leaderboards::Listed, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let listed = leaderboards::read(&text).map_err(|why| refused(&why))?;
    if let Some(other) = listed.app.filter(|named| *named != u64::from(app)) {
        return Err(refused(&format!("is for app {other}, not app {app}")));
    }
    show::field(
        "file",
        &format!(
            "{}, {}",
            path.display(),
            match listed.templates.len() {
                0 => show::counted(listed.leaderboards.len(), "leaderboard"),
                templates => format!(
                    "{}, {}",
                    show::counted(listed.leaderboards.len(), "leaderboard"),
                    show::counted(templates, "dated template")
                ),
            }
        ),
    );
    Ok(listed)
}

/// Gives `held`, app `app_id`'s leaderboards on Steam, as `listing` asks.
fn list_leaderboards(
    app_id: u32,
    held: &[webapi::Leaderboard],
    listing: Listing<'_>,
) -> Result<ExitCode, ExitCode> {
    let path = match listing {
        Listing::Lines => {
            show::fields(
                &held
                    .iter()
                    .map(|leaderboard| (leaderboard.name.clone(), leaderboards::line(leaderboard)))
                    .collect::<Vec<_>>(),
            );
            return Ok(ExitCode::SUCCESS);
        }
        Listing::Json => {
            show::plain(&format!("{}\n", leaderboards::json(app_id, held)));
            return Ok(ExitCode::SUCCESS);
        }
        Listing::Saved(path) => path,
    };
    let text = leaderboards::saved(app_id, held)
        .map_err(|why| fail(&format!("app {app_id}: {why}"), REFUSED))?;
    fs::write(path, text).map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    show::success(
        &format!(
            "app {app_id}: {} saved to {}",
            show::counted(held.len(), "leaderboard"),
            path.display()
        ),
        "steamship leaderboards --check compares Steam's with them",
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn try_leaderboards(
    app: &str,
    check: &[PathBuf],
    listing: Listing<'_>,
    changes: Changes<'_>,
) -> Result<ExitCode, ExitCode> {
    let json = matches!(listing, Listing::Json);
    if json {
        show::divert();
    }
    show::title("leaderboards");
    let app_id = app_named(app)?;
    if let Listing::Saved(path) = listing {
        written_over(path, &leaderboards::SAVED)?;
    }
    let mut files = Vec::with_capacity(check.len());
    for path in check {
        files.push((path.display().to_string(), leaderboards_file(path, app_id)?));
    }
    let wants = leaderboards::together(files).map_err(|why| fail(&why, REFUSED))?;
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let asking = Spinner::start("steam", "asking for the leaderboards", false);
    let mut held = match api.leaderboards(app_id) {
        Ok(held) => held,
        Err(error) => {
            let code = web_api_failed(asking, &error);
            if json {
                show::plain(&format!("{}\n", builds::error_json(app_id, &error, None)));
            }
            return Err(code);
        }
    };
    asking.done(&format!(
        "{} on Steam",
        show::counted(held.len(), "leaderboard")
    ));
    offer_to_keep(&home, &api, from);
    if check.is_empty() {
        return list_leaderboards(app_id, &held, listing);
    }
    if changes.delete_missing {
        let gone: Vec<String> = held
            .iter()
            .filter(|leaderboard| !wants.holds(&leaderboard.name))
            .map(|leaderboard| leaderboard.name.clone())
            .collect();
        if !gone.is_empty() {
            let released = format!(
                "deleting {} would take its players' scores with it. Delete it in Steamworks, \
                 under Stats & Achievements, Leaderboards, or put it back in the file",
                gone.join(", ")
            );
            refused_once_released(
                &home,
                app_id,
                changes.account,
                (&released, "deleting its leaderboards"),
            )?;
        }
        for name in gone {
            let spinner = Spinner::start("steam", &format!("deleting {name}"), false);
            if let Err(error) = api.delete_leaderboard(app_id, &name) {
                return Err(web_api_failed(spinner, &error));
            }
            spinner.done(&format!("deleted {name}"));
            held.retain(|leaderboard| leaderboard.name != name);
        }
    }
    if changes.create {
        for wanted in &wants.leaderboards {
            if held
                .iter()
                .any(|leaderboard| leaderboard.name == wanted.name)
            {
                continue;
            }
            let spinner = Spinner::start("steam", &format!("making {}", wanted.name), false);
            match api.create_leaderboard(app_id, wanted) {
                Ok(made) => {
                    spinner.done(&format!("made {}", made.name));
                    held.push(made);
                }
                Err(error) => return Err(web_api_failed(spinner, &error)),
            }
        }
    }
    let drift = leaderboards::compare(&wants, &held);
    if drift.is_empty() {
        let named = if check.len() > 1 { "files" } else { "file" };
        show::success(&format!("Steam's leaderboards match the {named}"), "");
        return Ok(ExitCode::SUCCESS);
    }
    for difference in &drift {
        show::failure(&difference.to_string(), "", None);
    }
    if drift
        .iter()
        .any(|difference| matches!(difference, leaderboards::Drift::Differs { .. }))
    {
        show::aside(
            "Steam keeps a leaderboard's settings. Change them in Steamworks, under Stats & \
             Achievements, Leaderboards",
        );
    }
    Ok(ExitCode::from(REFUSED))
}
