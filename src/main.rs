//! The command line.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::panic;
use std::path::{self, Path, PathBuf};
use std::process::{self, ExitCode};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use clap::{CommandFactory as _, FromArgMatches as _};
use steamship::account::Account;
use steamship::cli::{self, Cli, Command, FAILED, LOGIN, REFUSED, STEAMCMD, Writing};
use steamship::install::{self, Outcome, State};
use steamship::login::{self, Ending, Person};
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::project::{self, Project};
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
#[cfg(unix)]
use steamship::unix::Terminal;
use steamship::update::{self, Installed};
#[cfg(windows)]
use steamship::windows::Terminal;
use steamship::{
    achievements, assets, check, ci, conversation, digest, download, drm, dump, editor, fetch,
    init, itself, keychain, leaderboards, likeness, magic, picture, po, presence, presets, redist,
    release, run, scripts, settings, steamcmd, storefront, text, upload, vdf, webapi, workshop,
};
use zeroize::Zeroizing;

fn main() -> ExitCode {
    let mut cli = Cli::command();
    if io::stdout().is_terminal() {
        cli = cli.before_help(show::banner());
    }
    let parsed = cli
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let command = match parsed {
        Ok(parsed) => parsed.command,
        Err(error) => error.exit(),
    };
    // Completion is text for a shell to load, often at every start, so it asks nothing online.
    let update = if matches!(
        command,
        Command::Completions { .. } | Command::Languages { .. }
    ) {
        None
    } else {
        update_check()
    };
    let code = run(command);
    run::end_as_told();
    if let Some(latest) = update.and_then(update::Check::newer) {
        show::upgrade(
            &latest.to_string(),
            env!("CARGO_PKG_VERSION"),
            installed().hint(),
        );
    }
    code
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "steamship", &mut io::stdout());
            ExitCode::SUCCESS
        }
        Command::Check {
            script,
            scripts_only,
            demo,
        } => check_in(script.as_deref(), scripts_only, demo),
        Command::Assets { folder } => run_assets(&folder),
        Command::Ci {
            script,
            repo,
            account,
            secret,
            output,
        } => either(try_ci(&Setup {
            script: script.as_deref(),
            repo: repo.as_deref(),
            account: named(account.as_deref()),
            secret: &secret,
            output: output.as_deref(),
        })),
        Command::Install => run_install(),
        Command::Init {
            app,
            depots,
            folder,
        } => run_init(app, depots, &folder),
        Command::Builds {
            app,
            demo,
            count,
            json,
        } => either(app_or_game(app, demo).and_then(|app| try_builds(&app, count, json))),
        Command::Fetch(request) => either(try_fetch(&request)),
        Command::Achievements {
            app,
            check,
            writing,
            json,
        } => either(achievements_in(app, check, writing, json)),
        Command::DrmWrap {
            app,
            executable,
            output,
            compatibility,
            account,
        } => either(try_drm_wrap(
            &app,
            &executable,
            output.as_deref(),
            drm::Mode::chosen(compatibility),
            named(account.as_deref()),
        )),
        Command::Settings {
            app,
            demo,
            save,
            check,
            account,
        } => either(settings_in(
            app,
            demo,
            save.map(Snapshot::given),
            check.map(Snapshot::given),
            named(account.as_deref()),
        )),
        Command::Leaderboards(request) => either(leaderboards_in(request)),
        Command::RichPresence {
            app,
            demo,
            files,
            preview,
        } => either(rich_presence_in(app, demo, files, preview)),
        Command::Text(request) => either(text_in(request)),
        Command::Languages { json } => run_languages(json),
        Command::Itself(cli::Itself::Check) => either(try_self_check()),
        Command::Promote(request) => either(promote_in(request)),
        Command::Branch {
            app,
            branch,
            description,
        } => either(try_branch(&app, &branch, &description)),
        Command::Workshop(request) => {
            either(if request.script.is_none() && request.item.app.is_none() {
                try_game_workshop(&request)
            } else {
                try_workshop(&request)
            })
        }
        Command::Login {
            web_api_key: true, ..
        } => either(try_key_login()),
        Command::Login { account, .. } => either(try_login(named(account.as_deref()))),
        Command::Logout => run_logout(),
        Command::Status { account } => either(try_status(named(account.as_deref()))),
        Command::Upload(request) => upload_in(&request),
    }
}

/// `check` of the build's folder or the script given, or of the game, or its demo, when there is
/// neither.
fn check_in(script: Option<&Path>, scripts_only: bool, demo: bool) -> ExitCode {
    match script {
        Some(folder) if folder.is_dir() => run_check_folder(folder, scripts_only, demo),
        Some(script) if demo => fail(
            &format!(
                "{} is an app build script, which names its own app; give --demo a build's \
                 folder, or nothing in the game's folder",
                script.display()
            ),
            REFUSED,
        ),
        Some(script) => run_check(script, scripts_only, None),
        None => either(try_game_check(demo)),
    }
}

/// `upload` of the script given, or of the game when there is none.
fn upload_in(request: &cli::Upload) -> ExitCode {
    let shipping = Shipping {
        version: request.version.as_deref(),
        preview: request.preview,
        account: named(request.account.as_deref()),
        json: request.json,
        demo: request.demo,
    };
    let default = request.default;
    match (request.script.as_deref(), shipping.version) {
        (Some(script), Some(version)) => {
            run_upload(&Upload {
                script,
                version,
                preview: shipping.preview,
                default,
                account: shipping.account,
                json: shipping.json,
            })
            .0
        }
        (Some(script), None) => fail(
            &format!(
                "give the version {} goes up as with --version, or run steamship upload with no \
                 script in the game's folder, which takes project.godot's",
                script.display()
            ),
            REFUSED,
        ),
        (None, _) => either(try_game_upload(&shipping)),
    }
}

