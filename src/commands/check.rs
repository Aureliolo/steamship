//! `steamship check`: a script, a build's folder or the game, checked before anything is sent.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, ExitCode};
use std::time::SystemTime;

use steamship::cli::{FAILED, REFUSED, Writing};
use steamship::project::{self, Project};
use steamship::show::{self, Spinner};
use steamship::{check, presets, receipt, redist, run, scripts, settings, upload, vdf};

use super::achievements::try_achievements;
use super::builds::try_builds;
use super::exit::{either, fail, first_failure, refusals};
use super::game::{game_achievements, game_as, game_script, script_named};
use super::key::{key_at_hand, why_unanswered};
use super::leaderboards::{Changes, Listing, try_leaderboards};
use super::presence::{presence_read, try_rich_presence};
use super::settings::settings_file;

/// `check` of the build's folder or the script given, or of the game, or its demo, when there is
/// neither.
pub(crate) fn check_in(script: Option<&Path>, scripts_only: bool, demo: bool) -> ExitCode {
    match script {
        Some(folder) if folder.is_dir() => run_check_folder(folder, scripts_only, demo),
        Some(script) if demo => fail(
            &format!(
                "{} is an app build script, which names its own app. Give --demo a build's \
                 folder, or nothing in the game's folder",
                script.display()
            ),
            REFUSED,
        ),
        Some(script) => run_check(script, scripts_only, None),
        None => either(try_game_check(demo)),
    }
}

/// Refuses the game's settings snapshot when it shows `script`'s app unfit for it: an install
/// folder of anything but letters and digits, a depot of the script's not among the app's own, a
/// system the app's depots install on that no launch option runs on, or, for a demo, an app not
/// typed Demo with the full game as its base game, or without the Steam Input defaults the full
/// game's snapshot shows. Refused too when there is none and one is `needed`.
pub(crate) fn snapshot_sound(
    game: &Project,
    script: &scripts::AppScript,
    needed: bool,
) -> Result<(), ExitCode> {
    let app = script.app_id;
    let path = game.snapshot();
    if !path.is_file() {
        if needed {
            return Err(fail(
                &format!(
                    "{} is missing. Run steamship settings --save in the game, and commit the \
                     file",
                    path.display()
                ),
                REFUSED,
            ));
        }
        return Ok(());
    }
    let snapshot = settings_file(&path, app)?;
    let mut wrong: Vec<String> = settings::install_folder(&snapshot).into_iter().collect();
    let depots: Vec<u32> = script.depots.iter().map(|depot| depot.depot_id).collect();
    wrong.extend(settings::foreign(&snapshot, &depots).into_iter().map(|depot| {
        format!(
            "{} uploads depot {depot}, which is not among app {app}'s own depots. A depot goes up \
             in the app it was made under. If it is new, publish it in Steamworks and save the \
             settings again",
            game.script.display()
        )
    }));
    wrong.extend(settings::unlaunched(&snapshot).into_iter().map(|system| {
        format!(
            "app {app} has a depot for {system}, and no launch option runs on {system}. \
                     Add one under Installation, General Installation in Steamworks, then save \
                     the settings again"
        )
    }));
    if let Some(full) = &game.full {
        let full = scripts::load(full).map_err(|problems| {
            let problems: Vec<String> = problems.iter().map(ToString::to_string).collect();
            refusals(&problems)
        })?;
        wrong.extend(settings::demo_of(&snapshot, app, full.app_id));
        let full_snapshot = full
            .path
            .parent()
            .unwrap_or(&game.root)
            .join(settings::SNAPSHOT);
        if full_snapshot.is_file() {
            let game_settings = settings_file(&full_snapshot, full.app_id)?;
            wrong.extend(settings::controls_unlike(
                &snapshot,
                app,
                &game_settings,
                full.app_id,
            ));
        }
    }
    if wrong.is_empty() {
        return Ok(());
    }
    for why in &wrong {
        show::failure(&format!("{}: {why}", path.display()), "", None);
    }
    Err(ExitCode::from(REFUSED))
}

