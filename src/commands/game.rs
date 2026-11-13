//! The app a command works on: the game the current folder is in, or the one named.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED};
use steamship::project::{self, Project};
use steamship::show::{self, Spinner};
use steamship::{run, scripts};

use super::exit::fail;

/// The game the current folder is in: refused when there is none, its `steamship.toml` says what
/// steamship does not take, or it pins another steamship.
pub(crate) fn game() -> Result<Project, ExitCode> {
    let here = env::current_dir()
        .map_err(|error| fail(&format!("the current folder: {error}"), FAILED))?;
    let found = Project::find(&here)
        .map_err(|why| fail(&why, REFUSED))?
        .ok_or_else(|| {
            fail(
                &format!(
                    "there is no game in {}: no {} or steam/app_build.vdf here or in a folder \
                     above. Run steamship in the game's folder, run `steamship init <app ID>` \
                     to start one, or name the app and its files",
                    here.display(),
                    project::FILE
                ),
                REFUSED,
            )
        })?;
    found
        .pinned(env!("CARGO_PKG_VERSION"))
        .map_err(|why| fail(&why, REFUSED))?;
    Ok(found)
}

/// The game the current folder is in, or with `demo` that game seen as its demo.
pub(crate) fn game_as(demo: bool) -> Result<Project, ExitCode> {
    let found = game()?;
    if demo {
        found.demo().map_err(|why| fail(&why, REFUSED))
    } else {
        Ok(found)
    }
}

/// The app as `app` names it, or else as the game the current folder is in, or with `demo` its
/// demo: its build script.
pub(crate) fn app_or_game(app: Option<String>, demo: bool) -> Result<String, ExitCode> {
    app.map_or_else(|| game_as(demo).map(|found| script_named(&found)), Ok)
}

/// The game's app build script, as an app is named on the command line.
pub(crate) fn script_named(game: &Project) -> String {
    game.script.display().to_string()
}

/// The game's app build script, read, every problem in it shown, and refused unless it sets the
/// game's branch live.
pub(crate) fn game_script(game: &Project) -> Result<scripts::AppScript, ExitCode> {
    let script = scripts::load(&game.script).map_err(|problems| {
        for problem in &problems {
            show::failure("refused", &problem.to_string(), None);
        }
        ExitCode::from(REFUSED)
    })?;
    game.live_on(script.set_live.as_deref())
        .map_err(|why| fail(&why, REFUSED))?;
    Ok(script)
}

/// The game's files `achievements` checks or writes from: the records holding achievements,
/// stats or sets, and every catalogue beside them.
pub(crate) fn game_achievements(game: &Project) -> Result<Vec<PathBuf>, ExitCode> {
    let refused = |why: String| fail(&why, REFUSED);
    let mut files = game.records(project::ACHIEVEMENTS).map_err(refused)?;
    if !files.is_empty() {
        files.extend(game.catalogues().map_err(refused)?);
    }
    Ok(files)
}

/// Shows the game, and refuses it unless its tree holds nothing uncommitted and origin's default
/// branch is at its commit.
pub(crate) fn shipped_from(game: &Project) -> Result<(), ExitCode> {
    show::title("game");
    show::field(
        "folder",
        &if game.configured {
            format!("{}, as {} lays it out", game.root.display(), project::FILE)
        } else {
            game.root.display().to_string()
        },
    );
    show::field("cores", &held_to(game.cores)?);
    let spinner = Spinner::start("commit", "asking origin for its default branch", false);
    match project::pushed(&game.root) {
        Ok(pushed) => {
            spinner.done(&format!(
                "{}, as origin's {} has it, with nothing uncommitted",
                project::short(&pushed.commit),
                pushed.branch
            ));
            Ok(())
        }
        Err(why) => {
            spinner.failed("not pushed");
            Err(fail(&why, REFUSED))
        }
    }
}

/// Holds steamship, and so steamcmd and the build it starts, to `cores` processors, and says what
/// came of it.
fn held_to(cores: usize) -> Result<String, ExitCode> {
    match run::hold_to(cores) {
        Ok(Some((held, of))) if held < of => Ok(format!(
            "{held} of {of}, the most steamcmd and the build run on"
        )),
        Ok(Some((_, of))) => Ok(format!(
            "all {of}, within the {cores} steamcmd and the build may run on"
        )),
        Ok(None) => Ok("every one, as macOS holds no program to fewer".to_owned()),
        Err(error) => Err(fail(
            &format!("steamship could not hold itself to {cores} processors: {error}"),
            FAILED,
        )),
    }
}

/// The app a command works on, as its ID, and the game when one was looked for. A given app is
/// read before the game, so a file given first is told it is no app. The game is read when no app
/// is given or `own` asks for its files, and a given app must be the game's.
pub(crate) fn game_for(
    app: Option<&str>,
    own: bool,
    demo: bool,
) -> Result<(String, Option<Project>), ExitCode> {
    let given = app.map(app_read).transpose()?;
    let game = (given.is_none() || own)
        .then(|| game_as(demo))
        .transpose()?;
    match (given, &game) {
        (Some(app_id), Some(found)) => {
            names_app(found, app_id)?;
            Ok((app_id.to_string(), game))
        }
        (Some(app_id), None) => Ok((app_id.to_string(), game)),
        (None, found) => Ok((found.as_ref().map_or_else(String::new, script_named), game)),
    }
}

/// Refuses `app_id` unless the game's app build script names it.
fn names_app(game: &Project, app_id: u32) -> Result<(), ExitCode> {
    match scripts::app_named(&game.script) {
        Some(named) if named == app_id => Ok(()),
        named => Err(fail(
            &format!(
                "app {app_id} is not this game's: {} {}",
                game.script.display(),
                named.map_or_else(
                    || format!("names no app. Give the path of a script that names {app_id}"),
                    |named| format!(
                        "names app {named}. Give that app's ID, or the path of the script that \
                         names {app_id}"
                    )
                )
            ),
            REFUSED,
        )),
    }
}

/// The app build script `app` names for `upload` and `check`: the game's own when no app is
/// given or the number is the game's app, else the path given.
pub(crate) fn script_given(app: Option<&str>, demo: bool) -> Result<PathBuf, ExitCode> {
    match app {
        None => Ok(game_as(demo)?.script),
        Some(word) => match word.parse::<u32>() {
            Ok(app_id) => {
                let found = game_as(demo)?;
                names_app(&found, app_id)?;
                Ok(found.script)
            }
            Err(_) => Ok(PathBuf::from(word)),
        },
    }
}

/// The app named by its ID, or by the app build script that names it, with nothing shown unless
/// the word is refused.
pub(crate) fn app_read(app: &str) -> Result<u32, ExitCode> {
    match app.parse::<u32>() {
        Ok(app_id) if app_id > 0 => Ok(app_id),
        Ok(_) => Err(fail(
            &"0 is not an app ID. Give the app's ID from Steamworks",
            REFUSED,
        )),
        Err(_) => scripts::load(Path::new(app))
            .map(|script| script.app_id)
            .map_err(|problems| {
                for problem in &problems {
                    show::failure("refused", &problem.to_string(), None);
                }
                show::note(
                    "the first word names the app, by its ID or its app build script. Files come \
                     after it",
                );
                ExitCode::from(REFUSED)
            }),
    }
}

/// The app named by its ID, or by the app build script that names it, shown.
pub(crate) fn app_named(app: &str) -> Result<u32, ExitCode> {
    let app_id = app_read(app)?;
    show::field("app", &app_id.to_string());
    Ok(app_id)
}