/// The game the current folder is in: refused when there is none, its `steamship.toml` says what
/// steamship does not take, or it pins another steamship.
fn game() -> Result<Project, ExitCode> {
    let here = env::current_dir()
        .map_err(|error| fail(&format!("the current folder: {error}"), FAILED))?;
    let found = Project::find(&here)
        .map_err(|why| fail(&why, REFUSED))?
        .ok_or_else(|| {
            fail(
                &format!(
                    "there is no game in {}: no {} or steam/app_build.vdf in it or a folder \
                     above. Run steamship in the game's folder, start one here with \
                     `steamship init <app ID>`, or name the app and its files",
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
fn game_as(demo: bool) -> Result<Project, ExitCode> {
    let found = game()?;
    if demo {
        found.demo().map_err(|why| fail(&why, REFUSED))
    } else {
        Ok(found)
    }
}

/// The app as `app` names it, or else as the game the current folder is in, or with `demo` its
/// demo: its build script.
fn app_or_game(app: Option<String>, demo: bool) -> Result<String, ExitCode> {
    app.map_or_else(|| game_as(demo).map(|found| script_named(&found)), Ok)
}

/// The game's app build script, as an app is named on the command line.
fn script_named(game: &Project) -> String {
    game.script.display().to_string()
}

/// The game's app build script, read, every problem in it shown, and refused unless it sets the
/// game's branch live.
fn game_script(game: &Project) -> Result<scripts::AppScript, ExitCode> {
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
fn game_achievements(game: &Project) -> Result<Vec<PathBuf>, ExitCode> {
    let refused = |why: String| fail(&why, REFUSED);
    let mut files = game.records(project::ACHIEVEMENTS).map_err(refused)?;
    if !files.is_empty() {
        files.extend(game.catalogues().map_err(refused)?);
    }
    Ok(files)
}

/// Shows the game, and refuses it unless its tree holds nothing uncommitted and origin's default
/// branch is at its commit.
fn shipped_from(game: &Project) -> Result<(), ExitCode> {
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

/// Refuses the game's settings snapshot when it shows `script`'s app unfit for it: an install
/// folder of anything but letters and digits, a depot of the script's not among the app's own, a
/// system the app's depots install on that no launch option runs on, or, for a demo, an app not
/// typed Demo with the full game as its base game, or without the Steam Input defaults the full
/// game's snapshot shows. Refused too when there is none and one is `needed`.
fn snapshot_sound(
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
                    "{} is missing: run steamship settings --save in the game and commit it, so a \
                     build goes up only against the settings it was made for",
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
            "{} uploads depot {depot}, which is not among app {app}'s own depots: a depot goes up \
             in the app it was made under; if it is new, publish it in Steamworks and save the \
             settings again",
            game.script.display()
        )
    }));
    wrong.extend(settings::unlaunched(&snapshot).into_iter().map(|system| {
        format!(
            "app {app} has a depot for {system}, and no launch option runs on {system}: \
                     add one under Installation, General Installation in Steamworks, then save \
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
/// into the other's; nothing when the game has no demo.
///
/// The full game is shipped whatever is wrong with the demo's script, such as an app ID of 0
/// held in place until the demo's app exists: steamship cannot upload that script, so it cannot
/// take the full game's place. The demo's own check and upload refuse it.
fn apart(game: &Project) -> Result<(), ExitCode> {
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
            "{} names app {}, the full game's own in {}: a demo is its own app, so put the \
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
fn exports_apart(game: &Project, script: &scripts::AppScript) -> Result<(), ExitCode> {
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
            .filter(|depot| check::ships(depot, &preset.export))
        {
            let (whose, made) = if demo {
                ("the demo's", "without")
            } else {
                ("the full game's", "with")
            };
            wrong.push(format!(
                "{} ships {} into depot {}, {whose}, and the export preset \"{}\" in {} makes it \
                 {made} the `{}` feature: map only the folders {whose} presets export to",
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

/// The first code of `codes` that is not success, or success.
fn first_failure(codes: impl IntoIterator<Item = ExitCode>) -> ExitCode {
    codes
        .into_iter()
        .find(|code| *code != ExitCode::SUCCESS)
        .unwrap_or(ExitCode::SUCCESS)
}

/// Compares the game's achievements, leaderboards and rich presence with Steam's, each that the
/// game has, every one whatever the others found.
fn compared(game: &Project, app: &str) -> Result<ExitCode, ExitCode> {
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
                "no achievement, stat or set records in {}: name the game's in {}'s records",
                records(),
                project::FILE
            ),
        );
    } else {
        let receipt = game.import.join(editor::RECEIPT).is_file();
        let writing = Writing {
            write: None,
            out: receipt.then(|| game.import.clone()),
            unchanged: false,
            imported: false,
        };
        codes.push(either(try_achievements(
            app,
            &achievements,
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
        codes.push(either(try_leaderboards(app, &boards, Changes::default())));
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

/// How `upload` ships a game.
struct Shipping<'command> {
    version: Option<&'command str>,
    preview: bool,
    account: Option<&'command str>,
    json: bool,
    demo: bool,
}

/// `upload` in a game: the pushed commit, built, checked against Steam, uploaded, and its branch
/// described.
fn try_game_upload(shipping: &Shipping<'_>) -> Result<ExitCode, ExitCode> {
    if shipping.json {
        show::divert();
    }
    let game = game_as(shipping.demo)?;
    shipped_from(&game)?;
    let version = match shipping.version {
        Some(version) => version.to_owned(),
        None => game.version().map_err(|why| fail(&why, REFUSED))?,
    };
    show::field("version", &version);
    let script = game_script(&game)?;
    apart(&game)?;
    exports_apart(&game, &script)?;
    snapshot_sound(&game, &script, true)?;
    built(&game)?;
    let app = script_named(&game);
    let snapshot = game.snapshot();
    let steps = [
        either(try_settings(&app, None, Some(&snapshot), shipping.account)),
        compared(&game, &app)?,
    ];
    let compared = first_failure(steps);
    if compared != ExitCode::SUCCESS {
        return Ok(compared);
    }
    let (uploaded, held_back) = run_upload(&Upload {
        script: &game.script,
        version: &version,
        preview: shipping.preview,
        default: !shipping.preview,
        account: shipping.account,
        json: shipping.json,
    });
    if uploaded != ExitCode::SUCCESS || shipping.preview {
        return Ok(uploaded);
    }
    if let Some(branch) = &script.set_live {
        let description = game.description.as_deref().unwrap_or(&version);
        if held_back.is_some() {
            show::hint(Hint {
                before: &format!("once it is made, describe {branch} with "),
                command: &format!(
                    "steamship branch {} {branch} --description {description:?}",
                    script.app_id
                ),
                after: "",
            });
        } else {
            let described = either(try_branch(&app, branch, description));
            if described != ExitCode::SUCCESS {
                return Ok(described);
            }
        }
    }
    Ok(either(try_builds(&app, 10, false)))
}

/// Runs the game's `build` command in its folder, its output shown as it comes, and refuses when
/// it fails or leaves the tree changed.
fn built(game: &Project) -> Result<(), ExitCode> {
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

/// `achievements --write --unchanged` in `game`, which holds no achievement records: no import is
/// due, so it passes unless the import's folder still holds a table or an icon.
fn no_import_due(game: &Project) -> Result<ExitCode, ExitCode> {
    show::title("achievements, unchanged");
    let folder = &game.import;
    let mut left: Vec<String> = match fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                Path::new(name).extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("csv") || extension.eq_ignore_ascii_case("png")
                })
            })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(fail(&format!("{}: {error}", folder.display()), FAILED)),
    };
    if left.is_empty() {
        show::success(
            "the game holds no achievement records, so there is no import to write",
            "",
        );
        return Ok(ExitCode::SUCCESS);
    }
    left.sort();
    for name in &left {
        show::failure(&format!("{name} is left over"), "", None);
    }
    show::field(
        "next",
        &format!(
            "the game holds no achievement records, so remove these from {}",
            folder.display()
        ),
    );
    Ok(ExitCode::from(REFUSED))
}

/// `achievements` with whatever is not given taken from the game.
fn achievements_in(
    app: Option<String>,
    check: Option<Vec<PathBuf>>,
    mut writing: Writing,
    json: bool,
) -> Result<ExitCode, ExitCode> {
    let own_check = check.as_ref().is_some_and(Vec::is_empty);
    let own_write = writing.write.as_ref().is_some_and(Vec::is_empty)
        || (writing.imported && writing.write.is_none());
    let game = (app.is_none() || own_check || own_write)
        .then(game)
        .transpose()?;
    let own = |found: &Project, flag: &str| {
        let files = game_achievements(found)?;
        if files.is_empty() {
            return Err(fail(
                &format!(
                    "{flag} names no files, and the game holds no achievement records for it to \
                     read"
                ),
                REFUSED,
            ));
        }
        Ok(files)
    };
    let mut files = check.unwrap_or_default();
    if let Some(game) = &game {
        if own_check {
            files = own(game, "--check")?;
            let receipt = game.import.join(editor::RECEIPT);
            writing.out = writing
                .out
                .take()
                .or_else(|| receipt.is_file().then(|| game.import.clone()));
        }
        if own_write {
            if writing.unchanged && game_achievements(game)?.is_empty() {
                return no_import_due(game);
            }
            writing.write = Some(own(game, "--write")?);
            if writing.out.is_none() {
                writing.out = Some(game.import.clone());
            }
        }
    }
    if writing.write.is_some() && writing.out.is_none() {
        return Err(fail(
            &"--write needs --out, the folder it writes into",
            REFUSED,
        ));
    }
    try_achievements(&app_given_or(app, game.as_ref()), &files, &writing, json)
}

/// The app as `app` names it, or else as the game's build script does; `game` is there whenever
/// `app` is not.
fn app_given_or(app: Option<String>, game: Option<&Project>) -> String {
    app.or_else(|| game.map(script_named)).unwrap_or_default()
}

/// The snapshot `--save` or `--check` names: a file, or the game's own when it names none.
enum Snapshot {
    File(PathBuf),
    Game,
}

impl Snapshot {
    fn given(path: Option<PathBuf>) -> Self {
        path.map_or(Self::Game, Self::File)
    }
}

/// `promote` of the app named, or else the game's.
fn promote_in(mut request: cli::Promote) -> Result<ExitCode, ExitCode> {
    request.app = Some(app_or_game(request.app.take(), request.demo)?);
    try_promote(&request)
}

/// `settings` with whatever is not given taken from the game.
fn settings_in(
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

/// What `leaderboards` changes on Steam before it compares.
#[derive(Debug, Clone, Copy, Default)]
struct Changes<'command> {
    create: bool,
    delete_missing: bool,
    account: Option<&'command str>,
}

/// `leaderboards` with whatever is not given taken from the game.
fn leaderboards_in(request: cli::Leaderboards) -> Result<ExitCode, ExitCode> {
    let cli::Leaderboards {
        app,
        check,
        create,
        delete_missing,
        account,
    } = request;
    let changes = Changes {
        create,
        delete_missing,
        account: named(account.as_deref()),
    };
    let own =
        check.as_ref().is_none_or(Vec::is_empty) && (check.is_some() || create || delete_missing);
    let game = (app.is_none() || own).then(game).transpose()?;
    let mut files = check.unwrap_or_default();
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
    try_leaderboards(&app_given_or(app, game.as_ref()), &files, changes)
}

/// `rich-presence` with whatever is not given taken from the game: every language's file, sent
/// only from a pushed tree.
fn rich_presence_in(
    app: Option<String>,
    demo: bool,
    mut files: Vec<PathBuf>,
    preview: bool,
) -> Result<ExitCode, ExitCode> {
    let game = (app.is_none() || files.is_empty())
        .then(|| game_as(demo))
        .transpose()?;
    if let Some(game) = game.as_ref().filter(|_| files.is_empty()) {
        files = game.presence().map_err(|why| fail(&why, REFUSED))?;
        if files.is_empty() {
            show::title("rich-presence");
            return Err(fail(
                &format!(
                    "the game holds no rich_presence_<language>.vdf beside {}: write \
                     rich_presence_english.vdf there, or name the files to send",
                    game.script.display()
                ),
                REFUSED,
            ));
        }
        if !preview {
            shipped_from(game)?;
        }
    }
    try_rich_presence(&app_given_or(app, game.as_ref()), &files, preview)
}

/// `workshop` in a game: each example mod, or the one `--name` names, from a pushed tree.
fn try_game_workshop(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    let game = game()?;
    let mods = game.mods().map_err(|why| fail(&why, REFUSED))?;
    let chosen: Vec<project::Mod> = match &request.item.name {
        Some(name) => mods.into_iter().filter(|each| each.name == *name).collect(),
        None => mods,
    };
    if chosen.is_empty() {
        return Err(fail(
            &request.item.name.as_ref().map_or_else(
                || {
                    format!(
                        "the game has no example mod: no folder in {} holds a manifest.json",
                        game.mods.path.display()
                    )
                },
                |name| format!("the game has no example mod named {name}"),
            ),
            REFUSED,
        ));
    }
    if request.json {
        show::divert();
    }
    shipped_from(&game)?;
    let app = script_named(&game);
    // A file that cannot be read is named by the upload itself; until then no item counts as made.
    let made = fs::read_to_string(&game.items)
        .ok()
        .and_then(|text| workshop::Record::read(&text).ok())
        .map(|record| record.items)
        .unwrap_or_default();
    for each in chosen {
        // Visibility is chosen once, for an item being made: a published item keeps whatever it
        // was given on Steam since.
        let visibility = request
            .item
            .visibility
            .filter(|_| !made.contains_key(&each.name));
        let item = cli::Workshop {
            script: None,
            item: cli::WorkshopItem {
                app: Some(app.clone()),
                items: Some(game.items.clone()),
                name: Some(each.name),
                content: Some(each.folder),
                preview: Some(each.preview),
                title: Some(each.title),
                description: Some(each.description),
                change_note: Some(format!("Version {}", each.version)),
                visibility,
            },
            account: request.account.clone(),
            new: request.new,
            json: request.json,
        };
        let uploaded = either(try_workshop(&item));
        if uploaded != ExitCode::SUCCESS {
            return Ok(uploaded);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The code a command exits with, whether it finished or stopped early.
const fn either(result: Result<ExitCode, ExitCode>) -> ExitCode {
    match result {
        Ok(code) | Err(code) => code,
    }
}

/// How this steamship was installed, which is how it is upgraded.
fn installed() -> Installed {
    // Homebrew and winget run steamship through a link to where they keep it.
    env::current_exe().map_or(Installed::Archive, |program| {
        Installed::of(&fs::canonicalize(&program).unwrap_or(program), Path::exists)
    })
}

/// The account named with `--account` or `STEAMSHIP_ACCOUNT`, where an empty name is none: a CI
/// hands a secret it does not have over as an empty variable.
fn named(account: Option<&str>) -> Option<&str> {
    account.filter(|name| !name.is_empty())
}

/// Whether a newer steamship is out, found out while the command runs, when a person at a
/// terminal would be told.
fn update_check() -> Option<update::Check> {
    if !update::wanted(|name| env::var_os(name), io::stderr().is_terminal()) {
        return None;
    }
    let home = Platform::THIS.home(|name| env::var_os(name)).ok()?;
    Some(update::Check::start(home))
}

fn home() -> Result<PathBuf, ExitCode> {
    Platform::THIS
        .home(|name| env::var_os(name))
        .map_err(|error| fail(&error, FAILED))
}

/// The home, held, waiting for any other steamship using it to finish first.
fn hold(home: &Path) -> Result<install::Held, ExitCode> {
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
    held.map_err(|error| steamcmd_failed(&error))
}

/// The home, held until the returned [`install::Held`] is dropped, and the pinned steamcmd in it,
/// installed or verified.
fn ready() -> Result<(install::Held, PathBuf, Manifest), ExitCode> {
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
                install::Error::Altered { .. } => "not as pinned",
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

fn run_init(app: u32, mut depots: Vec<init::Depot>, folder: &Path) -> ExitCode {
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
            &format!("{} is already there, and is left as it is", there.display()),
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

fn run_install() -> ExitCode {
    show::title("install");
    match ready() {
        Ok((_held, home, _)) => {
            show::field("folder", &home.join(install::FOLDER).display().to_string());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn fail(error: &dyn Display, code: u8) -> ExitCode {
    show::failure(&error.to_string(), "", None);
    ExitCode::from(code)
}

fn steamcmd_failed(error: &install::Error) -> ExitCode {
    let code = if matches!(error, install::Error::Altered { .. }) {
        STEAMCMD
    } else {
        FAILED
    };
    show::failure(&error.to_string(), "", None);
    if let Some(remedy) = error.remedy() {
        show::note(remedy);
        show::hint(installed().hint());
    }
    ExitCode::from(code)
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
            &"name the build account with --account or STEAMSHIP_ACCOUNT",
            REFUSED,
        )),
        Err(error) => Err(fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )),
    }
}

/// What steamship shows while it waits for the login to be approved.
const APPROVING: &str = "waiting for you in the Steam Mobile app";

/// The person at the terminal, answering steamcmd through steamship's own prompts.
#[derive(Debug, Default)]
struct Typist {
    approving: Option<Spinner>,
    /// steamcmd used the login it saved, so nothing was asked.
    saved: bool,
}

impl Typist {
    fn ask(&mut self, label: &str, echo: Echo) -> io::Result<Zeroizing<String>> {
        drop(self.approving.take());
        typing::ask(label, echo)
    }
}

impl Person for Typist {
    fn password(&mut self) -> io::Result<Zeroizing<String>> {
        self.ask("password", Echo::Dots)
    }

    fn code(&mut self) -> io::Result<Zeroizing<String>> {
        self.ask("code", Echo::Typed)
    }

    fn approving(&mut self) {
        if self.approving.is_none() {
            self.approving = Some(Spinner::start("approve", APPROVING, true));
        }
    }

    fn approved(&mut self) {
        match self.approving.take() {
            Some(spinner) => spinner.done("approved"),
            None => show::done("approve", "approved"),
        }
    }

    fn saved(&mut self) {
        self.saved = true;
    }

    fn said(&mut self, line: &str) {
        if let Some(spinner) = self.approving.take() {
            self.approving = Some(spinner.around(|| show::aside(line), APPROVING, true));
        } else {
            show::aside(line);
        }
    }
}

/// `login` failed, for `reason`, and can be tried again.
fn not_logged_in(reason: &str) -> ExitCode {
    let again = Hint {
        before: "run ",
        command: "steamship login",
        after: " to try again",
    };
    show::failure("not logged in", reason, Some(again));
    ExitCode::from(FAILED)
}

fn run_logout() -> ExitCode {
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
    show::success(
        "logged out",
        "the next upload needs `steamship login` first",
    );
    ExitCode::SUCCESS
}

/// What the steamcmd in `home` is, shown without installing or changing anything.
fn steamcmd_state(home: &Path, manifest: &Manifest) -> Result<(), ExitCode> {
    match install::state(home, manifest) {
        Ok(State::Pinned) => show::done(
            "steamcmd",
            &format!(
                "{} ({}), pinned and verified",
                manifest.version, manifest.system
            ),
        ),
        Ok(State::Earlier) => show::field(
            "steamcmd",
            "an earlier pin; the next login or upload replaces it",
        ),
        Ok(State::Missing) => {
            show::field("steamcmd", "not installed; `steamship login` installs it");
        }
        Err(error) => {
            show::failed("steamcmd", "not as pinned");
            return Err(steamcmd_failed(&error));
        }
    }
    Ok(())
}

/// The login a CI handed over in `STEAMSHIP_LOGIN`, packed by `steamship ci`, if it did.
fn packed_login() -> Result<Option<ci::Login>, ExitCode> {
    let Some(value) = env::var_os(ci::VARIABLE).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let text = value
        .to_str()
        .ok_or_else(|| fail(&ci::Error::Unreadable, REFUSED))?;
    let login = ci::Login::unpack(text).map_err(|error| fail(&error, REFUSED))?;
    // GitHub masks the secret, but not the account's name inside it, which steamcmd's logs name.
    if dump::in_actions(|name| env::var(name).ok()) {
        show::plain(&format!("::add-mask::{}\n", login.account().name()));
    }
    Ok(Some(login))
}

/// Puts a login handed over in `STEAMSHIP_LOGIN` where steamcmd looks for it in `home`.
fn restore(packed: Option<&ci::Login>, home: &Path) -> Result<(), ExitCode> {
    if let Some(packed) = packed {
        drop(
            packed
                .restore(home, Platform::THIS)
                .map_err(|error| fail(&error, FAILED))?,
        );
    }
    Ok(())
}

fn try_status(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("status");
    show::field("version", env!("CARGO_PKG_VERSION"));
    let home = home()?;
    show::field("home", &home.display().to_string());
    status_key(&home);
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => {
            show::field("account", &format!("from {}", ci::VARIABLE));
            Some(packed.account().clone())
        }
        _ => status_account(named, &home)?,
    };
    let saved = packed.is_some()
        || !steamcmd::login_files(&home)
            .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?
            .is_empty();
    let login = Hint {
        before: "run ",
        command: "steamship login",
        after: "",
    };
    let Some(account) = account.filter(|_| saved) else {
        let manifest = Manifest::pinned(Platform::THIS).map_err(|error| fail(&error, FAILED))?;
        steamcmd_state(&home, &manifest)?;
        if saved {
            show::field("login", "saved");
            show::failure("no build account", "", Some(login));
        } else {
            show::field("login", "none saved");
            show::failure("not logged in", "", Some(login));
        }
        return Ok(ExitCode::from(LOGIN));
    };
    let judged = check_saved_login(&account, packed.as_ref())?;
    if judged == upload::Login::Taken {
        show::success("ready to upload", "");
    }
    Ok(verdict(judged, packed.as_ref()))
}

/// Where the Web API commands would find the key, found out without unlocking anything.
fn status_key(home: &Path) {
    if key_variable().is_some() {
        show::field("api key", &format!("from {}", webapi::KEY));
        return;
    }
    match keychain::has(home) {
        Ok(true) => show::field("api key", &format!("kept in {}", keychain::STORE)),
        Ok(false) => show::field("api key", "none kept"),
        Err(keychain::Error::Unavailable(_)) => show::field("api key", "no credential store here"),
        Err(error @ (keychain::Error::Failed(_) | keychain::Error::NotAKey)) => {
            show::failed("api key", &error.to_string());
        }
    }
}

/// The account named, or else the one remembered, shown by where it came from and never by
/// name, as `login` shows it.
fn status_account(named: Option<&str>, home: &Path) -> Result<Option<Account>, ExitCode> {
    if let Some(name) = named {
        let account = Account::parse(name).map_err(|error| fail(&error, REFUSED))?;
        show::field("account", "as named");
        return Ok(Some(account));
    }
    let remembered = Account::remembered(home).map_err(|error| {
        fail(
            &format!("the account remembered in {}: {error}", home.display()),
            FAILED,
        )
    })?;
    show::field(
        "account",
        if remembered.is_some() {
            "remembered"
        } else {
            "none remembered"
        },
    );
    Ok(remembered)
}

/// Logs `account` in with the login steamcmd saved, or the one handed over in `packed`, as an
/// upload does, and shows what Steam made of it.
fn check_saved_login(
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<upload::Login, ExitCode> {
    let (held, home, manifest) = ready()?;
    restore(packed, &home)?;
    match packed.and_then(ci::Login::packed_on) {
        Some(day) => show::field("login", &format!("from {}, packed {day}", ci::VARIABLE)),
        None => show::field("login", "saved"),
    }
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "checking the saved login", true);
    let finished = run::run(
        &program,
        &steamcmd::check_login(account),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::CHECK_LIMIT,
        steamcmd::HOPELESS,
    );
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    let judged = upload::judge_login(finished.code, &String::from_utf8_lossy(&console));
    match &judged {
        upload::Login::Taken => spinner.done("Steam takes the saved login"),
        upload::Login::Refused(_) => spinner.failed("Steam refused the saved login"),
        upload::Login::Failed(_) => {
            spinner.failed("could not check");
            dump_in_actions(
                &[],
                &[("steamcmd's console".to_owned(), console)],
                &marks.added(&steamcmd_logs),
                &redactor,
            );
        }
    }
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(judged)
}

/// Says Steam refused the login, and how to log in again: for one handed over in
/// `STEAMSHIP_LOGIN`, with the day it was packed, since the token in it may simply have expired.
fn login_refused(reason: &str, packed: Option<&ci::Login>) -> ExitCode {
    match packed.and_then(ci::Login::packed_on) {
        None => {
            let again = Hint {
                before: "log the build account in again with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", reason, Some(again));
        }
        Some(day) => {
            let again = Hint {
                before: "log in again with ",
                command: "steamship login",
                after: ", then run steamship ci to pack the new login",
            };
            show::failure(
                "not logged in",
                &format!(
                    "{reason}; the login in {} was packed on {day}",
                    ci::VARIABLE
                ),
                Some(again),
            );
        }
    }
    ExitCode::from(LOGIN)
}

/// The exit code for what Steam made of the saved login, or the one handed over in `packed`,
/// saying why when it did not take it.
fn verdict(judged: upload::Login, packed: Option<&ci::Login>) -> ExitCode {
    match judged {
        upload::Login::Taken => ExitCode::SUCCESS,
        upload::Login::Refused(reason) => login_refused(&reason, packed),
        upload::Login::Failed(reasons) => {
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            ExitCode::from(FAILED)
        }
    }
}

fn try_login(named: Option<&str>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("login");
    let (account, source) = account(named, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    let (held, home, manifest) = ready()?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let environment = steamcmd::environment(&home, Platform::THIS);
    let (terminal, mut output, mut input) =
        Terminal::start(&program, &steamcmd::login(&account), &environment, &root)
            .map_err(|error| not_logged_in(&format!("{}: {error}", program.display())))?;
    let mut typist = Typist::default();
    let conversed = login::converse(&mut output, &mut input, &mut typist);
    if let Some(spinner) = typist.approving.take() {
        spinner.failed("not approved");
    }
    // Dropping the terminal unwaited ends steamcmd, which is what giving up should do.
    let ending = conversed.map_err(|error| {
        let kind = error.kind();
        not_logged_in(&if kind == io::ErrorKind::Interrupted {
            "given up".to_owned()
        } else if kind == io::ErrorKind::UnexpectedEof {
            "input ended before steamcmd had an answer".to_owned()
        } else {
            error.to_string()
        })
    })?;
    drop(input);
    let code = terminal
        .wait()
        .map_err(|error| not_logged_in(&format!("{}: {error}", program.display())))?;
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    if let Ending::Refused(reason) = ending {
        return Err(not_logged_in(&conversation::explain(&reason)));
    }
    if code != Some(0_i32) {
        let exited = code.map_or_else(
            || "steamcmd was stopped".to_owned(),
            |code| format!("steamcmd exited {code}"),
        );
        return Err(not_logged_in(&exited));
    }
    account
        .remember(&home)
        .map_err(|error| fail(&format!("{}: {error}", home.display()), FAILED))?;
    offer_key(&home);
    if typist.saved {
        show::success(
            "already logged in",
            "uploads use the saved login until it expires; `steamship logout` forgets it",
        );
    } else {
        show::success("logged in", "uploads use this login until it expires");
    }
    Ok(ExitCode::SUCCESS)
}

/// Refuses, naming every problem, when `check` found any; otherwise shows what was checked, and
/// answers whether the branch the script sets live can be. The content is checked against the
/// settings snapshot beside the script when there is one; with `scripts_only`, it is left
/// unchecked.
fn checked(script: &Path, scripts_only: bool) -> Result<(check::Report, Branch), ExitCode> {
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

/// The settings snapshot beside `script`, for `app`, when there is one; refused when it cannot be
/// read.
fn snapshot_beside(script: &Path, app: Option<u32>) -> Result<Option<vdf::Block>, ExitCode> {
    let path = script
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(settings::SNAPSHOT);
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
fn redistributables_shown(judged: &redist::Judged, unknown: bool) -> Result<(), ExitCode> {
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

/// Refuses what nothing gives the Windows files of the build `found`, which no setting in
/// Steamworks changes, and answers whether they need a redistributable, which only Steam can say
/// the app ticks.
fn needs_redistributable(found: &check::Report) -> Result<bool, ExitCode> {
    let needs = redist::judged(found, None);
    if !needs.refused.is_empty() {
        redistributables_shown(&needs, true)?;
    }
    Ok(!needs.unchecked.is_empty())
}

/// Judges what the build `found` needs against the redistributables its app ticks, as Steam
/// shows them to `account` through the steamcmd `ready` gave.
fn ticked_on_steam(
    found: &check::Report,
    (held, home, manifest): (&install::Held, &Path, &Manifest),
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<(), ExitCode> {
    let app = found.app_id.unwrap_or_default();
    let asked = app_info(
        home,
        account,
        app,
        packed,
        "asking which redistributables the app ticks",
    );
    if asked.is_err() {
        install::verify(held, manifest).map_err(|error| steamcmd_failed(&error))?;
    }
    let (console, spinner) = asked?;
    match settings::from_console(&console, app) {
        Ok(shown) => {
            spinner.done("shown");
            redistributables_shown(&redist::judged(found, Some(&shown)), false)
        }
        Err(why) => {
            spinner.failed("none shown");
            Err(fail(&why, FAILED))
        }
    }
}

/// How to have checked which redistributables an app ticks.
const UNKNOWN_TICKS: &str = "upload asks Steam which redistributables the app ticks, and check \
                             reads them from a settings snapshot beside the script";

/// Why an unchecked `SetLive` branch matters, and how to have it checked.
const UNCHECKED_BRANCH: &str = "a branch the app does not have leaves the build uploaded but not \
                                live; steamship login --web-api-key keeps a key to check it with";

/// Whether the branch an app build script sets live can be set live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
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
                     other branch until it has: an upload with --default sets the build live on \
                     default alone, and then {branch} can be made in Steamworks under SteamPipe, \
                     Builds"
                ));
                return Ok(Branch::Unbranched);
            }
            spinner.failed("not checked");
            show::aside(&error.to_string());
            Ok(Branch::Ready)
        }
    }
}

/// The partner Web API, when a key is set in `STEAMSHIP_WEB_API_KEY` or kept; none otherwise,
/// without asking for one.
fn key_at_hand() -> Result<Option<webapi::Api>, ExitCode> {
    if let Some(text) = key_variable() {
        let key = webapi::Key::parse(&text).map_err(|_not_a_key| {
            fail(
                &format!(
                    "{} does not hold a publisher Web API key, which is 32 hexadecimal digits",
                    webapi::KEY
                ),
                REFUSED,
            )
        })?;
        return Ok(Some(webapi::Api::new(key)));
    }
    // Asked first without reading it, which no store prompts for; a store holding none is then
    // never asked to unlock, so a check stays free of prompts for anyone who kept no key.
    let kept = Platform::THIS
        .home(|name| env::var_os(name))
        .ok()
        .filter(|home| keychain::has(home).unwrap_or(false))
        .and_then(|home| keychain::kept(&home).ok().flatten());
    Ok(kept.map(webapi::Api::new))
}

struct Upload<'command> {
    script: &'command Path,
    version: &'command str,
    preview: bool,
    default: bool,
    account: Option<&'command str>,
    json: bool,
}

/// Uploads as `upload` asks, and answers the exit code with the branch the script sets live that
/// the upload held back from, as the app cannot have it yet.
fn run_upload(upload: &Upload<'_>) -> (ExitCode, Option<String>) {
    match try_upload(upload) {
        Ok(uploaded) => uploaded,
        Err(code) => (code, None),
    }
}

fn try_upload(request: &Upload<'_>) -> Result<(ExitCode, Option<String>), ExitCode> {
    if request.json {
        show::divert();
    }
    show::title(if request.preview {
        "upload, preview"
    } else {
        "upload"
    });
    let packed = packed_login()?;
    let account = match (&packed, request.account) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(request.account, false)?.0,
    };
    let (found, branch) = checked(request.script, false)?;
    let needing = needs_redistributable(&found)?;
    let description = upload::commit(request.script)
        .and_then(|commit| upload::description(request.version, &commit))
        .map_err(|error| fail(&error, REFUSED))?;
    show::field("build", &description);
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    if needing {
        ticked_on_steam(&found, (&held, &home, &manifest), &account, packed.as_ref())?;
    }
    let mut prepared = upload::prepare(&home, request.script, &description, request.preview)
        .map_err(|error| fail(&error, FAILED))?;
    // steamcmd uploads everything before Steam refuses to commit a build to a branch the app
    // cannot have yet, so the build goes up set live nowhere by steamcmd.
    if branch == Branch::Unbranched && !request.preview {
        prepared
            .hold_back_set_live()
            .map_err(|error| fail(&error, FAILED))?;
    }
    // Asked for on every upload by a game's tools, the default branch is left once the game is
    // released, and the upload goes on to its own branch as ever.
    let default = if request.default {
        let asked = default_unreleased(&home, &account, prepared.app_id, packed.as_ref());
        if asked.is_err() {
            install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
        }
        match asked? {
            Ok(()) => Some(web_api(&home)?),
            Err(why) => {
                show::note(&format!("{why}; the build goes up without it"));
                None
            }
        }
    } else {
        None
    };
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let saved = prepared.output.join("steamcmd.log");
    let steam = Steam {
        program: &program,
        account: &account,
        prepared: &prepared,
        home: &home,
        root: &root,
        saved: &saved,
        preview: request.preview,
    };
    let (doing, again) = if request.preview {
        ("computing the build", "computing the build again")
    } else {
        ("uploading", "uploading again")
    };
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let Ran {
        outcome,
        spinner,
        took,
        redactor,
        ..
    } = build_retrying(&steam, doing, again)?;
    let outcome = found_on_steam(outcome, &prepared, &description);
    let reporting = Reporting {
        spinner,
        took: &took,
        prepared: &prepared,
        saved: &saved,
        packed: packed.as_ref(),
        default: default.as_ref().map(|(api, _)| api),
    };
    let (code, live) = report(&outcome, reporting);
    let setting_default = default.is_some();
    if let Some((api, from)) = default {
        offer_to_keep(&home, &api, from);
    }
    if request.json {
        show::plain(&format!(
            "{}\n",
            summary(&outcome, live, setting_default, &prepared, &description)
        ));
    }
    if matches!(
        outcome,
        upload::Outcome::Failed(_) | upload::Outcome::BuiltThenFailed { .. }
    ) {
        dump_failed(saved, &prepared, &marks.added(&steamcmd_logs), &redactor);
    }
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok((code, prepared.held_back))
}

/// The logs of a failed upload, steamcmd's console saved at `saved` among them, as
/// [`dump_in_actions`] shows them with what steamcmd `added` to its own.
fn dump_failed(
    saved: PathBuf,
    prepared: &upload::Prepared,
    added: &[(String, Vec<u8>)],
    redactor: &Redactor,
) {
    let mut files = vec![
        ("steamcmd's console".to_owned(), saved),
        (format!("app_build_{}.log", prepared.app_id), prepared.log()),
    ];
    files.extend(depot_logs(&prepared.output));
    dump_in_actions(&files, &[], added, redactor);
}

/// What every run of steamcmd for one upload is given.
struct Steam<'upload> {
    program: &'upload Path,
    account: &'upload Account,
    prepared: &'upload upload::Prepared,
    home: &'upload Path,
    root: &'upload Path,
    /// Where steamcmd's console is written, redacted.
    saved: &'upload Path,
    preview: bool,
}

/// What one run of steamcmd for an upload came to, its spinner still turning.
struct Ran {
    outcome: upload::Outcome,
    /// Why the build failed, if running steamcmd again may pass.
    retry: Option<upload::Retry>,
    spinner: Spinner,
    took: String,
    redactor: Redactor,
}

/// Runs steamcmd's build, saying it is `doing` so, and runs it `again` while it fails in a way
/// that may pass: once Steam's CDN serves a manifest it did not, and once more for no connection
/// to Steam's build server.
fn build_retrying(steam: &Steam<'_>, doing: &str, again: &str) -> Result<Ran, ExitCode> {
    let mut ran = build(steam, doing)?;
    let mut runs = 1_u32;
    let mut patience = upload::CDN_PATIENCE;
    let mut reconnected = false;
    while let Some(retry) = ran.retry.take() {
        match retry {
            upload::Retry::Unserved(unserved) => {
                let depot = unserved.depot;
                if runs == upload::RUNS {
                    upload::still_unserved(&mut ran.outcome, depot);
                    break;
                }
                ran.spinner.failed(&format!(
                    "Steam's CDN does not serve depot {depot}'s last manifest yet"
                ));
                let waiting = Spinner::start(
                    "cdn",
                    &format!("waiting for depot {depot}'s last manifest"),
                    true,
                );
                let served = download::served(&unserved.url, patience);
                patience = patience.saturating_sub(waiting.elapsed());
                let waited = show::took(waiting.elapsed());
                match served {
                    download::Served::Yes => waiting.done(&format!("served after {waited}")),
                    // The CDN answers 401 for a request code it does not take, as one that has
                    // expired, and steamcmd's next run is given a new one.
                    download::Served::Otherwise(status) => {
                        waiting.done(&format!("answered {status} after {waited}; asking again"));
                    }
                    download::Served::No => {
                        ran.spinner = waiting;
                        ran.took = waited;
                        upload::still_unserved(&mut ran.outcome, depot);
                        break;
                    }
                }
            }
            lost @ (upload::Retry::NoConnection
            | upload::Retry::Disconnected
            | upload::Retry::NoClientConfig) => {
                if reconnected || runs == upload::RUNS {
                    break;
                }
                reconnected = true;
                let what = match lost {
                    upload::Retry::Disconnected => "Steam disconnected steamcmd",
                    upload::Retry::NoClientConfig => "Steam sent steamcmd no client config",
                    upload::Retry::NoConnection | upload::Retry::Unserved(_) => {
                        "no connection to Steam's build server"
                    }
                };
                ran.spinner.failed(&format!("{what} after {}", ran.took));
            }
        }
        runs = runs.saturating_add(1);
        ran = build(steam, again)?;
    }
    Ok(ran)
}

/// Runs steamcmd's build once, saying it is `doing` so, and reads what it came to.
fn build(steam: &Steam<'_>, doing: &str) -> Result<Ran, ExitCode> {
    upload::clear_log(steam.prepared).map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(steam.home).map_err(|error| fail(&error, FAILED))?;
    let spinner = Spinner::start("steam", doing, true);
    let finished = run::run(
        steam.program,
        &steamcmd::upload(steam.account, &steam.prepared.script),
        &steamcmd::environment(steam.home, Platform::THIS),
        steam.root,
        steamcmd::UPLOAD_LIMIT,
        steamcmd::HOPELESS,
    );
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(
                &format!("{}: {error}", steam.program.display()),
                FAILED,
            ));
        }
    };
    let redactor =
        before.and(Redactor::for_home(steam.home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    fs::write(steam.saved, &console)
        .map_err(|error| fail(&format!("{}: {error}", steam.saved.display()), FAILED))?;
    let log = fs::read(steam.prepared.log())
        .ok()
        .map(|log| redactor.redact(&log));
    let console = String::from_utf8_lossy(&console);
    let log = log.as_deref().map(String::from_utf8_lossy);
    let outcome = upload::judge(
        steam.prepared.app_id,
        finished.code,
        &console,
        log.as_deref(),
        steam.preview,
    );
    let retry = if matches!(outcome, upload::Outcome::Failed(_)) && run::told().is_none() {
        upload::retry(&console, log.as_deref())
    } else {
        None
    };
    Ok(Ran {
        outcome,
        retry,
        spinner,
        took,
        redactor,
    })
}

/// Asks Steam, when a key is at hand, whether `build_id` is what is live on `branch` now; false
/// only when Steam shows another build there, or no such branch. Asked a few times, since a
/// build just set live may not show at once; a Steam that cannot be asked holds nothing up.
fn confirmed_live(app: u32, branch: &str, build_id: u64) -> bool {
    const ASKS: u32 = 4;
    const APART: Duration = Duration::from_secs(2);
    let Ok(Some(api)) = key_at_hand() else {
        return true;
    };
    let spinner = Spinner::start("branch", "asking Steam what is live", false);
    let ask = || {
        api.branches(app)
            .map(|branches| upload::live_now(&branches, branch, build_id))
    };
    let mut seen = ask();
    for _ in 1..ASKS {
        if !matches!(seen, Ok(upload::Live::Other(_) | upload::Live::Missing)) {
            break;
        }
        thread::sleep(APART);
        seen = ask();
    }
    match seen {
        Ok(upload::Live::Confirmed) => {
            spinner.done(&format!("{branch}: BuildID {build_id}, as Steam shows it"));
            true
        }
        Ok(upload::Live::Other(other)) => {
            spinner.failed(&format!("{branch}: BuildID {other}, as Steam shows it"));
            false
        }
        Ok(upload::Live::Missing) => {
            spinner.failed(&format!("{branch}: no such branch on Steam"));
            false
        }
        Err(error) => {
            spinner.failed("not confirmed");
            show::aside(&error.to_string());
            true
        }
    }
}

/// A build Steam kept but set live nowhere, found by its description when a key is at hand, as
/// the build it is: steamcmd reports no build ID when Steam refuses to set one live.
fn found_on_steam(
    outcome: upload::Outcome,
    prepared: &upload::Prepared,
    description: &str,
) -> upload::Outcome {
    let upload::Outcome::Failed(reasons) = &outcome else {
        return outcome;
    };
    if upload::commit_refused(reasons, prepared.set_live.as_deref()).is_none() {
        return outcome;
    }
    let Ok(Some(api)) = key_at_hand() else {
        return outcome;
    };
    // Enough to reach past uploads made while this one ran.
    let found = api
        .builds(prepared.app_id, 10)
        .ok()
        .and_then(|builds| upload::uploaded_as(&builds, description));
    let Some(build_id) = found else {
        return outcome;
    };
    upload::Outcome::BuiltThenFailed {
        build_id,
        reasons: reasons.clone(),
    }
}

/// What an upload's ending is said with: the spinner still turning, how long steamcmd took,
/// where its console was saved, the login handed over if any, the Web API to set the build live
/// on the default branch with, when asked to.
struct Reporting<'upload> {
    spinner: Spinner,
    took: &'upload str,
    prepared: &'upload upload::Prepared,
    saved: &'upload Path,
    packed: Option<&'upload ci::Login>,
    default: Option<&'upload webapi::Api>,
}

/// Where an upload's build ended up live.
#[derive(Clone, Copy, Default)]
struct Live {
    /// On the branch the script sets live.
    branch: bool,
    /// On the default branch.
    default: bool,
}

/// Says what `outcome` came to, and answers its exit code and where the build is live.
fn report(outcome: &upload::Outcome, reporting: Reporting<'_>) -> (ExitCode, Live) {
    let Reporting {
        spinner,
        took,
        prepared,
        saved,
        packed,
        default,
    } = reporting;
    let app = prepared.app_id;
    let logs = format!(
        "log  {}, and steamcmd's own in {}",
        saved.display(),
        prepared.log().display()
    );
    let code = match outcome {
        upload::Outcome::Built { build_id } => {
            spinner.done(&format!("uploaded in {took}"));
            hand_on(*build_id);
            if let Some(branch) = &prepared.held_back {
                return first_on_default(app, *build_id, branch, default, &logs);
            }
            return set_live(app, *build_id, prepared.set_live.as_deref(), default, &logs);
        }
        upload::Outcome::Previewed => {
            spinner.done(&format!("preview finished in {took}"));
            show::success("nothing was uploaded, as asked", &logs);
            ExitCode::SUCCESS
        }
        upload::Outcome::NotLoggedIn(line) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(line, packed)
        }
        upload::Outcome::BuiltThenFailed { build_id, reasons } => {
            spinner.failed(&format!("failed after {took}"));
            show::failure(
                &format!("app {app}: built as BuildID {build_id}, then steamcmd failed"),
                "",
                None,
            );
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(&why);
            }
            let branches = prepared
                .set_live
                .iter()
                .map(String::as_str)
                .chain(default.map(|_| "default"));
            for branch in branches {
                show::hint(Hint {
                    before: "set it live with ",
                    command: &format!(
                        "steamship promote {app} --build {build_id} --branch {branch}"
                    ),
                    after: "",
                });
            }
            show::note(&logs);
            hand_on(*build_id);
            ExitCode::from(FAILED)
        }
        upload::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(upload::NOT_LIVE);
                show::note(&why);
                show::hint(Hint {
                    before: "see the app's branches with ",
                    command: &format!("steamship builds {app}"),
                    after: "",
                });
            }
            show::note(&logs);
            ExitCode::from(FAILED)
        }
    };
    (code, Live::default())
}

/// Says where `build_id`, just uploaded for `app`, is live: on `branch`, which the script set it
/// live on, once Steam shows it there, and then on default, set through `default` when given.
/// Answers the upload's exit code and where the build is live; `logs` says where to look.
fn set_live(
    app: u32,
    build_id: u64,
    branch: Option<&str>,
    default: Option<&webapi::Api>,
    logs: &str,
) -> (ExitCode, Live) {
    if let Some(branch) = branch
        && !confirmed_live(app, branch, build_id)
    {
        show::failure(
            &format!(
                "app {app}: BuildID {build_id} uploaded, but Steam does not show it live on \
                 {branch}"
            ),
            "",
            Some(Hint {
                before: "set it live with ",
                command: &format!("steamship promote {app} --build {build_id} --branch {branch}"),
                after: "",
            }),
        );
        show::note(logs);
        return (ExitCode::from(FAILED), Live::default());
    }
    let mut live = Live {
        branch: branch.is_some(),
        default: false,
    };
    if let Some(api) = default {
        if !live_on_default(api, app, build_id, branch) {
            show::note(logs);
            return (ExitCode::from(FAILED), live);
        }
        live.default = true;
    }
    let set = match (branch, live.default) {
        (Some(branch), true) => format!(", set live on {branch} and default"),
        (Some(branch), false) => format!(", set live on {branch}"),
        (None, true) => ", set live on default".to_owned(),
        (None, false) => String::new(),
    };
    show::success(&format!("app {app}: BuildID {build_id}{set}"), logs);
    (ExitCode::SUCCESS, live)
}

/// Says where `build_id`, just uploaded for `app`, is live while the app has no branch but
/// default, so that `branch`, which the script sets live, cannot be made yet: on default, set
/// through `default` when given. Answers as [`set_live`] does.
fn first_on_default(
    app: u32,
    build_id: u64,
    branch: &str,
    default: Option<&webapi::Api>,
    logs: &str,
) -> (ExitCode, Live) {
    let make = format!(
        "make {branch} in Steamworks under SteamPipe, Builds, and set the build live there with "
    );
    let promote = format!("steamship promote {app} --build {build_id} --branch {branch}");
    let Some(api) = default else {
        show::failure(
            &format!("app {app}: BuildID {build_id} uploaded, but set live nowhere"),
            &format!(
                "app {app} has no branch but default until a build is live there, so steamcmd \
                 was not asked to set it live on {branch}"
            ),
            Some(Hint {
                before: "set it live on default with ",
                command: &format!("steamship promote {app} --build {build_id} --branch default"),
                after: "",
            }),
        );
        show::hint(Hint {
            before: &make,
            command: &promote,
            after: "",
        });
        show::note(logs);
        return (ExitCode::from(FAILED), Live::default());
    };
    let (code, live) = set_live(app, build_id, None, Some(api), logs);
    if live.default {
        show::note(&format!(
            "not set live on {branch}: Steam gives app {app} no branch but default until a build \
             is live there"
        ));
        show::hint(Hint {
            before: &make,
            command: &promote,
            after: "",
        });
    }
    (code, live)
}

/// Sets `build_id` of `app` live on the default branch through `api`, and says whether Steam
/// did; the build is already live on `branch`, if the script sets one live.
fn live_on_default(api: &webapi::Api, app: u32, build_id: u64, branch: Option<&str>) -> bool {
    let spinner = Spinner::start("default", "setting it live", false);
    let Err(error) = api.set_live(app, build_id, webapi::DEFAULT_KEY) else {
        spinner.done("set live");
        return true;
    };
    spinner.failed("not set live");
    let also = branch.map_or_else(String::new, |branch| format!(" and live on {branch}"));
    show::failure(
        &format!(
            "app {app}: BuildID {build_id} uploaded{also}, but Steam did not set it live on \
             default"
        ),
        &error.to_string(),
        Some(Hint {
            before: "set it live with ",
            command: &format!("steamship promote {app} --build {build_id} --branch default"),
            after: "",
        }),
    );
    false
}

/// What an upload came to, for the program that ran steamship with `--json`: the outcome, the
/// app, the build Steam made of it if any, the branch the script sets live and whether the build
/// is live there and on default, and the build's description. `default` is whether it was to go
/// live on default too: asked to, while the app is unreleased.
fn summary(
    outcome: &upload::Outcome,
    live: Live,
    default: bool,
    prepared: &upload::Prepared,
    description: &str,
) -> serde_json::Value {
    let branch = prepared.set_live.as_ref().or(prepared.held_back.as_ref());
    let (said, build_id) = match outcome {
        upload::Outcome::Built { build_id } => {
            let said = if (branch.is_some() && !live.branch) || (default && !live.default) {
                "not-live"
            } else {
                "uploaded"
            };
            (said, Some(*build_id))
        }
        upload::Outcome::Previewed => ("previewed", None),
        upload::Outcome::NotLoggedIn(_) => ("not-logged-in", None),
        upload::Outcome::BuiltThenFailed { build_id, .. } => ("built-then-failed", Some(*build_id)),
        upload::Outcome::Failed(_) => ("failed", None),
    };
    serde_json::json!({
        "outcome": said,
        "app": prepared.app_id,
        "build_id": build_id,
        "branch": branch,
        "live": live.branch,
        "default": live.default,
        "description": description,
    })
}

/// Steam's logs of a failed run, as collapsed groups in a GitHub Actions job's log: `files` as
/// they are, what was `captured` without being saved, then what steamcmd `added` to its own logs
/// this run, all of it redacted. The action deletes steamship's home when the job ends, so this
/// is the only look at them there is.
fn dump_in_actions(
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
    show::plain(&shown);
}

/// Each depot's build log in `output`, by name, in name order.
fn depot_logs(output: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(output)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let log = Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
            (name.starts_with("depot_build_") && log).then(|| (name, entry.path()))
        })
        .collect();
    found.sort();
    found
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
                    "not checked; upload checks it once the game is built",
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
/// full game; and its rich presence files are read as `rich-presence` sends them.
fn try_check_folder(folder: &Path, scripts_only: bool, demo: bool) -> Result<ExitCode, ExitCode> {
    if scripts_only {
        return Err(fail(
            &format!(
                "{} is a build's folder, which holds no scripts; give --scripts-only the app \
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
                    "{} is in no game's folder, so it has no demo; run the check in the game's \
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
                .any(|file| app.depots.iter().any(|depot| check::ships(depot, file)))
        })
    };
    ships(&demo.script) && !ships(&game.script)
}

/// The key set in `STEAMSHIP_WEB_API_KEY`, where an empty one is none, as for the account.
fn key_variable() -> Option<Zeroizing<String>> {
    env::var(webapi::KEY)
        .ok()
        .filter(|text| !text.is_empty())
        .map(Zeroizing::new)
}

/// Where Steamworks shows the key, said before it is asked for.
const WHERE_KEY: &str = "the publisher Web API key is in Steamworks under Users & Permissions, \
                         Manage Groups: the group that holds the app";

/// How the Web API key for a command was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyFrom {
    Variable,
    Kept,
    Typed,
}

/// The partner Web API, with the key from `STEAMSHIP_WEB_API_KEY`, or else the one kept in the
/// credential store, or else, at a terminal, one typed in.
fn web_api(home: &Path) -> Result<(webapi::Api, KeyFrom), ExitCode> {
    if let Some(text) = key_variable() {
        let key = webapi::Key::parse(&text).map_err(|_not_a_key| {
            fail(
                &format!(
                    "{} does not hold a publisher Web API key, which is 32 hexadecimal digits",
                    webapi::KEY
                ),
                REFUSED,
            )
        })?;
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
            command: "steamship login --web-api-key",
            after: " at a terminal to keep one, or set STEAMSHIP_WEB_API_KEY",
        };
        show::failure("no Web API key", "", Some(keep));
        return Err(ExitCode::from(REFUSED));
    }
    show::aside(WHERE_KEY);
    let key = typed_key()?.ok_or_else(|| fail(&"no Web API key typed", REFUSED))?;
    Ok((webapi::Api::new(key), KeyFrom::Typed))
}

/// The key typed at the prompt, shown as dots; none when only Enter was pressed.
fn typed_key() -> Result<Option<webapi::Key>, ExitCode> {
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
fn check_key(api: &webapi::Api) -> Result<(), ExitCode> {
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

/// Keeps `key` for `home` in the credential store, and says whether it did; why not is shown.
fn keep_key(home: &Path, key: &webapi::Key) -> bool {
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

/// `login --web-api-key`: asks for the key, has Steam check it, and keeps it.
fn try_key_login() -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("login");
    let home = home()?;
    show::aside(WHERE_KEY);
    let key = typed_key()?.ok_or_else(|| fail(&"no Web API key typed", REFUSED))?;
    let api = webapi::Api::new(key);
    check_key(&api)?;
    if !keep_key(&home, api.key()) {
        return Err(ExitCode::from(FAILED));
    }
    show::success(
        "Web API key kept",
        "`steamship builds` and the other Web API commands use it; `steamship logout` forgets it",
    );
    Ok(ExitCode::SUCCESS)
}

/// At the end of `login`, at a terminal: offers to keep the Web API key too, when none is set or
/// kept and there is a store to keep it in. Only Enter skips it; nothing here fails the login.
fn offer_key(home: &Path) {
    if !io::stdin().is_terminal() || key_variable().is_some() {
        return;
    }
    match keychain::has(home) {
        Ok(false) => {}
        Ok(true) => {
            show::field("api key", &format!("kept in {}", keychain::STORE));
            return;
        }
        Err(_) => return,
    }
    show::aside("`steamship builds` and the other Web API commands need the publisher key;");
    show::aside(&format!("{WHERE_KEY}. Enter skips it"));
    let key = match typed_key() {
        Ok(Some(key)) => key,
        Ok(None) => {
            show::field(
                "api key",
                "skipped; `steamship login --web-api-key` keeps one",
            );
            return;
        }
        Err(_) => return,
    };
    let api = webapi::Api::new(key);
    if check_key(&api).is_ok() {
        let _kept = keep_key(home, api.key());
    }
}

/// After a command that worked with a key typed at its prompt, offers to keep that key.
fn offer_to_keep(home: &Path, api: &webapi::Api, from: KeyFrom) {
    if from != KeyFrom::Typed {
        return;
    }
    show::aside(&format!(
        "Enter keeps the key in {} for next time; n does not",
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

/// The app named by its ID, or by the app build script that names it.
fn app_named(app: &str) -> Result<u32, ExitCode> {
    let app_id = match app.parse::<u32>() {
        Ok(app_id) if app_id > 0 => app_id,
        Ok(_) => return Err(fail(&"0 is not an app's ID", REFUSED)),
        Err(_) => scripts::load(Path::new(app))
            .map(|script| script.app_id)
            .map_err(|problems| {
                for problem in &problems {
                    show::failure("refused", &problem.to_string(), None);
                }
                ExitCode::from(REFUSED)
            })?,
    };
    show::field("app", &app_id.to_string());
    Ok(app_id)
}

/// Why Steam gave `error` about `app`, when the apps the key holds can say: asked only for the
/// one error that needs them.
fn why_unanswered(api: &webapi::Api, app: u32, error: &webapi::Error) -> Option<String> {
    if !matches!(error, webapi::Error::Status(500, _)) {
        return None;
    }
    webapi::no_build_yet(error, app, &api.apps().ok()?)
}

/// Says why the Web API did not answer, and the exit code for it.
fn web_api_failed(spinner: Spinner, error: &webapi::Error) -> ExitCode {
    spinner.failed(&match error {
        webapi::Error::NotAKey => "not a key".to_owned(),
        webapi::Error::Refused => "key refused".to_owned(),
        webapi::Error::Status(status, _) => format!("HTTP {status}"),
        webapi::Error::Unreachable(_) => "no answer".to_owned(),
        webapi::Error::Unreadable(_) => "unreadable".to_owned(),
        webapi::Error::Failed(_) => "failed".to_owned(),
    });
    fail(error, FAILED)
}

fn try_builds(app: &str, count: u32, json: bool) -> Result<ExitCode, ExitCode> {
    if json {
        show::divert();
    }
    show::title("builds");
    let app_id = app_named(app)?;
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "asking for the branches and builds", false);
    let overview = match api.overview(app_id, count) {
        Ok(overview) => overview,
        Err(error) => {
            let why = why_unanswered(&api, app_id, &error);
            let code = web_api_failed(spinner, &error);
            if let Some(why) = &why {
                show::note(why);
            }
            if json {
                show::plain(&format!(
                    "{}\n",
                    webapi::error_json(app_id, &error, why.as_deref())
                ));
            }
            return Err(code);
        }
    };
    spinner.done(&overview.summary());
    show::fields(&overview.lines());
    offer_to_keep(&home, &api, from);
    if json {
        show::plain(&format!("{}\n", overview.json(app_id)));
    }
    Ok(ExitCode::SUCCESS)
}

fn try_fetch(request: &cli::Fetch) -> Result<ExitCode, ExitCode> {
    if request.json {
        show::divert();
    }
    show::title("fetch");
    let app = app_or_game(request.app.clone(), request.demo)?;
    let app_id = app_named(&app)?;
    let home = home()?;
    let branch = branch_to_fetch(&home, app_id, request)?;
    let shown = webapi::shown_name(&branch.name);
    let system = request.system.unwrap_or(Platform::THIS);
    let given = request
        .into
        .clone()
        .unwrap_or_else(|| fetch::folder(&home, app_id, &branch.name, system));
    show::field("branch", shown);
    show::field("system", fetch::system_name(system));
    show::field("folder", &given.display().to_string());
    let folder = fetch_folder(&given, app_id, shown)?;
    let password = if branch.locked {
        Some(branch_password(
            &home,
            app_id,
            &branch.name,
            request.password.on_command_line,
        )?)
    } else {
        None
    };
    let held = fetched(&Fetching {
        app: app_id,
        branch: &branch,
        system,
        folder: &folder,
        named: named(request.account.as_deref()),
        password: password.as_ref(),
        on_command_line: request.password.on_command_line,
    })?;
    if let Some(password) = &password {
        keep_branch_password(&home, app_id, &branch.name, password, request.password.keep);
    }
    if held.build_id != branch.build_id || held.branch != shown {
        return Err(fail(
            &format!(
                "steamcmd put build {} of the {} branch in {}, where Steam's Web API showed build \
                 {} live on {shown}; if a build was set live meanwhile, fetch again",
                held.build_id,
                held.branch,
                given.display(),
                branch.build_id
            ),
            FAILED,
        ));
    }
    show::success(
        &format!(
            "app {app_id}: build {} of the {shown} branch is in {}",
            held.build_id,
            given.display()
        ),
        "",
    );
    if request.json {
        show::plain(&format!(
            "{}\n",
            fetch::json(app_id, &held, system, &folder)
        ));
    }
    Ok(ExitCode::SUCCESS)
}

/// The branch `request` asks to fetch of `app`, among the branches Steam's Web API shows.
fn branch_to_fetch(
    home: &Path,
    app: u32,
    request: &cli::Fetch,
) -> Result<webapi::Branch, ExitCode> {
    let (api, from) = web_api(home)?;
    let spinner = Spinner::start("steam", "asking for the branches", false);
    let branches = match api.branches(app) {
        Ok(branches) => branches,
        Err(error) => {
            let why = why_unanswered(&api, app, &error);
            let code = web_api_failed(spinner, &error);
            if let Some(why) = &why {
                show::note(why);
            }
            return Err(code);
        }
    };
    spinner.done(&match branches.len() {
        1 => "1 branch".to_owned(),
        count => format!("{count} branches"),
    });
    offer_to_keep(home, &api, from);
    fetch::chosen(&branches, app, request.branch.as_deref(), request.build)
        .cloned()
        .map_err(|why| fail(&why, REFUSED))
}

/// `folder` as a path from the root, made if it is not there. Refused when it holds `app`'s
/// build of another branch than `branch`, or files and no build of `app`, which steamcmd would
/// mix the build into.
fn fetch_folder(folder: &Path, app: u32, branch: &str) -> Result<PathBuf, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", folder.display()), REFUSED);
    let record = fetch::manifest(folder, app);
    match fs::read_to_string(&record) {
        Ok(text) => {
            if let Ok(held) = fetch::installed(&text)
                && held.branch != branch
            {
                return Err(refused(&format!(
                    "holds app {app}'s {} branch; fetch {branch} into a folder of its own",
                    held.branch
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some()) {
                return Err(refused(&format!(
                    "holds files and no build of app {app}; fetch into an empty folder, or one a \
                     fetch of {branch} made"
                )));
            }
        }
        Err(error) => return Err(fail(&format!("{}: {error}", record.display()), FAILED)),
    }
    fs::create_dir_all(folder).map_err(|error| refused(&error))?;
    path::absolute(folder).map_err(|error| refused(&error))
}

/// Where the password of a branch being fetched came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordFrom {
    Variable,
    Kept,
    Typed,
}

/// The password of a branch being fetched.
struct GivenPassword {
    text: Zeroizing<String>,
    from: PasswordFrom,
}

/// The password of `app`'s `branch`: the one in the variable, or else the one kept for it, or
/// else one typed at the prompt. Refused when none can be had, or when one bound for steamcmd's
/// console, rather than its command line, is one the console would split.
fn branch_password(
    home: &Path,
    app: u32,
    branch: &str,
    on_command_line: bool,
) -> Result<GivenPassword, ExitCode> {
    let shown = webapi::shown_name(branch);
    let given = if let Some(text) = env::var_os(fetch::PASSWORD).filter(|text| !text.is_empty()) {
        let text = text
            .into_string()
            .map_err(|_not_text| fail(&format!("{} is not text", fetch::PASSWORD), REFUSED))?;
        show::field("password", &format!("from {}", fetch::PASSWORD));
        GivenPassword {
            text: Zeroizing::new(text),
            from: PasswordFrom::Variable,
        }
    } else if let Some(text) = kept_branch_password(home, app, branch) {
        show::field("password", &format!("kept in {}", keychain::STORE));
        GivenPassword {
            text,
            from: PasswordFrom::Kept,
        }
    } else {
        let needed = || {
            fail(
                &format!(
                    "app {app}'s {shown} branch takes a password: set {} to it, or type it when \
                     fetch asks, which it does without --json",
                    fetch::PASSWORD
                ),
                REFUSED,
            )
        };
        // With --json, standard output holds the JSON alone, and the prompt would go there.
        if show::diverted() {
            return Err(needed());
        }
        show::aside(&format!(
            "app {app}'s {shown} branch takes a password, the one its Steamworks betas page shows"
        ));
        let text = typing::ask("password", Echo::Dots)
            .ok()
            .filter(|text| !text.is_empty())
            .ok_or_else(needed)?;
        GivenPassword {
            text,
            from: PasswordFrom::Typed,
        }
    };
    match (on_command_line, fetch::typeable(&given.text)) {
        (true, _) => show::aside(
            "the password goes on steamcmd's command line, where any program on this machine can \
             read it while the fetch runs",
        ),
        (false, true) => {}
        (false, false) => {
            return Err(fail(
                &"steamcmd's console splits a password at a space or a quote, so this one can \
                  only be given with --password-on-command-line",
                REFUSED,
            ));
        }
    }
    Ok(given)
}

/// The password kept for `app`'s `branch`, if one is and the store can be read; a store that
/// fails is named, and the password asked for instead.
fn kept_branch_password(home: &Path, app: u32, branch: &str) -> Option<Zeroizing<String>> {
    match keychain::branch_password(home, app, branch) {
        Ok(kept) => kept,
        Err(keychain::Error::Unavailable(_)) => None,
        Err(error) => {
            show::note(&error.to_string());
            None
        }
    }
}

/// Once Steam has taken `password` for `app`'s `branch`, keeps it: one given by the variable or
/// typed when `asked`, and a typed one too when Enter answers the offer.
fn keep_branch_password(
    home: &Path,
    app: u32,
    branch: &str,
    password: &GivenPassword,
    asked: bool,
) {
    let keep = match password.from {
        PasswordFrom::Kept => false,
        PasswordFrom::Variable => asked,
        PasswordFrom::Typed => asked || keeping_offered(),
    };
    if !keep {
        return;
    }
    match keychain::keep_branch_password(home, app, branch, &password.text) {
        Ok(()) => show::done("password", &format!("kept in {}", keychain::STORE)),
        Err(error) => {
            show::failed("password", "not kept");
            show::failure(&error.to_string(), "", None);
        }
    }
}

/// Offers to keep a typed password, and answers whether Enter, or a yes, took the offer.
fn keeping_offered() -> bool {
    show::aside(&format!(
        "Enter keeps the password in {} for next time; n does not",
        keychain::STORE
    ));
    let keep = typing::ask("keep it", Echo::Typed).is_ok_and(|answer| {
        matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "" | "y" | "yes"
        )
    });
    if !keep {
        show::field("password", "not kept");
    }
    keep
}

/// What `fetched` downloads, and as whom.
struct Fetching<'command> {
    app: u32,
    branch: &'command webapi::Branch,
    system: Platform,
    folder: &'command Path,
    named: Option<&'command str>,
    password: Option<&'command GivenPassword>,
    on_command_line: bool,
}

/// Has steamcmd download the build, and answers which one the folder then holds.
fn fetched(fetching: &Fetching<'_>) -> Result<fetch::Installed, ExitCode> {
    let packed = packed_login()?;
    let account = match (&packed, fetching.named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(fetching.named, false)?.0,
    };
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let secret = fetching
        .password
        .map_or_else(Zeroizing::default, |password| password.text.clone());
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "downloading", true);
    let finished = run_fetch(fetching, &account, &home, &secret);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = fetch::hidden(
        &String::from_utf8_lossy(&redactor.redact(&finished.output)),
        &secret,
    );
    let log = marks
        .added(&steamcmd_logs)
        .into_iter()
        .find(|(name, _)| name == "content_log.txt")
        .map(|(_, added)| {
            fetch::hidden(&String::from_utf8_lossy(&redactor.redact(&added)), &secret)
        })
        .unwrap_or_default();
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    match fetch::judge(finished.code, &console, &log, fetching.app) {
        fetch::Outcome::Fetched => spinner.done(&format!("downloaded in {took}")),
        fetch::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            return Err(login_refused(&reason, packed.as_ref()));
        }
        fetch::Outcome::PasswordRefused(reasons) => {
            spinner.failed(&format!("refused after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            forget_refused_password(&home, fetching);
            return Err(ExitCode::from(FAILED));
        }
        fetch::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            return Err(ExitCode::from(FAILED));
        }
    }
    let record = fetch::manifest(fetching.folder, fetching.app);
    fs::read_to_string(&record)
        .map_err(|error| error.to_string())
        .and_then(|text| fetch::installed(&text))
        .map_err(|why| fail(&format!("{}: {why}", record.display()), FAILED))
}

/// Has steamcmd in `home` fetch as `account`, typing `secret`, the branch's password, into its
/// console, or giving it on its command line when asked to.
fn run_fetch(
    fetching: &Fetching<'_>,
    account: &Account,
    home: &Path,
    secret: &Zeroizing<String>,
) -> io::Result<run::Finished> {
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let branch = if webapi::is_default(&fetching.branch.name) {
        webapi::DEFAULT_KEY
    } else {
        &fetching.branch.name
    };
    let system = fetch::system_name(fetching.system);
    let environment = steamcmd::environment(home, Platform::THIS);
    if fetching.password.is_some() && !fetching.on_command_line {
        return typed_fetch(
            &program,
            &steamcmd::fetch_console(account, system, fetching.folder),
            &environment,
            &root,
            fetch::typed_command(fetching.app, branch, secret),
            secret.clone(),
        );
    }
    run::run(
        &program,
        &steamcmd::fetch(
            account,
            fetching.app,
            branch,
            fetching.password.map(|_| secret.as_str()),
            system,
            fetching.folder,
        ),
        &environment,
        &root,
        steamcmd::FETCH_LIMIT,
        steamcmd::HOPELESS,
    )
}

/// Forgets the branch's password Steam refused, when it was the one kept, so that the next fetch
/// asks for the one the branch takes now.
fn forget_refused_password(home: &Path, fetching: &Fetching<'_>) {
    if !fetching
        .password
        .is_some_and(|password| password.from == PasswordFrom::Kept)
    {
        return;
    }
    match keychain::forget_branch_password(home, fetching.app, &fetching.branch.name) {
        Ok(_) => show::note(&format!(
            "the password kept in {} is forgotten; fetch again to give the one the branch takes \
             now",
            keychain::STORE
        )),
        Err(error) => show::failure(&error.to_string(), "", None),
    }
}

/// Runs `program` with `args` at a console of its own and types `command` at its prompt, as
/// [`fetch::typed`] does with `secret` hidden, within a fetch's time or it is stopped. A
/// password typed there is seen by no other program, as one on the command line is.
fn typed_fetch(
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    directory: &Path,
    command: Zeroizing<String>,
    secret: Zeroizing<String>,
) -> io::Result<run::Finished> {
    let (terminal, mut output, mut input) = Terminal::start(program, args, environment, directory)?;
    let (ended, ending) = mpsc::channel();
    let reading = thread::spawn(move || {
        let read = fetch::typed(
            &mut output,
            &mut input,
            &command,
            &secret,
            steamcmd::HOPELESS,
        );
        // The receiver has gone only once the limit passed, and then nothing waits to hear.
        let _told = ended.send(());
        read
    });
    let in_time = ending.recv_timeout(steamcmd::FETCH_LIMIT).is_ok();
    // Dropping the terminal unwaited ends steamcmd, and with it the output being read.
    let terminal = in_time.then_some(terminal);
    let lines = reading
        .join()
        .unwrap_or_else(|panic| panic::resume_unwind(panic))?;
    let given_up = lines
        .last()
        .is_some_and(|line| steamcmd::HOPELESS.iter().any(|words| line.contains(words)));
    let code = match terminal {
        Some(terminal) if !given_up => terminal.wait()?,
        _ => None,
    };
    Ok(run::Finished {
        code,
        output: lines.join("\n").into_bytes(),
    })
}

/// The achievements file at `path`, read, and refused when it is not one or names another app
/// than `app`: before Steam is asked anything.
fn achievements_file(path: &Path, app: u32) -> Result<achievements::Listed, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let localisation = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"));
    if localisation {
        let listed = achievements::read_localisation(&text).map_err(|why| refused(&why))?;
        show::field(
            "file",
            &format!(
                "{}, {} in {}",
                path.display(),
                show::counted(listed.translations.len(), "achievement"),
                show::counted(listed.languages().len(), "language")
            ),
        );
        return Ok(listed);
    }
    let mut listed = achievements::read(&text).map_err(|why| refused(&why))?;
    if let Some(other) = listed.app.filter(|named| *named != u64::from(app)) {
        return Err(refused(&format!("is for app {other}, not app {app}")));
    }
    let own = path.parent().unwrap_or_else(|| Path::new(""));
    // games-kit gives a record's icons inside the mod that sets it, wherever under the mod its
    // records are kept; a mod's folder is the one holding its manifest.
    let folder = if text.trim_start().starts_with('[') {
        own.ancestors()
            .find(|folder| folder.join("manifest.json").is_file())
            .unwrap_or(own)
    } else {
        own
    };
    for wanted in &mut listed.achievements {
        let defined = &mut wanted.defined;
        for icon in [&mut defined.icon, &mut defined.icon_locked]
            .into_iter()
            .flatten()
        {
            *icon = folder.join(&*icon);
        }
    }
    let mut holds = show::counted(listed.achievements.len(), "achievement");
    if let Some(stats) = &listed.stats {
        holds = format!("{holds} and {}", show::counted(stats.len(), "stat"));
    }
    show::field("file", &format!("{}, {holds}", path.display()));
    Ok(listed)
}

/// The gettext catalogue at `path`, read, and refused when it is not one, is in a language Steam
/// has none of or in English, or is in the same language as one of the `others`.
fn catalogue_file(
    path: &Path,
    others: &[achievements::Catalogued],
) -> Result<achievements::Catalogued, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let catalogue = po::read(&text).map_err(|why| refused(&why))?;
    let locale = catalogue
        .language
        .clone()
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let Some(language) = po::steam_language(&locale) else {
        return Err(refused(&format!(
            "is in the locale {locale}, which Steam has no language for"
        )));
    };
    if language == "english" {
        return Err(refused(
            &"is in English, the language the game's own files are written in",
        ));
    }
    if let Some(other) = others.iter().find(|other| other.language == language) {
        return Err(refused(&format!("is in {language}, as {} is", other.file)));
    }
    show::field("file", &format!("{}, {language}", path.display()));
    Ok(achievements::Catalogued {
        file: path.display().to_string(),
        language,
        catalogue,
    })
}

/// The achievements files at `paths`, each read as `achievements_file` reads one or as a
/// catalogue, as one; or none when there are none to check against. When `checking` them
/// against Steam, it says what in them goes unchecked.
fn achievements_files(
    paths: &[PathBuf],
    app: u32,
    checking: bool,
) -> Result<Option<achievements::Listed>, ExitCode> {
    if paths.is_empty() {
        return Ok(None);
    }
    let mut files = Vec::with_capacity(paths.len());
    let mut catalogues = Vec::new();
    for path in paths {
        let catalogue = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("po"));
        if catalogue {
            let read = catalogue_file(path, &catalogues)?;
            catalogues.push(read);
        } else {
            files.push((path.display().to_string(), achievements_file(path, app)?));
        }
    }
    let mut listed = achievements::together(files).map_err(|why| fail(&why, REFUSED))?;
    achievements::translate(&mut listed, &catalogues).map_err(|why| fail(&why, REFUSED))?;
    if listed.achievements.len() > achievements::MOST_UNVETTED {
        show::note(&format!(
            "Steam takes more than {} achievements only once Valve has vetted the app",
            achievements::MOST_UNVETTED
        ));
    }
    if checking && !listed.unchecked.is_empty() {
        show::field(
            "unchecked",
            &format!(
                "{}, which Steam's Web API does not give",
                listed.unchecked.join(" and ")
            ),
        );
    }
    Ok(Some(listed))
}

