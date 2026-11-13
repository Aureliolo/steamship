//! `steamship init`: the first build scripts for an app.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED};
use steamship::init;
use steamship::project::Project;
use steamship::show;

use super::exit::fail;

pub(crate) fn run_init(app: u32, mut depots: Vec<init::Depot>, folder: &Path) -> ExitCode {
    show::title("init");
    if depots.is_empty() {
        let Some(id) = app.checked_add(1) else {
            return fail(
                &format!("app {app} leaves no ID for a depot after it"),
                REFUSED,
            );
        };
        depots.push(init::Depot {
            id,
            folder: "build".to_owned(),
        });
    }
    // The game's branch is read from its steamship.toml once the scripts are there, so a file that
    // cannot be read is refused before anything is written.
    if let Some(Err(why)) = env::current_dir().ok().map(|here| Project::find(&here)) {
        return fail(&why, REFUSED);
    }
    let scripts = match init::scripts(app, &depots, folder, None) {
        Ok(scripts) => scripts,
        Err(why) => return fail(&why, REFUSED),
    };
    let paths: Vec<(PathBuf, String)> = scripts
        .into_iter()
        .map(|(name, text)| (folder.join(name), text))
        .collect();
    // All or nothing: a script already there is someone's, and half a set would not check.
    if let Some((there, _)) = paths.iter().find(|(path, _)| path.exists()) {
        return fail(
            &format!(
                "{} already exists and is kept. Give another --output, or remove it",
                there.display()
            ),
            REFUSED,
        );
    }
    if let Err(error) = fs::create_dir_all(folder) {
        return fail(&format!("{}: {error}", folder.display()), FAILED);
    }
    for (path, text) in &paths {
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| io::Write::write_all(&mut file, text.as_bytes()));
        if let Err(error) = written {
            return fail(&format!("{}: {error}", path.display()), FAILED);
        }
        show::field("wrote", &path.display().to_string());
    }
    let app_script = folder.join("app_build.vdf");
    // Which game a script belongs to, if any, shows only once it is there to be found.
    if let Some(branch) = branch_of_game(&app_script) {
        let set = init::scripts(app, &depots, folder, Some(&branch))
            .map_err(|why| fail(&why, REFUSED))
            .and_then(|again| {
                let (_, text) = again.into_iter().next().unwrap_or_default();
                fs::write(&app_script, text)
                    .map_err(|error| fail(&format!("{}: {error}", app_script.display()), FAILED))
            });
        if let Err(code) = set {
            return code;
        }
        show::field("branch", &format!("{branch}, the game's, set live"));
    }
    show::success(
        "scripts written",
        &format!(
            "put the build in each depot's folder, then check it with steamship check {}",
            app_script.display()
        ),
    );
    ExitCode::SUCCESS
}

/// The branch of the game in the current folder whose app build script, or demo's, `script` is.
fn branch_of_game(script: &Path) -> Option<String> {
    let here = env::current_dir().ok()?;
    let game = Project::find(&here).ok()??;
    let written = fs::canonicalize(script).ok()?;
    let is_it = |found: &Path| fs::canonicalize(found).is_ok_and(|found| found == written);
    if is_it(&game.script) {
        return Some(game.branch);
    }
    game.demo()
        .ok()
        .filter(|demo| is_it(&demo.script))
        .map(|demo| demo.branch)
}