/// Refuses a game and its demo that share an app or a depot, so that neither's build can go up
/// into the other's. Does nothing when the game has no demo.
///
/// The full game is shipped whatever is wrong with the demo's script, such as an app ID of 0
/// held in place until the demo's app exists: steamship cannot upload that script, so it cannot
/// take the full game's place. The demo's own check and upload refuse it.
pub(crate) fn apart(game: &Project) -> Result<(), ExitCode> {
    let (full_path, demo_path) = match (&game.full, &game.demo) {
        (Some(full), _) => (full.clone(), game.script.clone()),
        (None, Some(demo)) => (game.script.clone(), demo.script.clone()),
        (None, None) => return Ok(()),
    };
    let load = |path: &Path| {
        scripts::load(path).map_err(|problems| {
            let problems: Vec<String> = problems.iter().map(ToString::to_string).collect();
            refusals(&problems)
        })
    };
    let demo = if game.full.is_some() {
        load(&demo_path)?
    } else {
        match scripts::load(&demo_path) {
            Ok(demo) => demo,
            Err(_) => return Ok(()),
        }
    };
    let full = load(&full_path)?;
    let mut wrong = Vec::new();
    if full.app_id == demo.app_id {
        wrong.push(format!(
            "{} names app {}, the full game's own in {}. A demo is its own app, so put the \
             demo's ID in it",
            demo_path.display(),
            demo.app_id,
            full_path.display()
        ));
    }
    for depot in &demo.depots {
        if full
            .depots
            .iter()
            .any(|other| other.depot_id == depot.depot_id)
        {
            wrong.push(format!(
                "depot {} is in both {} and {}: a demo ships depots of its own",
                depot.depot_id,
                demo_path.display(),
                full_path.display()
            ));
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(refusals(&wrong))
    }
}

/// Refuses `script`'s depots shipping an export that the game's Godot presets make for the other
/// of the full game and its demo: one with the demo's feature in the full game's, or one without
/// it in the demo's.
pub(crate) fn exports_apart(game: &Project, script: &scripts::AppScript) -> Result<(), ExitCode> {
    let presets = presets::read(&game.root).map_err(|why| fail(&why, FAILED))?;
    let demo = game.full.is_some();
    let mut wrong = Vec::new();
    for preset in presets
        .iter()
        .filter(|preset| preset.features.contains(&game.feature) != demo)
    {
        for depot in script
            .depots
            .iter()
            .filter(|depot| depot.ships(&preset.export))
        {
            let (whose, made) = if demo {
                ("the demo's", "without")
            } else {
                ("the full game's", "with")
            };
            wrong.push(format!(
                "{} ships {} into depot {}, {whose}, and the export preset \"{}\" in {} makes it \
                 {made} the \"{}\" feature. Map only the folders {whose} presets export to",
                depot.path.display(),
                preset.export.display(),
                depot.depot_id,
                preset.name,
                presets::FILE,
                game.feature
            ));
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(refusals(&wrong))
    }
}

/// Compares the game's achievements, leaderboards and rich presence with Steam's, each that the
/// game has, every one whatever the others found.
pub(crate) fn compared(game: &Project, app: &str) -> Result<ExitCode, ExitCode> {
    let refused = |why: String| fail(&why, REFUSED);
    let records = || {
        game.records
            .iter()
            .map(|given| given.path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut codes = Vec::new();
    if game.full.is_some() {
        let why = "the game's records are the full game's, so the demo's are not compared";
        unchecked("achievements", why);
        unchecked("leaderboards", why);
        return compared_presence(game, app, codes);
    }
    let achievements = game_achievements(game)?;
    if achievements.is_empty() {
        unchecked(
            "achievements",
            &format!(
                "no achievement, stat or set records in {}. List the game's under `records` in \
                 {}",
                records(),
                project::FILE
            ),
        );
    } else {
        let receipt = game.import.join(receipt::RECEIPT).is_file();
        let writing = Writing {
            write: None,
            output: receipt.then(|| game.import.clone()),
            unchanged: false,
            imported: false,
        };
        codes.push(either(try_achievements(
            app,
            &achievements,
            None,
            &writing,
            false,
        )));
    }
    let boards = game.records(project::LEADERBOARDS).map_err(refused)?;
    if boards.is_empty() {
        unchecked(
            "leaderboards",
            &format!("no leaderboard records in {}", records()),
        );
    } else {
        codes.push(either(try_leaderboards(
            app,
            &boards,
            Listing::Lines,
            Changes::default(),
        )));
    }
    compared_presence(game, app, codes)
}

/// The first failure of `codes` and the comparison of the game's rich presence with Steam's.
fn compared_presence(
    game: &Project,
    app: &str,
    mut codes: Vec<ExitCode>,
) -> Result<ExitCode, ExitCode> {
    let presence = game.presence().map_err(|why| fail(&why, REFUSED))?;
    if presence.is_empty() {
        unchecked(
            "rich-presence",
            &format!(
                "no rich_presence_<language>.vdf beside {}",
                game.full.as_ref().unwrap_or(&game.script).display()
            ),
        );
    } else {
        codes.push(either(try_rich_presence(app, &presence, true)));
    }
    Ok(first_failure(codes))
}

/// Says that the game gave `command` nothing to compare, so that a comparison left out is never
/// taken for one that passed.
fn unchecked(command: &str, why: &str) {
    show::title(command);
    show::field("unchecked", why);
}

/// `check` in a game: its scripts, then its records and rich presence against Steam's, and its
/// branches.
fn try_game_check(demo: bool) -> Result<ExitCode, ExitCode> {
    let game = game_as(demo)?;
    let checked = run_check(&game.script, true, Some(&game));
    if checked != ExitCode::SUCCESS {
        return Ok(checked);
    }
    let script = game_script(&game)?;
    apart(&game)?;
    exports_apart(&game, &script)?;
    snapshot_sound(&game, &script, false)?;
    let app = script_named(&game);
    let compared = compared(&game, &app)?;
    let builds = either(try_builds(&app, 10, false));
    Ok(first_failure([compared, builds]))
}

/// Runs the game's `build` command in its folder, its output shown as it comes, and refuses when
/// it fails or leaves the tree changed.
pub(crate) fn built(game: &Project) -> Result<(), ExitCode> {
    let Some((program, args)) = game.build.split_first() else {
        show::field(
            "build",
            &format!(
                "none named {}in {}, so the content goes up as it is",
                if game.full.is_some() {
                    "under [demo] "
                } else {
                    ""
                },
                project::FILE
            ),
        );
        return Ok(());
    };
    let named = game.build.join(" ");
    show::field("build", &format!("running {named}"));
    let since = project::written_now().map_err(|error| {
        fail(
            &format!("steamship could not mark when the build starts: {error}"),
            FAILED,
        )
    })?;
    let started = SystemTime::now();
    let finished = run::spawn(
        run::command(program)
            .args(args)
            .current_dir(&game.root)
            .stdin(process::Stdio::null())
            // Standard output is kept for what steamship itself answers, such as --json's line.
            .stdout(io::stderr()),
    )
    .and_then(|mut child| child.wait());
    let took = show::took(started.elapsed().unwrap_or_default());
    match finished {
        Ok(status) if status.success() => {}
        Ok(status) => {
            return Err(fail(
                &format!("{named} failed after {took} ({status}), so nothing goes up"),
                FAILED,
            ));
        }
        Err(error) => {
            return Err(fail(
                &format!("{program} could not be started: {error}"),
                FAILED,
            ));
        }
    }
    project::unchanged(&game.root).map_err(|why| fail(&why, REFUSED))?;
    let unwritten = project::unwritten(&game.script, since);
    if !unwritten.is_empty() {
        return Err(fail(
            &project::left_over(&unwritten, &game.root, &named),
            REFUSED,
        ));
    }
    show::done("build", &format!("built in {took}"));
    Ok(())
}

/// Refuses, naming every problem, when `check` found any. Otherwise shows what was checked, and
/// answers whether the branch the script sets live can be. The content is checked against the
/// settings snapshot beside the script when there is one. With `scripts_only`, it is left
/// unchecked.
pub(crate) fn checked(
    script: &Path,
    scripts_only: bool,
) -> Result<(check::Report, Branch), ExitCode> {
    let report = if scripts_only {
        check::scripts_only(script)
    } else {
        let settings = snapshot_beside(script, scripts::app_named(script))?;
        check::against(script, settings.as_ref())
    };
    if report.problems.is_empty() {
        let what = if report.content {
            let files = report.depots.iter().map(|depot| depot.files.len()).sum();
            format!("{}, checked", show::counted(files, "file"))
        } else {
            "scripts checked".to_owned()
        };
        show::done(
            "app",
            &format!(
                "{}, {}, {what}",
                report.app_id.unwrap_or_default(),
                show::counted(report.depots.len(), "depot"),
            ),
        );
        let branch = match (report.app_id, report.set_live.as_deref()) {
            (Some(app), Some(branch)) => live_branch_found(script, app, branch)?,
            _ => Branch::Ready,
        };
        return Ok((report, branch));
    }
    for problem in &report.problems {
        show::failure("refused", &problem.to_string(), None);
    }
    Err(ExitCode::from(REFUSED))
}

/// The settings snapshot beside `script`, for `app`, when there is one. Refused when it cannot be
/// read.
fn snapshot_beside(script: &Path, app: Option<u32>) -> Result<Option<vdf::Block>, ExitCode> {
    let path = settings::snapshot_beside(script);
    match app {
        Some(app) if path.is_file() => {
            let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
            let text = fs::read_to_string(&path).map_err(|error| refused(&error))?;
            settings::read(&text, app)
                .map(Some)
                .map_err(|why| refused(&why))
        }
        _ => Ok(None),
    }
}

/// Shows which of Steamworks' Common Redistributables the build's Windows files need, refusing
/// what nothing gives them. `unknown` says that which the app ticks was not known.
pub(crate) fn redistributables_shown(
    judged: &redist::Judged,
    unknown: bool,
) -> Result<(), ExitCode> {
    if !judged.refused.is_empty() {
        for problem in &judged.refused {
            show::failure("refused", &problem.to_string(), None);
        }
        return Err(ExitCode::from(REFUSED));
    }
    if !judged.ticked.is_empty() {
        let ticked: Vec<&str> = judged.ticked.iter().copied().collect();
        show::done(
            "redist",
            &format!("{}, ticked in Steamworks", ticked.join(", ")),
        );
    }
    if !judged.unchecked.is_empty() {
        show::field(
            "redist",
            &format!(
                "{}, not checked",
                show::counted(judged.unchecked.len(), "DLL")
            ),
        );
        for problem in &judged.unchecked {
            show::aside(&problem.to_string());
        }
        if unknown {
            show::aside(UNKNOWN_TICKS);
        }
    }
    Ok(())
}

/// How to have checked which redistributables an app ticks.
const UNKNOWN_TICKS: &str = "upload asks Steam which redistributables the app ticks, and check \
                             reads them from a settings snapshot beside the script";

/// Why an unchecked `SetLive` branch matters, and how to have it checked.
const UNCHECKED_BRANCH: &str = "a branch the app does not have leaves the build uploaded but not \
                                live. steamship login --web-api-key-only keeps a key to check it with";

/// Whether the branch an app build script sets live can be set live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Branch {
    /// Found on Steam, or left for Steam to judge.
    Ready,
    /// Not there yet: the app has no build live on its default branch, and Steam gives it no other
    /// branch until it has.
    Unbranched,
}

/// Refuses a `SetLive` branch the app does not have, which Valve would otherwise report only
/// after the whole upload. Steam is asked only with a key already at hand, never one typed for
/// the purpose, and when it cannot answer the upload is not held up by it.
fn live_branch_found(script: &Path, app: u32, branch: &str) -> Result<Branch, ExitCode> {
    let Some(api) = key_at_hand()? else {
        show::field(
            "branch",
            &format!("{branch}, not checked: no Web API key at hand"),
        );
        show::aside(UNCHECKED_BRANCH);
        return Ok(Branch::Ready);
    };
    let spinner = Spinner::start("branch", "asking Steam for the app's branches", false);
    match api.branches(app) {
        Ok(branches)
            if branches
                .iter()
                .any(|found| found.name.eq_ignore_ascii_case(branch)) =>
        {
            spinner.done(&format!("{branch}, found on Steam"));
            Ok(Branch::Ready)
        }
        Ok(branches) => {
            spinner.failed("not on Steam");
            let problem = scripts::Problem::new(script, upload::no_branch(app, branch, &branches));
            show::failure("refused", &problem.to_string(), None);
            Err(ExitCode::from(REFUSED))
        }
        Err(error) => {
            if why_unanswered(&api, app, &error).is_some() {
                spinner.skipped(&format!("{branch}, not on Steam yet"));
                show::aside(&format!(
                    "app {app} has no build live on its default branch, and Steam gives it no \
                     other branch until it has. An upload with --default sets the build live on \
                     default alone. Then make {branch} in Steamworks under SteamPipe, Builds"
                ));
                return Ok(Branch::Unbranched);
            }
            spinner.failed("not checked");
            show::aside(&error.to_string());
            Ok(Branch::Ready)
        }
    }
}

/// `check` of `script`, or of `game`'s script, whose content waits for its upload to build it.
fn run_check(script: &Path, scripts_only: bool, game: Option<&Project>) -> ExitCode {
    show::title("check");
    show::field("script", &script.display().to_string());
    match checked(script, scripts_only).and_then(|(report, _)| {
        if let Some(game) = game {
            game.live_on(report.set_live.as_deref())
                .map_err(|why| fail(&why, REFUSED))?;
        }
        if report.content {
            let snapshot = snapshot_beside(script, report.app_id)?;
            redistributables_shown(
                &redist::judged(&report, snapshot.as_ref()),
                snapshot.is_none(),
            )?;
        }
        Ok(report)
    }) {
        Ok(report) => {
            match (report.content, game) {
                (true, _) => {}
                (false, Some(_)) => show::field(
                    "content",
                    "not checked (upload checks it once the game is built)",
                ),
                (false, None) => show::field("content", "not checked (--scripts-only)"),
            }
            for depot in report.depots.iter().filter(|_| report.content) {
                let symbols = match depot.debug_symbols {
                    0 => String::new(),
                    count => format!(", {count} of them debug symbols"),
                };
                show::field(
                    "depot",
                    &format!(
                        "{}, {}{symbols}",
                        depot.depot_id,
                        show::counted(depot.files.len(), "file")
                    ),
                );
            }
            show::success("nothing refused", "");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn run_check_folder(folder: &Path, scripts_only: bool, demo: bool) -> ExitCode {
    show::title("check");
    show::field("folder", &folder.display().to_string());
    either(try_check_folder(folder, scripts_only, demo))
}

/// Checks the build in `folder` on its own, as soon as it is built. In a game's folder its
/// Windows files are judged against the redistributables its settings snapshot ticks: the demo's
/// with `demo`, or when only the demo's depots ship the folder, as a demo can tick fewer than the
/// full game. Its rich presence files are read as `rich-presence` sends them.
fn try_check_folder(folder: &Path, scripts_only: bool, demo: bool) -> Result<ExitCode, ExitCode> {
    if scripts_only {
        return Err(fail(
            &format!(
                "{} is a build's folder, which holds no scripts. Give --scripts-only the app \
                 build script",
                folder.display()
            ),
            REFUSED,
        ));
    }
    let here = env::current_dir()
        .map_err(|error| fail(&format!("the current folder: {error}"), FAILED))?;
    let mut report = check::folder(folder, None);
    let (app, snapshot, presence) = match Project::find(&here).map_err(|why| fail(&why, REFUSED))? {
        Some(game) => {
            game.pinned(env!("CARGO_PKG_VERSION"))
                .map_err(|why| fail(&why, REFUSED))?;
            let game = if demo || demos_alone(&game, &here, &report) {
                game.demo().map_err(|why| fail(&why, REFUSED))?
            } else {
                game
            };
            // A push gate checks a build offline, so the rich presence the build will show is
            // read here too rather than left to the check that asks Steam.
            let presence = game.presence().map_err(|why| fail(&why, REFUSED))?;
            // The build is checked before the app exists too, so an app script naming none is
            // left to the commands that upload.
            if let Some(app) = scripts::app_named(&game.script) {
                let path = game.snapshot();
                let snapshot = if path.is_file() {
                    Some(settings_file(&path, app)?)
                } else {
                    None
                };
                (Some(app), snapshot, presence)
            } else {
                show::field(
                    "app",
                    &format!("not named yet in {}", game.script.display()),
                );
                (None, None, presence)
            }
        }
        None if demo => {
            return Err(fail(
                &format!(
                    "{} is in no game's folder, so it has no demo. Run the check in the game's \
                     folder, or leave out --demo",
                    here.display()
                ),
                REFUSED,
            ));
        }
        None => (None, None, Vec::new()),
    };
    report.app_id = app;
    if let (Some(app), Some(settings)) = (app, &snapshot) {
        report
            .problems
            .extend(check::folder_manifest(folder, app, settings));
        report
            .problems
            .extend(check::folder_launch(folder, app, settings));
    }
    let presence_refused = presence_read(&presence).err();
    if !report.problems.is_empty() || presence_refused.is_some() {
        for problem in &report.problems {
            show::failure("refused", &problem.to_string(), None);
        }
        if let Some(why) = &presence_refused {
            show::failure("refused", why, None);
        }
        return Err(ExitCode::from(REFUSED));
    }
    for build in &report.depots {
        let symbols = match build.debug_symbols {
            0 => String::new(),
            count => format!(", {count} of them debug symbols"),
        };
        show::done(
            "build",
            &format!(
                "{}{symbols}, {} for Windows, checked",
                show::counted(build.files.len(), "file"),
                build.windows.len()
            ),
        );
    }
    if !presence.is_empty() {
        show::done(
            "presence",
            &format!(
                "{}, checked",
                show::counted(presence.len(), "rich presence file")
            ),
        );
    }
    redistributables_shown(
        &redist::judged(&report, snapshot.as_ref()),
        snapshot.is_none(),
    )?;
    show::success("nothing refused", "");
    Ok(ExitCode::SUCCESS)
}

/// Whether the build `found` in a folder under `here` is the demo's alone: some of its files are
/// shipped by the depots of `game`'s demo, and none by the full game's.
fn demos_alone(game: &Project, here: &Path, found: &check::Report) -> bool {
    let Some(demo) = &game.demo else {
        return false;
    };
    let files: Vec<PathBuf> = found
        .depots
        .iter()
        .flat_map(|build| &build.files)
        .map(|file| scripts::resolve(here, &file.to_string_lossy()))
        .collect();
    let ships = |script: &Path| {
        scripts::load(script).is_ok_and(|app| {
            files
                .iter()
                .any(|file| app.depots.iter().any(|depot| depot.ships(file)))
        })
    };
    ships(&demo.script) && !ships(&game.script)
}