/// Every way Steam's achievements differ from the `listed` translations, in each language they
/// are in: English from what Steam `held`, and each other language asked of Steam in turn.
fn translation_drift(
    api: &webapi::Api,
    app: u32,
    listed: &achievements::Listed,
    held: &webapi::Schema,
) -> Result<Vec<achievements::Drift>, ExitCode> {
    let languages = listed.languages();
    let others: Vec<&str> = languages
        .iter()
        .copied()
        .filter(|language| *language != "english")
        .collect();
    let mut drift = achievements::compare_translations(
        &listed.translations,
        "english",
        &held.achievements,
        &held.achievements,
    );
    // Steam has a text in another language only of an achievement it holds.
    if others.is_empty() || held.achievements.is_empty() {
        return Ok(drift);
    }
    let spinner = Spinner::start(
        "steam",
        &format!(
            "asking for them in {}",
            show::counted(others.len(), "more language")
        ),
        false,
    );
    for language in &others {
        match api.schema(app, language) {
            Ok(in_language) => drift.extend(achievements::compare_translations(
                &listed.translations,
                language,
                &in_language.achievements,
                &held.achievements,
            )),
            Err(error) => return Err(web_api_failed(spinner, &error)),
        }
    }
    spinner.done(&format!(
        "in {} as well",
        show::counted(others.len(), "language")
    ));
    Ok(drift)
}

fn try_achievements(
    app: &str,
    check: &[PathBuf],
    writing: &Writing,
    json: bool,
) -> Result<ExitCode, ExitCode> {
    if let Some(out) = writing.out.as_deref().filter(|_| writing.write.is_some()) {
        return try_achievements_write(app, out, writing);
    }
    if json {
        show::divert();
    }
    show::title("achievements");
    let app_id = app_named(app)?;
    let listed = achievements_files(check, app_id, true)?;
    let receipt = match (&listed, &writing.out) {
        (Some(listed), Some(out)) => Some((receipt_in(out, app_id)?, achievement_icons(listed)?)),
        _ => None,
    };
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "asking for the achievements", false);
    let held = match api.schema(app_id, "english") {
        Ok(held) => held,
        Err(error) => {
            let code = web_api_failed(spinner, &error);
            if json {
                show::plain(&format!("{}\n", webapi::error_json(app_id, &error, None)));
            }
            return Err(code);
        }
    };
    let mut holds = show::counted(held.achievements.len(), "achievement");
    if !held.stats.is_empty() {
        holds = format!("{holds} and {}", show::counted(held.stats.len(), "stat"));
    }
    spinner.done(&format!("{holds} on Steam"));
    offer_to_keep(&home, &api, from);
    let Some(listed) = listed else {
        return list_achievements(&api, app_id, &held, json);
    };
    let mut drift = achievements::compare(&listed.achievements, &held.achievements);
    if let Some((receipt, icons)) = &receipt {
        drift.extend(receipt.drift(icons, &held.achievements));
    }
    drift.extend(translation_drift(&api, app_id, &listed, &held)?);
    let totalled = aggregation(&api, app_id, &listed, &held)?;
    let stat_drift = listed
        .stats
        .as_ref()
        .map(|stats| achievements::compare_stats(stats, &held.stats, totalled.as_ref()))
        .unwrap_or_default();
    if drift.is_empty() && stat_drift.is_empty() {
        let what = if listed.stats.is_some() {
            "achievements and stats"
        } else {
            "achievements"
        };
        let files = if check.len() > 1 { "files" } else { "file" };
        show::success(&format!("Steam's {what} match the {files}"), "");
        return Ok(ExitCode::SUCCESS);
    }
    show::note(PUBLISHED);
    for difference in drift.iter().chain(&stat_drift) {
        show::failure(&difference.to_string(), "", None);
    }
    for (kind, wanted, kind_drift) in [
        ("achievements", Some(listed.achievements.len()), &drift),
        ("stats", listed.stats.as_ref().map(Vec::len), &stat_drift),
    ] {
        // A kind the file lists none of and Steam agrees on has nothing to count.
        let Some(wanted) = wanted.filter(|wanted| *wanted > 0 || !kind_drift.is_empty()) else {
            continue;
        };
        let tally = achievements::tally(kind_drift, wanted, kind);
        if kind_drift.is_empty() {
            show::success(&tally, "");
        } else {
            show::failure(&tally, "", None);
        }
    }
    Ok(ExitCode::from(REFUSED))
}

/// Why Steam can differ from an import saved in Steamworks: its Web API and image host give the
/// achievements, stats and icons as last published, where a saved import changes only the editor
/// until then.
const PUBLISHED: &str = "Steam answers with what is published: an import saved in Steamworks \
                         counts here once it is published";

/// What Steam says of whether it adds up over all players each stat it `held` that the `listed`
/// files say it does or does not; nothing is asked when no file says of any.
fn aggregation(
    api: &webapi::Api,
    app: u32,
    listed: &achievements::Listed,
    held: &webapi::Schema,
) -> Result<Option<webapi::Totalled>, ExitCode> {
    let said: Vec<String> = listed
        .stats
        .iter()
        .flatten()
        .filter(|wanted| {
            wanted.aggregated.is_some()
                && held
                    .stats
                    .iter()
                    .any(|stat| stat.api_name == wanted.api_name)
        })
        .map(|wanted| wanted.api_name.clone())
        .collect();
    if said.is_empty() {
        return Ok(None);
    }
    let spinner = Spinner::start("steam", "asking which stats it adds up", false);
    let totalled = match api.totalled(app, &said) {
        Ok(totalled) => totalled,
        Err(error) => return Err(web_api_failed(spinner, &error)),
    };
    spinner.done(&format!(
        "{} of {} aggregated",
        totalled.totals.len(),
        show::counted(said.len(), "stat")
    ));
    let unsaid = said
        .iter()
        .filter(|api_name| totalled.aggregated(api_name).is_none())
        .count();
    if unsaid > 0 {
        show::note(&format!(
            "whether {} {} aggregated went unchecked: Steam answered \"{}\"",
            show::counted(unsaid, "stat"),
            if unsaid == 1 { "is" } else { "are" },
            totalled.withheld.as_deref().unwrap_or_default()
        ));
    }
    Ok(Some(totalled))
}

/// Lists the achievements and stats Steam `held` for `app`, each with how far players have got
/// with it, and prints them as JSON as well when asked.
fn list_achievements(
    api: &webapi::Api,
    app: u32,
    held: &webapi::Schema,
    json: bool,
) -> Result<ExitCode, ExitCode> {
    if held.achievements.is_empty() && held.stats.is_empty() {
        show::field(
            "next",
            &format!(
                "make and publish app {app}'s achievements and stats in Steamworks, under Stats \
                 & Achievements"
            ),
        );
        if json {
            show::plain(&format!("{}\n", held.json(app, &webapi::Global::default())));
        }
        return Ok(ExitCode::SUCCESS);
    }
    let spinner = Spinner::start("steam", "asking how far players have got with them", false);
    let stat_names: Vec<String> = held
        .stats
        .iter()
        .map(|stat| stat.api_name.clone())
        .collect();
    let global = match api.global(app, &stat_names) {
        Ok(global) => global,
        Err(error) => {
            let code = web_api_failed(spinner, &error);
            if json {
                show::plain(&format!("{}\n", webapi::error_json(app, &error, None)));
            }
            return Err(code);
        }
    };
    let totalled = global
        .totals
        .values()
        .filter(|total| total.is_some())
        .count();
    spinner.done(&if global.shares.is_empty() && totalled == 0 {
        "Steam gives no shares of players or totals yet".to_owned()
    } else {
        format!(
            "shares of {} and totals of {}",
            show::counted(global.shares.len(), "achievement"),
            show::counted(totalled, "stat")
        )
    });
    let achievements = held.achievements.iter().map(|achievement| {
        (
            achievement.api_name.clone(),
            achievements::line(
                achievement,
                global.shares.get(&achievement.api_name).copied(),
            ),
        )
    });
    let stats = held.stats.iter().map(|stat| {
        (
            format!("stat {}", stat.api_name),
            achievements::stat_line(
                stat,
                global.totals.get(&stat.api_name).and_then(Option::as_deref),
            ),
        )
    });
    show::fields(&achievements.chain(stats).collect::<Vec<_>>());
    if json {
        show::plain(&format!("{}\n", held.json(app, &global)));
    }
    Ok(ExitCode::SUCCESS)
}

/// Each icon the `listed` achievements give, read and named as the achievement editor puts it on
/// its achievement; refused unless it is a square PNG of 64 pixels or more.
fn achievement_icons(listed: &achievements::Listed) -> Result<Vec<editor::Icon>, ExitCode> {
    let mut icons = Vec::new();
    for wanted in &listed.achievements {
        let defined = &wanted.defined;
        for (path, locked) in [(&defined.icon, false), (&defined.icon_locked, true)] {
            let Some(path) = path else {
                continue;
            };
            let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
            let bytes = fs::read(path).map_err(|error| refused(&error))?;
            let read = picture::read(&bytes).map_err(|why| refused(&why))?;
            if !matches!(read.format, picture::Format::Png { .. })
                || read.width != read.height
                || read.width < 64
            {
                return Err(refused(&format!(
                    "{}'s {} is {} of {} by {} pixels, where the achievement editor takes a \
                     square PNG of 64 pixels or more, 256 at best",
                    wanted.api_name,
                    if locked { "locked icon" } else { "icon" },
                    read.format.with_article(),
                    read.width,
                    read.height
                )));
            }
            icons.push(editor::Icon {
                api_name: wanted.api_name.clone(),
                locked,
                name: editor::icon_name(&wanted.api_name, locked),
                bytes,
            });
        }
    }
    Ok(icons)
}

/// The receipt kept in `out` of the last import of `app`'s icons, refused when there is none.
fn receipt_in(out: &Path, app: u32) -> Result<editor::Receipt, ExitCode> {
    let path = out.join(editor::RECEIPT);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(fail(
                &format!(
                    "{}: has no {}: import and publish the files --write wrote there, then run \
                     --imported",
                    out.display(),
                    editor::RECEIPT
                ),
                REFUSED,
            ));
        }
        Err(error) => return Err(fail(&format!("{}: {error}", path.display()), FAILED)),
    };
    let receipt = editor::Receipt::read(&text)
        .map_err(|why| fail(&format!("{}: {why}", path.display()), REFUSED))?;
    if receipt.app != app {
        return Err(fail(
            &format!(
                "{}: records app {}'s icons, not app {app}'s",
                path.display(),
                receipt.app
            ),
            REFUSED,
        ));
    }
    Ok(receipt)
}

/// The names of the files in `out` of the kinds written for `app`, in order; none when there is
/// no such folder yet.
fn written_before(out: &Path, app: u32) -> Result<Vec<String>, ExitCode> {
    // A folder that cannot be looked at is taken as none yet: making it says why it cannot be.
    let Ok(found) = fs::metadata(out) else {
        return Ok(Vec::new());
    };
    if !found.is_dir() {
        return Err(fail(
            &format!(
                "{}: is a file, where --out names the folder the files are written into",
                out.display()
            ),
            REFUSED,
        ));
    }
    let entries =
        fs::read_dir(out).map_err(|error| fail(&format!("{}: {error}", out.display()), FAILED))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| fail(&format!("{}: {error}", out.display()), FAILED))?;
        if let Ok(name) = entry.file_name().into_string()
            && editor::written_for(app, &name)
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

fn try_achievements_write(app: &str, out: &Path, writing: &Writing) -> Result<ExitCode, ExitCode> {
    let unchanged = writing.unchanged;
    show::title(if unchanged {
        "achievements, unchanged"
    } else if writing.imported {
        "achievements, imported"
    } else {
        "achievements, write"
    });
    let app_id = app_named(app)?;
    let sources = writing.write.as_deref().unwrap_or_default();
    let listed = achievements_files(sources, app_id, false)?.unwrap_or_default();
    let mut package: Vec<(String, Vec<u8>)> = editor::tables(app_id, &listed)
        .into_iter()
        .map(|table| (table.name, table.text.into_bytes()))
        .collect();
    let icons = achievement_icons(&listed)?;
    package.extend(
        icons
            .iter()
            .map(|icon| (icon.name.clone(), icon.bytes.clone())),
    );
    let before = written_before(out, app_id)?;
    let stale: Vec<&(String, Vec<u8>)> = package
        .iter()
        .filter(|(name, bytes)| fs::read(out.join(name)).ok().as_ref() != Some(bytes))
        .collect();
    let left: Vec<&String> = before
        .iter()
        .filter(|name| !package.iter().any(|(written, _)| written == *name))
        .collect();
    let folder = out.display();
    let stale_names: Vec<&String> = stale.iter().map(|(name, _)| name).collect();
    if unchanged {
        return Ok(unchanged_or_not(
            out,
            package.len(),
            &stale_names,
            &left,
            &before,
        ));
    }
    if writing.imported {
        if !stale_names.is_empty() || !left.is_empty() {
            name_differences(&stale_names, &left, &before);
            show::field(
                "next",
                &format!(
                    "run it without --imported to write {folder} as the files make it, import \
                     and publish what it writes, then run --imported"
                ),
            );
            return Ok(ExitCode::from(REFUSED));
        }
        return try_achievements_imported(app_id, &listed.achievements, &icons, out);
    }
    let tables = [
        format!("{app_id}-achievements-definitions.csv"),
        format!("{app_id}-achievements-localization.csv"),
    ];
    let tables_before = tables
        .each_ref()
        .map(|name| fs::read_to_string(out.join(name)).unwrap_or_default());
    let failed = |error: &io::Error| fail(&format!("{folder}: {error}"), FAILED);
    fs::create_dir_all(out).map_err(|error| failed(&error))?;
    for (name, bytes) in &stale {
        fs::write(out.join(name), bytes).map_err(|error| failed(&error))?;
        show::field("wrote", name);
    }
    for name in &left {
        fs::remove_file(out.join(name)).map_err(|error| failed(&error))?;
        show::field("removed", name);
    }
    show::success(
        &format!(
            "{folder} holds the {} the achievement editor imports",
            show::counted(package.len(), "file")
        ),
        "",
    );
    show::field(
        "next",
        &format!(
            "in Steamworks, under app {app_id}'s Stats & Achievements, Bulk Import/Export: drop \
             every CSV and PNG file in {folder} and {SAVED}; then drop \
             {app_id}-achievements-localization.csv again on its own the same way, since the \
             first import applies it only in part; publish, and run this again with --imported"
        ),
    );
    import_notes(&listed);
    changed_notes(&package, &tables, &tables_before, &stale_names);
    Ok(ExitCode::SUCCESS)
}

/// Names what an import of `package` changes that Steam's Web API cannot show, comparing its
/// definitions and localisation, the `tables`, with what they held `before`: each progress bar
/// that moves, and each achievement whose meaning changes while its icons, among the `stale`
/// files written afresh, do not.
fn changed_notes(
    package: &[(String, Vec<u8>)],
    tables: &[String; 2],
    before: &[String; 2],
    stale: &[&String],
) {
    let [defined, localised] = tables.each_ref().map(|table| {
        package
            .iter()
            .find(|(name, _)| name == table)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default()
    });
    let [defined_before, localised_before] = before;
    for (api_name, was, is) in editor::rebarred(defined_before, &defined) {
        show::note(&format!(
            "{api_name}'s progress bar goes from {was} to {is}, which Steam's Web API cannot \
             show: look at it in the editor once imported, and set it there by hand if it has \
             not moved, before publishing"
        ));
    }
    let reworded = editor::reworded((defined_before, localised_before), (&defined, &localised));
    for (api_name, changed) in reworded {
        let drawn = [false, true].map(|locked| editor::icon_name(&api_name, locked));
        let has_icons = package.iter().any(|(name, _)| drawn.contains(name));
        let redrawn = stale.iter().any(|name| drawn.contains(name));
        if !has_icons || redrawn {
            continue;
        }
        let changed = match changed.as_slice() {
            [only] => (*only).to_owned(),
            [first @ .., last] => format!("{} and {last}", first.join(", ")),
            [] => continue,
        };
        show::note(&format!(
            "{api_name}'s {changed} changed and its icons did not: look at them, so that they \
             still show what it asks for"
        ));
    }
}

/// How an import is saved so that all of it is applied: a page left before Pending Changes
/// cleared had applied only some of its icons.
const SAVED: &str = "Save imported data, staying on the page until Pending Changes is gone";

/// Records in `out` the image Steam made of each of `app`'s `icons`, once they are imported and
/// published; refused when Steam lacks one or plainly has not taken the import yet.
fn try_achievements_imported(
    app: u32,
    wanted: &[achievements::Wanted],
    icons: &[editor::Icon],
    out: &Path,
) -> Result<ExitCode, ExitCode> {
    let path = out.join(editor::RECEIPT);
    // Only a readable receipt of this app tells an import Steam has not taken; any other is
    // replaced.
    let last = fs::read_to_string(&path)
        .ok()
        .and_then(|text| editor::Receipt::read(&text).ok())
        .filter(|receipt| receipt.app == app);
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "asking for the achievements", false);
    let held = match api.schema(app, "english") {
        Ok(held) => held,
        Err(error) => return Err(web_api_failed(spinner, &error)),
    };
    spinner.done(&format!(
        "{} on Steam",
        show::counted(held.achievements.len(), "achievement")
    ));
    offer_to_keep(&home, &api, from);
    let receipt = editor::Receipt::of(app, icons, &held.achievements);
    let untaken = untaken_icons(icons, &held.achievements, last.as_ref());
    // An icon uploaded in the editor alone would pass unseen: the receipt records only the
    // icons the files give.
    let unnamed = unnamed_icons(wanted, icons, &held.achievements);
    if !unnamed.is_empty() || !untaken.is_empty() {
        for why in unnamed.iter().chain(&untaken) {
            show::failure(why, "", None);
        }
        let folder = out.display();
        show::field(
            "next",
            &if unnamed.is_empty() {
                format!(
                    "in Steamworks, under app {app}'s Stats & Achievements, import and publish \
                     the files in {folder}, then run --imported again"
                )
            } else {
                format!(
                    "give each of those achievements its `icon` in the files, run it without \
                     --imported to write {folder}, import and publish what it writes, then run \
                     --imported again"
                )
            },
        );
        show::note(PUBLISHED);
        return Ok(ExitCode::from(REFUSED));
    }
    let unlike = unlike_icons(&api, app, icons, &held.achievements);
    if !unlike.is_empty() {
        for why in &unlike {
            show::failure(why, "", None);
        }
        show::field(
            "next",
            &format!(
                "in Steamworks, under app {app}'s Stats & Achievements, Bulk Import/Export: drop \
                 the PNG files in {} again, {SAVED}, and publish, then run --imported again",
                out.display()
            ),
        );
        show::note(PUBLISHED);
        return Ok(ExitCode::from(REFUSED));
    }
    fs::write(&path, receipt.text())
        .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    show::field("wrote", editor::RECEIPT);
    show::success(
        &format!(
            "{} in {} records the {} Steam has published, each as its file",
            editor::RECEIPT,
            out.display(),
            show::counted(icons.len(), "icon")
        ),
        "",
    );
    show::field(
        "next",
        &format!(
            "keep {} with the files; --check with --out names each icon that changes after",
            editor::RECEIPT
        ),
    );
    Ok(ExitCode::SUCCESS)
}

/// Each of the `icons` Steam, among its achievements `held`, has not taken: its achievement or
/// itself missing, or, by the `last` receipt, still the image of an import before the file.
fn untaken_icons(
    icons: &[editor::Icon],
    held: &[webapi::Achievement],
    last: Option<&editor::Receipt>,
) -> Vec<String> {
    let mut untaken: Vec<String> = Vec::new();
    for icon in icons {
        let address = icon.on_steam(held);
        let recorded = last.and_then(|last| last.icons.get(&icon.name));
        let why = if !held
            .iter()
            .any(|achievement| achievement.api_name == icon.api_name)
        {
            // Said once for the achievement, not again for each of its icons.
            format!("{}: not on Steam", icon.api_name)
        } else if address.is_empty() {
            format!("{}: Steam has no {}", icon.api_name, icon.which())
        } else if recorded.is_some_and(|imported| {
            imported.sha256 != icon.sha256() && imported.steam == editor::steam_image(address)
        }) {
            format!(
                "{}: Steam still has the {} the last import made, where the file changed since",
                icon.api_name,
                icon.which()
            )
        } else {
            continue;
        };
        if !untaken.contains(&why) {
            untaken.push(why);
        }
    }
    untaken
}

/// Each achievement `wanted` that Steam, among its achievements `held`, has an icon of where the
/// files give none among their `icons`. A locked icon the files lack is Steam's to make, when the
/// import is told to generate it, so only the icon is asked after.
fn unnamed_icons(
    wanted: &[achievements::Wanted],
    icons: &[editor::Icon],
    held: &[webapi::Achievement],
) -> Vec<String> {
    held.iter()
        .filter(|achievement| {
            !achievement.icon.is_empty()
                && wanted
                    .iter()
                    .any(|wanted| wanted.api_name == achievement.api_name)
                && !icons
                    .iter()
                    .any(|icon| icon.api_name == achievement.api_name && !icon.locked)
        })
        .map(|achievement| {
            format!(
                "{}: the files give no icon, where Steam has one",
                achievement.api_name
            )
        })
        .collect()
}

/// Each of `app`'s `icons` whose image on Steam, among its achievements `held`, is not its file,
/// pixel by pixel, or could not be compared, saying how.
fn unlike_icons(
    api: &webapi::Api,
    app: u32,
    icons: &[editor::Icon],
    held: &[webapi::Achievement],
) -> Vec<String> {
    let spinner = Spinner::start(
        "steam",
        "comparing the icons it serves with the files",
        false,
    );
    let mut unlike = Vec::new();
    for icon in icons {
        let name = editor::steam_image(icon.on_steam(held));
        let compared = api
            .icon_image(app, name)
            .and_then(|served| likeness::compare(&icon.bytes, &served));
        match compared {
            Ok(likeness) if likeness.same() => {}
            Ok(likeness) => unlike.push(format!(
                "{}: Steam's {} is not the file's: {likeness}",
                icon.api_name,
                icon.which()
            )),
            Err(why) => unlike.push(format!(
                "{}: Steam's {} could not be compared with the file: {why}",
                icon.api_name,
                icon.which()
            )),
        }
    }
    let alike = icons.len().saturating_sub(unlike.len());
    let said = format!(
        "{alike} of {} as the files",
        show::counted(icons.len(), "icon")
    );
    if unlike.is_empty() {
        spinner.done(&said);
    } else {
        spinner.failed(&said);
    }
    unlike
}

/// `text` in a game: its English beside the build scripts translated by every catalogue, written
/// there, or with `unchanged` compared with what is there.
fn try_text(unchanged: bool) -> Result<ExitCode, ExitCode> {
    show::title(if unchanged {
        "text, unchanged"
    } else {
        "text, write"
    });
    let game = game()?;
    let folder = game.scripts().to_path_buf();
    let english = text_english(&game, &folder)?;
    let catalogued = text_catalogued(&game)?;
    let files = text::written(&english, &catalogued).map_err(|wrong| refusals(&wrong))?;
    let before = text_kept(&folder, &english)?;
    let stale: Vec<&(String, String)> = files
        .iter()
        .filter(|(name, written)| {
            fs::read_to_string(folder.join(name)).ok().as_ref() != Some(written)
        })
        .collect();
    let left: Vec<&String> = before
        .iter()
        .filter(|name| !files.iter().any(|(written, _)| written == *name))
        .collect();
    let stale_names: Vec<&String> = stale.iter().map(|(name, _)| name).collect();
    if english
        .store
        .as_ref()
        .is_some_and(|store| store.item.is_empty())
    {
        show::note(&format!(
            "{} names no itemid, the store page's own, so no store upload is written",
            text::STORE
        ));
    }
    if unchanged {
        return Ok(unchanged_or_not(
            &folder,
            files.len(),
            &stale_names,
            &left,
            &before,
        ));
    }
    let failed = |error: &io::Error| fail(&format!("{}: {error}", folder.display()), FAILED);
    for (name, written) in &stale {
        let path = folder.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| failed(&error))?;
        }
        fs::write(&path, written).map_err(|error| failed(&error))?;
        show::field("wrote", name);
    }
    for name in &left {
        fs::remove_file(folder.join(name)).map_err(|error| failed(&error))?;
        show::field("removed", name);
    }
    let holds = if files.is_empty() {
        format!("{} holds none of the game's text", folder.display())
    } else {
        format!(
            "{} holds the {} of the game's text in {}",
            folder.display(),
            show::counted(files.len(), "file"),
            show::counted(catalogued.len().saturating_add(1), "language")
        )
    };
    show::success(&holds, "");
    text_next(&files);
    Ok(ExitCode::SUCCESS)
}

/// `text`, or `--template` or `--steam` of it, in a game.
fn text_in(request: cli::Text) -> Result<ExitCode, ExitCode> {
    if request.steam {
        try_text_on_steam()
    } else if request.template {
        try_text_template()
    } else {
        try_text(request.unchanged)
    }
}

/// `text --template` in a game: every English text Steam shows, as a gettext template on standard
/// output, for the game's own template to take in.
fn try_text_template() -> Result<ExitCode, ExitCode> {
    show::divert();
    show::title("text, template");
    let game = game()?;
    let folder = game.scripts().to_path_buf();
    let english = text_english(&game, &folder)?;
    let entries = text::entries(&english).map_err(|wrong| refusals(&wrong))?;
    show::plain(&text::template(&entries));
    show::success(
        &format!(
            "the template holds {} of Steam's text",
            show::counted(entries.len(), "message")
        ),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

/// `languages`: every locale a catalogue may be named by, with Steam's name for its language.
fn run_languages(json: bool) -> ExitCode {
    let languages = po::languages();
    if json {
        let table: serde_json::Map<String, serde_json::Value> = languages
            .iter()
            .map(|(locale, language)| ((*locale).to_owned(), (*language).into()))
            .collect();
        show::plain(&format!("{}\n", serde_json::Value::Object(table)));
    } else {
        show::title("languages");
        let rows: Vec<(String, String)> = languages
            .iter()
            .map(|(locale, language)| ((*locale).to_owned(), (*language).to_owned()))
            .collect();
        show::fields(&rows);
    }
    ExitCode::SUCCESS
}

/// `self check`: this binary's SHA-256 against the one its release's SBOM lists for this system.
fn try_self_check() -> Result<ExitCode, ExitCode> {
    show::title("self check");
    let version = env!("CARGO_PKG_VERSION");
    let Some(target) = itself::target() else {
        return Err(fail(
            &format!(
                "steamship {version} was built for a system no release builds for, so there is no \
                 release binary to compare it with"
            ),
            REFUSED,
        ));
    };
    let path = env::current_exe()
        .map_err(|error| fail(&format!("where this steamship is: {error}"), FAILED))?;
    show::field("binary", &path.display().to_string());
    let mut file = fs::File::open(&path)
        .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    let own = digest::sha256(&mut file)
        .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    let url = itself::sbom(version, |name| env::var(name).ok());
    let spinner = Spinner::start("release", &format!("reading {url}"), false);
    let sbom = match download::text(&url) {
        Ok(sbom) => sbom,
        Err(error) => {
            spinner.failed(&error.to_string());
            return Err(ExitCode::from(FAILED));
        }
    };
    let listed = match itself::listed(&sbom, version, target) {
        Ok(listed) => listed,
        Err(why) => {
            spinner.failed("unreadable");
            return Err(fail(&format!("{url} {why}"), FAILED));
        }
    };
    if listed != own {
        spinner.failed("differs");
        show::field("this binary", &digest::hex(&own));
        show::field("release's", &digest::hex(&listed));
        return Err(fail(
            &format!(
                "{} is not the {target} binary of steamship {version}'s release: it was built \
                 from source, as `cargo install` builds it, or elsewhere, or altered since. \
                 Install the release's own binary, from {} or with `cargo binstall steamship`",
                path.display(),
                update::LATEST
            ),
            REFUSED,
        ));
    }
    spinner.done(&format!("the {target} binary of v{version}"));
    show::success(
        &format!(
            "{} is steamship {version}'s release binary for {target}, byte for byte",
            path.display()
        ),
        &digest::hex(&own),
    );
    Ok(ExitCode::SUCCESS)
}

/// `text --steam` in a game: each system requirement its English and catalogues give, looked for
/// on its public store page in that language.
fn try_text_on_steam() -> Result<ExitCode, ExitCode> {
    show::title("text, on Steam");
    let game = game()?;
    let folder = game.scripts().to_path_buf();
    let english = text_english(&game, &folder)?;
    let catalogued = text_catalogued(&game)?;
    let wanted = match &english.store {
        Some(store) => text::requirements(store, &catalogued).map_err(|wrong| refusals(&wrong))?,
        None => Vec::new(),
    };
    if wanted.is_empty() {
        return Err(fail(
            &format!(
                "{} gives no system requirements, so there are none to look for on the store page",
                text::STORE
            ),
            REFUSED,
        ));
    }
    let app = scripts::load(&game.script)
        .map_err(|problems| {
            let problems: Vec<String> = problems.iter().map(ToString::to_string).collect();
            refusals(&problems)
        })?
        .app_id;
    show::field("app", &app.to_string());
    let store = storefront::Storefront::new();
    let mut languages: Vec<&str> = Vec::new();
    for each in &wanted {
        if !languages.contains(&each.language.as_str()) {
            languages.push(&each.language);
        }
    }
    let mut compared = 0_usize;
    let mut missing = 0_usize;
    let mut reworded: Vec<String> = Vec::new();
    for language in &languages {
        let spinner = Spinner::start("store", &format!("reading the page in {language}"), false);
        let shown = match store.requirements(app, language) {
            Ok(Some(shown)) => {
                spinner.done(language);
                shown
            }
            Ok(None) => {
                spinner.failed(language);
                return Err(fail(
                    &format!(
                        "Steam shows no public store page for app {app}, so its system \
                         requirements can be checked only on its Basic Info form"
                    ),
                    REFUSED,
                ));
            }
            Err(why) => {
                spinner.failed(language);
                return Err(fail(&why, FAILED));
            }
        };
        let (looked, lacking) = requirements_shown(&wanted, language, &shown, &mut reworded);
        compared = compared.saturating_add(looked);
        missing = missing.saturating_add(lacking);
    }
    if !reworded.is_empty() {
        show::note(&format!(
            "Steam rewords {} on the store page, so they are not compared",
            reworded.join(", ")
        ));
    }
    if missing > 0 {
        show::field(
            "next",
            &format!(
                "type the recommended ones from {} on the store page's Basic Info form, upload the \
                 files in {}/ on its Localization tab for the minimum ones, and run it again",
                text::BASIC_INFO,
                text::STORE_FOLDER
            ),
        );
        return Ok(ExitCode::from(REFUSED));
    }
    if compared == 0 {
        return Err(fail(
            &format!(
                "none of the system requirements {} gives is one the store page shows as typed",
                text::STORE
            ),
            REFUSED,
        ));
    }
    show::success(
        &format!(
            "app {app}'s store page shows the {} of the files in {}",
            show::counted(compared, "system requirement"),
            show::counted(languages.len(), "language")
        ),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

/// How many of the requirements `wanted` in `language` were compared with what the store page
/// `shown` holds, and how many it lacks, each named; those Steam rewords are added to `reworded`.
fn requirements_shown(
    wanted: &[text::Requirement],
    language: &str,
    shown: &storefront::Shown,
    reworded: &mut Vec<String>,
) -> (usize, usize) {
    let mut compared = 0_usize;
    let mut missing = 0_usize;
    for each in wanted.iter().filter(|each| each.language == language) {
        let named = format!("{} {}", each.platform, each.field.replace("][", " "));
        if !storefront::comparable(each) {
            if !reworded.contains(&named) {
                reworded.push(named);
            }
            continue;
        }
        compared = compared.saturating_add(1);
        if !storefront::holds(shown, each) {
            missing = missing.saturating_add(1);
            let tier = if each.tier == "rec" {
                "recommended"
            } else {
                "minimum"
            };
            show::failure(
                &format!("{language}: the {tier} {named} is not on the store page"),
                &format!("\"{}\"", each.text),
                None,
            );
        }
    }
    (compared, missing)
}

/// Each of `wrong` shown as refused, and the exit code for it.
fn refusals(wrong: &[String]) -> ExitCode {
    for why in wrong {
        show::failure("refused", why, None);
    }
    ExitCode::from(REFUSED)
}

/// Every catalogue in the game's locale folder, read.
fn text_catalogued(game: &Project) -> Result<Vec<achievements::Catalogued>, ExitCode> {
    let mut catalogued: Vec<achievements::Catalogued> = Vec::new();
    for path in game.catalogues().map_err(|why| fail(&why, REFUSED))? {
        let read = catalogue_file(&path, &catalogued)?;
        catalogued.push(read);
    }
    Ok(catalogued)
}

/// The game's English, read from the files beside its build scripts in `folder`, each shown.
fn text_english(game: &Project, folder: &Path) -> Result<text::English, ExitCode> {
    let read = |name: &str| -> Result<Option<(PathBuf, String)>, ExitCode> {
        let path = folder.join(name);
        match fs::read_to_string(&path) {
            Ok(given) => {
                show::field("file", &path.display().to_string());
                Ok(Some((path, given)))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(fail(&format!("{}: {error}", path.display()), FAILED)),
        }
    };
    let refused = |path: &Path, why: &str| fail(&format!("{}: {why}", path.display()), REFUSED);
    let mut english = text::English::default();
    if let Some((path, given)) = read(text::STORE)? {
        english.store = Some(text::store(&given).map_err(|why| refused(&path, &why))?);
    }
    if let Some((path, given)) = read(text::WORKSHOP)? {
        english.workshop = Some(text::workshop(&given).map_err(|why| refused(&path, &why))?);
    }
    english.presence = match read(text::PRESENCE)? {
        Some((path, given)) => {
            let records = text_records(game)?;
            Some(text::presence(&given, &records).map_err(|why| refused(&path, &why))?)
        }
        None => read(text::PRESENCE_ENGLISH)?
            .map(|(path, given)| text::presence_english(&given).map_err(|why| refused(&path, &why)))
            .transpose()?,
    };
    if english == text::English::default() {
        show::note(&format!(
            "there is no {}, {}, {} or {} beside the build scripts, so there is no English to \
             write from",
            text::STORE,
            text::WORKSHOP,
            text::PRESENCE,
            text::PRESENCE_ENGLISH
        ));
    }
    Ok(english)
}

/// Every record of the game, from each of its record files that holds a list of them.
fn text_records(game: &Project) -> Result<Vec<serde_json::Value>, ExitCode> {
    let mut records = Vec::new();
    for path in game.record_files().map_err(|why| fail(&why, REFUSED))? {
        let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
        let read = fs::read_to_string(&path).map_err(|error| refused(&error))?;
        if let serde_json::Value::Array(listed) =
            serde_json::from_str(&read).map_err(|error| refused(&error))?
        {
            records.extend(listed);
        }
    }
    Ok(records)
}

/// Each file in `folder` of a kind `text` writes from the `english` the game gives, by its path from
/// there: the rich presence files but a hand-kept English one, the store's JSON files, the
/// statement of how AI was used and the sheets typed by hand, and the Workshop's JSON files. A kind whose English the game does
/// not give is left to whatever else writes it.
fn text_kept(folder: &Path, english: &text::English) -> Result<Vec<String>, ExitCode> {
    let failed =
        |path: &Path, error: &io::Error| fail(&format!("{}: {error}", path.display()), FAILED);
    let names = |within: &Path, wanted: &dyn Fn(&str) -> bool| -> Result<Vec<String>, ExitCode> {
        let entries = match fs::read_dir(within) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(failed(within, &error)),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| failed(within, &error))?;
            let is_dir = entry
                .file_type()
                .map_err(|error| failed(within, &error))?
                .is_dir();
            if let Ok(name) = entry.file_name().into_string()
                && !is_dir
                && wanted(&name)
            {
                found.push(name);
            }
        }
        Ok(found)
    };
    let extended = |name: &str, wanted: &str| {
        Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
    };
    let presence = english.presence.as_ref();
    let hand_kept = presence.is_some_and(|told| !told.writes_english);
    let mut kept: Vec<String> = names(folder, &|name| {
        (presence.is_some()
            && name.starts_with("rich_presence_")
            && extended(name, "vdf")
            && !(hand_kept && name == text::PRESENCE_ENGLISH))
            || (english.store.is_some()
                && [text::DISCLOSURE, text::BASIC_INFO, text::SCREENSHOTS].contains(&name))
    })?;
    for (within, given) in [
        (text::STORE_FOLDER, english.store.is_some()),
        (text::WORKSHOP_FOLDER, english.workshop.is_some()),
    ] {
        if !given {
            continue;
        }
        kept.extend(
            names(&folder.join(within), &|name| extended(name, "json"))?
                .into_iter()
                .map(|name| format!("{within}/{name}")),
        );
    }
    kept.sort();
    Ok(kept)
}

/// Where each kind of file `text` wrote goes in Steamworks.
fn text_next(files: &[(String, String)]) {
    let wrote = |prefix: &str| files.iter().any(|(name, _)| name.starts_with(prefix));
    if wrote(text::STORE_FOLDER) {
        show::field(
            "next",
            &format!(
                "upload every file in {}, at once, on the store page's Localization tab",
                text::STORE_FOLDER
            ),
        );
    }
    if wrote(text::BASIC_INFO) {
        show::field(
            "next",
            &format!(
                "type each language's recommended system requirements in {} on the store page's \
                 Basic Info form, as the upload leaves them out; once the page is public, \
                 `steamship text --steam` finds any missed",
                text::BASIC_INFO
            ),
        );
    }
    if wrote(text::SCREENSHOTS) {
        show::field(
            "next",
            &format!(
                "type each language's screenshot alt text in {} into the screenshots' Edit \
                 Accessibility Text panel on the Store Page tab, one language at a time, as the \
                 upload carries none",
                text::SCREENSHOTS
            ),
        );
    }
    if wrote(text::DISCLOSURE) {
        show::field(
            "next",
            &format!(
                "enter each language's statement in {} under the content survey's Generative AI \
                 section",
                text::DISCLOSURE
            ),
        );
    }
    if wrote(text::WORKSHOP_FOLDER) {
        show::field(
            "next",
            &format!(
                "enter each file in {} on the Workshop's Steamworks page, one language to a save",
                text::WORKSHOP_FOLDER
            ),
        );
    }
    if wrote("rich_presence_") {
        show::field(
            "next",
            "steamship rich-presence sends the rich presence files",
        );
    }
}

/// Says whether the `count` files `--write` makes are in `out` as it would write them, naming each
/// of them `stale` and each of those it `left` there from `before`; and the exit code for it.
fn unchanged_or_not(
    out: &Path,
    count: usize,
    stale: &[&String],
    left: &[&String],
    before: &[String],
) -> ExitCode {
    if stale.is_empty() && left.is_empty() {
        show::success(
            &format!(
                "the {} in {} are as the files make them",
                show::counted(count, "file"),
                out.display()
            ),
            "",
        );
        return ExitCode::SUCCESS;
    }
    name_differences(stale, left, before);
    show::field(
        "next",
        "run it again without --unchanged to write them as the files make them",
    );
    ExitCode::from(REFUSED)
}

/// Names each file `stale` in a folder, as missing when it is not among those there `before`, and
/// each `left` over there.
fn name_differences(stale: &[&String], left: &[&String], before: &[String]) {
    for name in stale {
        let why = if before.contains(name) {
            "differs from what the files make"
        } else {
            "is missing"
        };
        show::failure(&format!("{name} {why}"), "", None);
    }
    for name in left {
        show::failure(&format!("{name} is left over"), "", None);
    }
}

/// What the import of what `listed` makes needs beside the files themselves.
fn import_notes(listed: &achievements::Listed) {
    if !listed.sets.is_empty() {
        show::note("each set has to be made in the achievement editor before the import");
    }
    let defined = listed.achievements.iter().map(|wanted| &wanted.defined);
    if defined.clone().any(|defined| defined.progress.is_some()) {
        show::note(
            "each stat a progress bar counts has to be in Steamworks first, with its \
             achievement's permission",
        );
    }
    let bare = defined
        .clone()
        .filter(|defined| defined.icon.is_none())
        .count();
    if bare > 0 {
        show::note(&format!(
            "no icon for {}: Steam shows its own",
            show::counted(bare, "achievement")
        ));
    }
    let unlocked_only = defined
        .filter(|defined| defined.icon.is_some() && defined.icon_locked.is_none())
        .count();
    if unlocked_only > 0 {
        show::note(&format!(
            "no locked icon for {}: tick Automatically generate unachieved icons, and Steam \
             greys the icon",
            show::counted(unlocked_only, "achievement")
        ));
    }
}

/// The rich presence files, each read and checked, refused before Steam is asked anything when
/// one is wrong or two are for the same language.
fn presence_files(files: &[PathBuf]) -> Result<Vec<presence::Language>, ExitCode> {
    let languages = presence_read(files).map_err(|why| fail(&why, REFUSED))?;
    for (path, language) in files.iter().zip(&languages) {
        show::field(
            &language.language,
            &format!(
                "{}, from {}",
                show::counted(language.tokens.len(), "token"),
                path.display()
            ),
        );
    }
    Ok(languages)
}

/// Each rich presence file in `files`, read as `rich-presence` sends it, in their order.
fn presence_read(files: &[PathBuf]) -> Result<Vec<presence::Language>, String> {
    let mut languages: Vec<presence::Language> = Vec::with_capacity(files.len());
    for path in files {
        let refused = |why: &dyn Display| format!("{}: {why}", path.display());
        let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
        let language = presence::read(&text).map_err(|why| refused(&why))?;
        if languages
            .iter()
            .any(|other| other.language == language.language)
        {
            return Err(refused(&format!(
                "is a second file for {}",
                language.language
            )));
        }
        languages.push(language);
    }
    Ok(languages)
}

fn try_rich_presence(app: &str, files: &[PathBuf], preview: bool) -> Result<ExitCode, ExitCode> {
    show::title(if preview {
        "rich-presence, preview"
    } else {
        "rich-presence"
    });
    let app_id = app_named(app)?;
    let languages = presence_files(files)?;
    if preview {
        show::success("the files are sound; nothing was sent, as asked", "");
        return Ok(ExitCode::SUCCESS);
    }
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let spinner = Spinner::start("steam", "sending the rich presence", false);
    if let Err(error) = api.set_rich_presence(&presence::request(app_id, &languages)) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("taken");
    offer_to_keep(&home, &api, from);
    let names: Vec<&str> = languages
        .iter()
        .map(|language| language.language.as_str())
        .collect();
    show::success(
        &format!(
            "app {app_id}: rich presence replaced for {}",
            names.join(", ")
        ),
        "each language's tokens on Steam are now exactly its file's",
    );
    Ok(ExitCode::SUCCESS)
}

fn run_assets(folder: &Path) -> ExitCode {
    show::title("assets");
    show::field("folder", &folder.display().to_string());
    let store = match assets::check(folder) {
        Ok(store) => store,
        Err(error) => return fail(&format!("{}: {error}", folder.display()), REFUSED),
    };
    if !store.unchecked.is_empty() {
        show::field(
            "skipped",
            &format!("{}, named for no language", store.unchecked.join(", ")),
        );
    }
    let checked = store.files;
    if checked.is_empty() {
        return fail(
            &format!(
                "{}: holds no PNG, JPEG, icon or zip, nor a folder for a language with one",
                folder.display()
            ),
            REFUSED,
        );
    }
    let mut taken = Vec::new();
    let mut refused = Vec::new();
    for file in &checked {
        let name = &file.name;
        match &file.found {
            Ok(found) => taken.push((name.clone(), found.clone())),
            Err(problems) => {
                refused.extend(problems.iter().map(|problem| format!("{name}: {problem}")));
            }
        }
    }
    show::fields(&taken);
    let mut wrong = !refused.is_empty();
    for problem in &refused {
        show::failure(problem, "", None);
    }
    if let Some(count) = assets::too_few_screenshots(&checked) {
        wrong = true;
        show::failure(
            &format!(
                "Steamworks needs at least {} screenshots for a store page, and the folder has \
                 {count}",
                assets::SCREENSHOTS
            ),
            "",
            None,
        );
    }
    if wrong {
        return ExitCode::from(REFUSED);
    }
    show::success("the artwork is as Steamworks takes it", "");
    ExitCode::SUCCESS
}

fn try_drm_wrap(
    app: &str,
    executable: &Path,
    output: Option<&Path>,
    mode: drm::Mode,
    named: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    show::title("drm-wrap");
    let app_id = app_named(app)?;
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", executable.display()), REFUSED);
    match magic::looks_like_windows_program(executable) {
        Ok(true) => {}
        Ok(false) => return Err(refused(&"is not a Windows executable")),
        Err(error) => return Err(refused(&error)),
    }
    show::field("program", &executable.display().to_string());
    let given = output.unwrap_or(executable);
    if let Some(output) = output {
        show::field("output", &output.display().to_string());
    }
    let input = path::absolute(executable).map_err(|error| refused(&error))?;
    let destination = path::absolute(given)
        .map_err(|error| fail(&format!("{}: {error}", given.display()), REFUSED))?;
    let (Some(folder), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Err(fail(
            &format!("{}: is not a file's path", destination.display()),
            REFUSED,
        ));
    };
    // Beside where it goes, so that it is moved into place whole, and never copied half-written.
    let wrapped = folder.join(format!(".{}.steamship-drm", name.to_string_lossy()));
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    drop(fs::remove_file(&wrapped));
    let spinner = Spinner::start("steam", "wrapping", true);
    let finished = run::run(
        &program,
        &steamcmd::drm_wrap(&account, app_id, &input, &wrapped, mode),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::WRAP_LIMIT,
        steamcmd::HOPELESS,
    );
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    let written = fs::metadata(&wrapped).is_ok_and(|found| found.len() > 0);
    match drm::judge(finished.code, &String::from_utf8_lossy(&console), written) {
        drm::Outcome::Wrapped => {
            let placed = fs::metadata(&input)
                .and_then(|original| fs::set_permissions(&wrapped, original.permissions()))
                .and_then(|()| fs::rename(&wrapped, &destination));
            if let Err(error) = placed {
                spinner.failed(&format!("wrapped in {took}, not placed"));
                drop(fs::remove_file(&wrapped));
                return Err(fail(&format!("{}: {error}", destination.display()), FAILED));
            }
            spinner.done(&format!("wrapped in {took}"));
            show::success(
                &format!("app {app_id}: {} is wrapped in Steam DRM", given.display()),
                "",
            );
            Ok(ExitCode::SUCCESS)
        }
        drm::Outcome::NotLoggedIn(reason) => {
            drop(fs::remove_file(&wrapped));
            spinner.failed(&format!("refused after {took}"));
            Err(login_refused(&reason, packed.as_ref()))
        }
        drm::Outcome::Failed(reasons) => {
            drop(fs::remove_file(&wrapped));
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            Err(ExitCode::from(FAILED))
        }
    }
}

/// The settings snapshot at `path`, refused before steamcmd is run when it is not `app`'s.
fn settings_file(path: &Path, app: u32) -> Result<vdf::Block, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let wanted = settings::read(&text, app).map_err(|why| refused(&why))?;
    show::field("file", &path.display().to_string());
    Ok(wanted)
}

/// What steamcmd, run from `home` and logged in as `account`, printed of `app`'s info, redacted,
/// with the spinner that said it was `doing` so still turning; refused when Steam did not take
/// the login.
fn app_info(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
    doing: &str,
) -> Result<(String, Spinner), ExitCode> {
    let before = Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let spinner = Spinner::start("steam", doing, true);
    let finished = match run::run(
        &program,
        &steamcmd::app_info(account, app),
        &steamcmd::environment(home, Platform::THIS),
        &root,
        steamcmd::CHECK_LIMIT,
        steamcmd::HOPELESS,
    ) {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed("could not start");
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?);
    let console = String::from_utf8_lossy(&redactor.redact(&finished.output)).into_owned();
    match upload::judge_login(finished.code, &console) {
        upload::Login::Taken => Ok((console, spinner)),
        judged @ (upload::Login::Refused(_) | upload::Login::Failed(_)) => {
            spinner.failed("not logged in");
            Err(verdict(judged, packed))
        }
    }
}

/// The default branch, as [`unreleased`] leaves it: a build live on a released app's default
/// branch is the one its players get, and Valve then asks for a Steam Mobile confirmation, given in
/// Steamworks.
const DEFAULT_RELEASED: (&str, &str) = (
    "its default branch is set live in Steamworks, where Valve asks for a Steam Mobile \
     confirmation",
    "its default branch",
);

/// Whether steamship may do what is only safe before players have the app: only while Steam
/// shows `account` that nobody can buy it yet, asked through steamcmd run from `home`. `left`
/// says what happens instead once the app is released, and what steamship leaves to Steamworks
/// when Steam shows no release state. Answers why not when it may not, and fails when Steam could
/// not be asked.
fn unreleased(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
    (released, left): (&str, &str),
) -> Result<Result<(), String>, ExitCode> {
    let (console, spinner) = app_info(
        home,
        account,
        app,
        packed,
        "asking whether the app is released",
    )?;
    let info = settings::info_from_console(&console, app);
    match info.as_ref().map(release::state) {
        Ok(Some(state)) if release::unreleased(state) => {
            spinner.done(&format!("unreleased, as Steam shows it ({state})"));
            Ok(Ok(()))
        }
        Ok(Some(state)) => {
            spinner.failed(&format!("released, as Steam shows it ({state})"));
            Ok(Err(format!("app {app} is released, so {released}")))
        }
        Ok(None) => {
            spinner.failed("not shown");
            Ok(Err(format!(
                "Steam shows no release state for app {app}, so steamship leaves {left} to \
                 Steamworks"
            )))
        }
        Err(why) => {
            spinner.failed("not shown");
            Err(fail(why, FAILED))
        }
    }
}

/// Whether steamship may set `app`'s default branch live, as [`unreleased`] answers.
fn default_unreleased(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
) -> Result<Result<(), String>, ExitCode> {
    unreleased(home, account, app, packed, DEFAULT_RELEASED)
}

/// Refuses, as [`unreleased`] says why, unless Steam shows the build account named `asked_as`,
/// or the CI login's, that `app` is unreleased.
fn refused_once_released(
    home: &Path,
    app: u32,
    asked_as: Option<&str>,
    left: (&str, &str),
) -> Result<(), ExitCode> {
    let packed = packed_login()?;
    let account = match (&packed, asked_as) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(asked_as, false)?.0,
    };
    let (held, _, manifest) = ready()?;
    restore(packed.as_ref(), home)?;
    let asked = unreleased(home, &account, app, packed.as_ref(), left);
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    asked?.map_err(|why| fail(&why, REFUSED))
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
            Err(fail(&why, FAILED))
        }
    }
}

fn try_settings(
    app: &str,
    save: Option<&Path>,
    check: Option<&Path>,
    named: Option<&str>,
) -> Result<ExitCode, ExitCode> {
    show::title("settings");
    let app_id = app_named(app)?;
    let wanted = check.map(|path| settings_file(path, app_id)).transpose()?;
    let packed = packed_login()?;
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
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
            "`steamship settings --check` compares Steam's with them",
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
        "change the app in Steamworks, or `steamship settings --save` the file again if the \
         change was meant",
    );
    Ok(ExitCode::from(REFUSED))
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

fn try_leaderboards(
    app: &str,
    check: &[PathBuf],
    changes: Changes<'_>,
) -> Result<ExitCode, ExitCode> {
    show::title("leaderboards");
    let app_id = app_named(app)?;
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
        Err(error) => return Err(web_api_failed(asking, &error)),
    };
    asking.done(&format!(
        "{} on Steam",
        show::counted(held.len(), "leaderboard")
    ));
    offer_to_keep(&home, &api, from);
    if check.is_empty() {
        show::fields(
            &held
                .iter()
                .map(|leaderboard| (leaderboard.name.clone(), leaderboards::line(leaderboard)))
                .collect::<Vec<_>>(),
        );
        return Ok(ExitCode::SUCCESS);
    }
    if changes.delete_missing {
        let gone: Vec<String> = held
            .iter()
            .filter(|leaderboard| !wants.holds(&leaderboard.name))
            .map(|leaderboard| leaderboard.name.clone())
            .collect();
        if !gone.is_empty() {
            let released = format!(
                "deleting {} would take its players' scores with it: delete it in Steamworks, \
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
            "Steam keeps a leaderboard's settings: change them in Steamworks, under Stats & \
             Achievements, Leaderboards",
        );
    }
    Ok(ExitCode::from(REFUSED))
}

fn try_branch(app: &str, branch: &str, description: &str) -> Result<ExitCode, ExitCode> {
    show::title("branch");
    let app_id = app_named(app)?;
    if webapi::is_default(branch) {
        return Err(fail(
            &"the default branch's description is set in Steamworks, not by steamship",
            REFUSED,
        ));
    }
    show::field("branch", branch);
    let home = home()?;
    let (api, from) = web_api(&home)?;
    let asking = Spinner::start("steam", "asking for the branches", false);
    let branches = match api.branches(app_id) {
        Ok(branches) => branches,
        Err(error) => {
            let why = why_unanswered(&api, app_id, &error);
            let code = web_api_failed(asking, &error);
            if let Some(why) = why {
                show::note(&why);
            }
            return Err(code);
        }
    };
    let Some(found) = branches.iter().find(|found| found.name == branch) else {
        asking.failed("not on Steam");
        return Err(fail(
            &format!(
                "app {app_id} has no branch \"{branch}\"; {}",
                upload::create_first(&branches)
            ),
            REFUSED,
        ));
    };
    if found.description.is_empty() {
        asking.done("no description");
    } else {
        asking.done(&format!("described as \"{}\"", found.description));
    }
    offer_to_keep(&home, &api, from);
    if found.description == description {
        show::success(
            &format!("app {app_id}: {branch} is already described so"),
            "",
        );
        return Ok(ExitCode::SUCCESS);
    }
    let setting = Spinner::start("steam", "setting the description", false);
    if let Err(error) = api.describe_branch(app_id, branch, description) {
        return Err(web_api_failed(setting, &error));
    }
    setting.done("set");
    show::success(
        &format!("app {app_id}: {branch} described as \"{description}\""),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

fn try_promote(request: &cli::Promote) -> Result<ExitCode, ExitCode> {
    show::title("promote");
    let app_id = app_named(request.app.as_deref().unwrap_or_default())?;
    let build = request.build;
    let default = webapi::is_default(&request.branch);
    let (branch, key) = if default {
        ("default", webapi::DEFAULT_KEY)
    } else {
        (request.branch.as_str(), request.branch.as_str())
    };
    show::field("build", &build.to_string());
    show::field("branch", branch);
    let home = home()?;
    let (api, from) = web_api(&home)?;
    if default {
        refused_once_released(
            &home,
            app_id,
            named(request.account.as_deref()),
            DEFAULT_RELEASED,
        )?;
    }
    let spinner = Spinner::start("steam", "setting it live", false);
    if let Err(error) = api.set_live(app_id, build, key) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("set live");
    offer_to_keep(&home, &api, from);
    show::success(
        &format!("app {app_id}: BuildID {build} live on {branch}"),
        "",
    );
    Ok(ExitCode::SUCCESS)
}

/// Where an item given by its fields keeps its ID: the `--items` file at `path`, as `record`
/// holds it, under `name`.
struct Kept<'command> {
    path: &'command Path,
    name: &'command str,
    record: workshop::Record,
}

/// How the item to upload was given.
enum Given<'command> {
    Script(&'command Path),
    Fields(Box<workshop::Fields>, Kept<'command>),
}

fn try_workshop(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    if request.json {
        show::divert();
    }
    show::title("workshop");
    let given = match request.script.as_deref() {
        Some(script) => {
            show::field("script", &script.display().to_string());
            Given::Script(script)
        }
        None => given_fields(&request.item)?,
    };
    let packed = packed_login()?;
    let named = named(request.account.as_deref());
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let item = checked_item(&given, request.new)?;
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let copy = match &given {
        Given::Script(script) => workshop::prepare(&home, script, item.app_id),
        Given::Fields(fields, kept) => workshop::prepare_fields(&home, fields, kept.name),
    }
    .map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let root = home.join(install::FOLDER);
    let program = steamcmd::program(&root, Platform::THIS);
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "uploading", true);
    let finished = run::run(
        &program,
        &steamcmd::workshop(&account, &copy),
        &steamcmd::environment(&home, Platform::THIS),
        &root,
        steamcmd::UPLOAD_LIMIT,
        steamcmd::HOPELESS,
    );
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&format!("{}: {error}", program.display()), FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    let saved = copy.with_file_name("steamcmd.log");
    fs::write(&saved, &console)
        .map_err(|error| fail(&format!("{}: {error}", saved.display()), FAILED))?;
    let outcome = workshop::judge(
        finished.code,
        &String::from_utf8_lossy(&console),
        workshop::published(&copy),
    );
    let log = format!("log  {}", saved.display());
    if request.json {
        show::plain(&format!(
            "{}\n",
            item_summary(&outcome, &item, &given, &saved)
        ));
    }
    let code = match outcome {
        workshop::Outcome::Published { published } => {
            spinner.done(&format!("uploaded in {took}"));
            output("published-file-id", published, "the item's ID");
            item_published(&given, &item, published, &log)
        }
        workshop::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(&reason, packed.as_ref())
        }
        workshop::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in &reasons {
                show::failure(reason, "", None);
            }
            // Valve gives the real reason for a Workshop failure there, not on the console.
            show::note(&format!(
                "{log}, and steamcmd's workshop log in {}",
                steamcmd_logs.join("workshop_log.txt").display()
            ));
            dump_in_actions(
                &[("steamcmd's console".to_owned(), saved)],
                &[],
                &marks.added(&steamcmd_logs),
                &redactor,
            );
            ExitCode::from(FAILED)
        }
    };
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

/// What came of uploading `item`, given as `given`, as `--json` prints it, with `log`, where
/// steamcmd's console was saved.
fn item_summary(
    outcome: &workshop::Outcome,
    item: &workshop::Item,
    given: &Given<'_>,
    log: &Path,
) -> serde_json::Value {
    let (said, published) = match outcome {
        workshop::Outcome::Published { published } if item.published.is_none() => {
            ("made", Some(*published))
        }
        workshop::Outcome::Published { published } => ("updated", Some(*published)),
        workshop::Outcome::NotLoggedIn(_) => ("not-logged-in", item.published),
        workshop::Outcome::Failed(_) => ("failed", item.published),
    };
    let name = match given {
        Given::Script(_) => None,
        Given::Fields(_, kept) => Some(kept.name),
    };
    serde_json::json!({
        "outcome": said,
        "app": item.app_id,
        "item": published,
        "name": name,
        "log": log.to_string_lossy(),
    })
}

/// Says that `item`, given as `given`, is up as `published`, and keeps a new one's ID where it
/// was given by its fields.
fn item_published(given: &Given<'_>, item: &workshop::Item, published: u64, log: &str) -> ExitCode {
    let said = format!("app {}: Workshop item {published}", item.app_id);
    match (given, item.published) {
        (_, Some(_)) => show::success(&said, log),
        (Given::Script(script), None) => show::success(
            &said,
            &format!(
                "a new item: add \"publishedfileid\" \"{published}\" to {} so that later uploads \
                 update it",
                script.display()
            ),
        ),
        (Given::Fields(_, kept), None) => return keep_item(kept, item.app_id, published),
    }
    ExitCode::SUCCESS
}

/// The item `item` gives by its fields, with the `--items` file it is kept in, as read.
fn given_fields(item: &cli::WorkshopItem) -> Result<Given<'_>, ExitCode> {
    let refused = |why: &dyn Display| fail(why, REFUSED);
    let (Some(app), Some(path), Some(name), Some(content), Some(title)) = (
        item.app.as_deref(),
        item.items.as_deref(),
        item.name.as_deref(),
        item.content.as_deref(),
        item.title.as_deref(),
    ) else {
        return Err(refused(
            &"give the item's script, or --app, --items, --name, --content and --title",
        ));
    };
    let app_id = app_named(app)?;
    workshop::named(name).map_err(|why| refused(&why))?;
    show::field("items", &path.display().to_string());
    let record = match fs::read_to_string(path) {
        Ok(text) => workshop::Record::read(&text)
            .map_err(|why| refused(&format!("{}: {why}", path.display())))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => workshop::Record {
            app_id,
            items: BTreeMap::new(),
        },
        Err(error) => return Err(refused(&format!("{}: {error}", path.display()))),
    };
    if record.app_id != app_id {
        return Err(refused(&format!(
            "{}: is for app {}, not app {app_id}",
            path.display(),
            record.app_id
        )));
    }
    let fields = workshop::Fields {
        app_id,
        published: record.items.get(name).copied(),
        content: content.to_path_buf(),
        preview: item.preview.clone(),
        title: title.to_owned(),
        description: item.description.clone(),
        change_note: item.change_note.clone(),
        visibility: item.visibility,
    };
    Ok(Given::Fields(Box::new(fields), Kept { path, name, record }))
}

/// Writes `published`, the item just made for `app_id`, into the `--items` file under its name,
/// and says so; or, when the file cannot be written, what to add to it by hand.
fn keep_item(kept: &Kept<'_>, app_id: u32, published: u64) -> ExitCode {
    let mut record = kept.record.clone();
    let _: Option<u64> = record.items.insert(kept.name.to_owned(), published);
    let written = kept
        .path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(kept.path, record.text()));
    match written {
        Ok(()) => {
            show::success(
                &format!("app {app_id}: Workshop item {published}"),
                &format!(
                    "a new item, kept in {} as {}: commit it so that later uploads update it",
                    kept.path.display(),
                    kept.name
                ),
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            show::failure(
                &format!("app {app_id}: Workshop item {published} was made"),
                &format!(
                    "{}: {error}; add \"{}\": {published} to its \"items\", or the next upload \
                     makes another",
                    kept.path.display(),
                    kept.name
                ),
                None,
            );
            ExitCode::from(FAILED)
        }
    }
}

/// The Workshop item `given`, checked, and said.
fn checked_item(given: &Given<'_>, new: bool) -> Result<workshop::Item, ExitCode> {
    let item = match given {
        Given::Script(script) => workshop::check(script).map_err(|problems| {
            for problem in &problems {
                show::failure("refused", &problem.to_string(), None);
            }
            ExitCode::from(REFUSED)
        })?,
        Given::Fields(fields, _) => workshop::check_fields(fields).map_err(|problems| {
            for problem in &problems {
                show::failure("refused", problem, None);
            }
            ExitCode::from(REFUSED)
        })?,
    };
    // Nobody keeps the new item's ID from CI, so every run would make another.
    if item.published.is_none() && !new && dump::in_ci(|name| env::var(name).ok()) {
        let why = match given {
            Given::Script(script) => format!(
                "{}: there is no \"publishedfileid\", so every CI run would make a new Workshop \
                 item; add the ID of the item to update, or pass --new to make one",
                script.display()
            ),
            Given::Fields(_, kept) => format!(
                "{} has no item {}, so every CI run would make a new Workshop item; make it \
                 outside CI and commit the file, or pass --new to make one",
                kept.path.display(),
                kept.name
            ),
        };
        show::failure("refused", &why, None);
        return Err(ExitCode::from(REFUSED));
    }
    let which = item.published.map_or_else(
        || "a new item".to_owned(),
        |published| format!("item {published}"),
    );
    show::done(
        "item",
        &format!(
            "app {}, {which}, {}, checked",
            item.app_id,
            show::counted(item.files, "file")
        ),
    );
    Ok(item)
}

/// Hands `build_id` on as the `build-id` output of the GitHub Actions step steamship runs in.
fn hand_on(build_id: u64) {
    output("build-id", build_id, "the BuildID");
}

/// Sets the output `name` of the GitHub Actions step steamship runs in to `value`, through the
/// file GitHub names for a step's outputs. The upload is done by then, so a file that cannot be
/// written is said, as `what`, and does not fail it.
fn output(name: &str, value: u64, what: &str) {
    let Some(outputs) = env::var_os("GITHUB_OUTPUT").filter(|path| !path.is_empty()) else {
        return;
    };
    let written = fs::OpenOptions::new()
        .append(true)
        .open(&outputs)
        .and_then(|mut file| {
            io::Write::write_all(&mut file, format!("{name}={value}\n").as_bytes())
        });
    if let Err(error) = written {
        show::note(&format!(
            "{what} could not be handed to the workflow: {error}"
        ));
    }
}

struct Setup<'command> {
    script: Option<&'command Path>,
    repo: Option<&'command str>,
    account: Option<&'command str>,
    secret: &'command str,
    output: Option<&'command Path>,
}

fn try_ci(setup: &Setup<'_>) -> Result<ExitCode, ExitCode> {
    show::banner_on_terminal();
    show::title("ci");
    if !ci::secret_name(setup.secret) {
        return Err(fail(
            &format!(
                "{} cannot name a GitHub secret: letters, digits and underscores, not starting \
                 with a digit or GITHUB_",
                setup.secret
            ),
            REFUSED,
        ));
    }
    let root = repository_root(Path::new("."));
    let script = ci_script(setup.script, root.as_deref())?;
    // CI is set up before it has built anything, and a checkout seldom holds the build, so the
    // content is left to the check each upload makes.
    drop(checked(&script, true)?);
    show::field("content", "not checked; each upload checks it");
    let repository = match setup.output {
        Some(_) => None,
        None => Some(ci_repository(setup.repo, &script)?),
    };
    let (account, source) = account(setup.account, true)?;
    match source {
        Source::Named => show::field("account", "as named"),
        Source::Remembered => show::field("account", "remembered"),
        Source::Typed => {}
    }
    let home = home()?;
    let packed = ci::Login::saved(&home, Platform::THIS, account.clone()).map_err(|error| {
        if matches!(error, ci::Error::NotSaved { .. }) {
            let login = Hint {
                before: "log in here first with ",
                command: "steamship login",
                after: "",
            };
            show::failure("not logged in", "", Some(login));
            ExitCode::from(LOGIN)
        } else {
            fail(&error, FAILED)
        }
    })?;
    let judged = check_saved_login(&account, None)?;
    if judged != upload::Login::Taken {
        return Ok(verdict(judged, None));
    }
    let today = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let packed = packed.packed(&ci::day(today));
    let again = "when the login expires, run `steamship login` and `steamship ci` again";
    match (setup.output, repository) {
        (Some(output), _) => {
            ci::write_private(output, packed.as_bytes())
                .map_err(|error| fail(&format!("{}: {error}", output.display()), FAILED))?;
            show::done("secret", &format!("written to {}", output.display()));
            show::success(
                "ready for CI",
                &format!(
                    "give your CI the file's contents as {}, then delete the file; {again}",
                    ci::VARIABLE
                ),
            );
        }
        (None, Some(repository)) => {
            fits_a_secret(&packed)?;
            set_secret(&repository, setup.secret, &packed)?;
            let relative = root
                .as_deref()
                .and_then(|root| {
                    path::absolute(&script)
                        .ok()?
                        .strip_prefix(root)
                        .ok()
                        .map(Path::to_path_buf)
                })
                .unwrap_or_else(|| script.clone());
            let (pinned, release) = pinned_action();
            show::success(
                "ready for CI",
                &format!("add this step after the one that builds the content; {again}"),
            );
            show::verbatim(&ci::step(
                &relative.to_string_lossy().replace('\\', "/"),
                &pinned,
                &release,
                setup.secret,
            ));
        }
        (None, None) => {}
    }
    Ok(ExitCode::SUCCESS)
}

/// The top of the Git repository `folder` is in, if it is in one.
fn repository_root(folder: &Path) -> Option<PathBuf> {
    let output = run::output(
        run::command("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(folder),
    )
    .ok()
    .filter(|output| output.status.success())?;
    let root = String::from_utf8(output.stdout).ok()?;
    Some(Path::new(root.trim()).components().collect())
}

/// The app build script named, or the one found in the repository, asked for when there are
/// several and a person to ask.
fn ci_script(named: Option<&Path>, root: Option<&Path>) -> Result<PathBuf, ExitCode> {
    if let Some(script) = named {
        show::field("script", &script.display().to_string());
        return Ok(script.to_path_buf());
    }
    let root = root.ok_or_else(|| {
        fail(
            &"this is not a Git repository; run it in the game's, or name the script with --script",
            REFUSED,
        )
    })?;
    let found = ci::app_scripts(root)
        .map_err(|error| fail(&format!("{}: {error}", root.display()), FAILED))?;
    let shown = |path: &Path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    match found.as_slice() {
        [] => Err(fail(
            &format!(
                "no app build script in {}; name one with --script",
                root.display()
            ),
            REFUSED,
        )),
        [only] => {
            show::field("script", &format!("{}, found", shown(only)));
            Ok(only.clone())
        }
        several if io::stdin().is_terminal() => {
            for (number, script) in several.iter().enumerate() {
                show::field(&number.saturating_add(1).to_string(), &shown(script));
            }
            let picked = typing::ask("script", Echo::Typed)
                .map_err(|error| fail(&format!("the script: {error}"), FAILED))?;
            picked
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|number| several.get(number.checked_sub(1)?))
                .cloned()
                .ok_or_else(|| fail(&"pick a script by its number", REFUSED))
        }
        several => {
            let listed: Vec<String> = several.iter().map(|script| shown(script)).collect();
            Err(fail(
                &format!(
                    "several app build scripts, {}; name one with --script",
                    listed.join(", ")
                ),
                REFUSED,
            ))
        }
    }
}

/// The GitHub repository named, or the one the script's repository's `origin` points at.
fn ci_repository(named: Option<&str>, script: &Path) -> Result<String, ExitCode> {
    if let Some(named) = named {
        let repository =
            ci::github_repository(&format!("https://github.com/{named}")).ok_or_else(|| {
                fail(
                    &format!("{named} is not a repository's owner/name"),
                    REFUSED,
                )
            })?;
        show::field("repo", &format!("{repository}, as named"));
        return Ok(repository);
    }
    let folder = script
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let origin = run::output(
        run::command("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(folder),
    )
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| String::from_utf8(output.stdout).ok());
    let repository = origin
        .as_deref()
        .and_then(ci::github_repository)
        .ok_or_else(|| {
            fail(
                &"the script's repository has no GitHub origin; name one with --repo owner/name, \
                  or write the login to a file with --output",
                REFUSED,
            )
        })?;
    show::field("repo", &format!("{repository}, from origin"));
    Ok(repository)
}

/// Refuses a packed login GitHub would not keep in a secret, before `gh` is asked to.
fn fits_a_secret(packed: &str) -> Result<(), ExitCode> {
    if packed.len() <= ci::SECRET_LIMIT {
        return Ok(());
    }
    let output = Hint {
        before: "write it to a file for another CI with ",
        command: "steamship ci --output login.txt",
        after: "",
    };
    show::failure(
        "the packed login is too large for a GitHub secret",
        &format!(
            "{} KB, where a secret holds 48 KB",
            packed.len().div_ceil(1024)
        ),
        Some(output),
    );
    Err(ExitCode::from(FAILED))
}

/// Sets `packed` as the secret `secret` of `repository` with the GitHub command line, which
/// reads it from its input, so that it is never on a command line or on the screen.
fn set_secret(repository: &str, secret: &str, packed: &str) -> Result<(), ExitCode> {
    let spinner = Spinner::start(
        "secret",
        &format!("setting {secret} on {repository}"),
        false,
    );
    let started = run::spawn(
        run::command("gh")
            .args(["secret", "set", secret, "--repo", repository])
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::piped()),
    );
    let mut child = match started {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            spinner.failed("gh is not installed");
            let install = Hint {
                before: "install it from ",
                command: "https://cli.github.com",
                after: ", or write the login to a file with --output",
            };
            show::failure("the GitHub command line is needed", "", Some(install));
            return Err(ExitCode::from(FAILED));
        }
        Err(error) => {
            spinner.failed("gh could not start");
            return Err(fail(&format!("gh: {error}"), FAILED));
        }
    };
    let written = child.stdin.take().map_or(Ok(()), |mut input| {
        io::Write::write_all(&mut input, packed.as_bytes())
    });
    let finished = child.wait_with_output();
    match (written, finished) {
        (Ok(()), Ok(output)) if output.status.success() => {
            spinner.done(&format!("{secret} set on {repository}"));
            Ok(())
        }
        (_, Ok(output)) => {
            spinner.failed("not set");
            let said = String::from_utf8_lossy(&output.stderr);
            let login = Hint {
                before: "if gh is not logged in, run ",
                command: "gh auth login",
                after: "",
            };
            show::failure("gh could not set the secret", said.trim(), Some(login));
            Err(ExitCode::from(FAILED))
        }
        (_, Err(error)) => {
            spinner.failed("not set");
            Err(fail(&format!("gh: {error}"), FAILED))
        }
    }
}

/// The commit this release of steamship's action is at, and the release, for a step pinned by
/// commit; the release's tag alone when GitHub cannot be asked.
fn pinned_action() -> (String, String) {
    let release = format!("v{}", env!("CARGO_PKG_VERSION"));
    let commit = run::output(run::command("gh").args([
        "api",
        &format!("repos/Aureliolo/steamship/commits/{release}"),
        "--jq",
        ".sha",
    ]))
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|sha| sha.trim().to_owned())
    .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()));
    match commit {
        Some(commit) => (commit, release),
        None => (release.clone(), release),
    }
}
