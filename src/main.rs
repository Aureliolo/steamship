//! The command line.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Display;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{self, Path, PathBuf};
use std::process::{self, ExitCode};
use std::time::SystemTime;

use clap::{CommandFactory as _, FromArgMatches as _};
use steamship::account::Account;
use steamship::cli::{self, Cli, Command, FAILED, LOGIN, REFUSED};
use steamship::install;
use steamship::platform::Platform;
use steamship::project::{self, Project};
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::typing::{self, Echo};
use steamship::update;
use steamship::{
    assets, ci, conversation, digest, download, drm, dump, init, itself, magic, po, release, run,
    scripts, steamcmd, workshop,
};

#[expect(
    clippy::redundant_pub_crate,
    reason = "rustc's unreachable_pub refuses `pub` on an item of a private module, and these are the binary's alone"
)]
mod commands {
    pub(crate) mod achievements;
    pub(crate) mod branch;
    pub(crate) mod builds;
    pub(crate) mod check;
    pub(crate) mod exit;
    pub(crate) mod fetch;
    pub(crate) mod game;
    pub(crate) mod key;
    pub(crate) mod leaderboards;
    pub(crate) mod login;
    pub(crate) mod logout;
    pub(crate) mod presence;
    pub(crate) mod promote;
    pub(crate) mod ready;
    pub(crate) mod released;
    pub(crate) mod rollback;
    pub(crate) mod settings;
    pub(crate) mod status;
    pub(crate) mod text;
    pub(crate) mod upload;
}

use commands::achievements::achievements_in;
use commands::branch::try_branch;
use commands::builds::try_builds;
use commands::check::{check_in, checked};
use commands::exit::{either, fail};
use commands::fetch::try_fetch;
use commands::game::{app_named, app_or_game, game, script_named, shipped_from};
use commands::leaderboards::leaderboards_in;
use commands::login::{try_key_login, try_login};
use commands::logout::run_logout;
use commands::presence::rich_presence_in;
use commands::promote::promote_in;
use commands::ready::{
    Source, account, dump_in_actions, home, installed, login_refused, named, packed_login, ready,
    restore, steamcmd_failed, verdict,
};
use commands::released::{refused_unless, release_shown};
use commands::rollback::try_rollback;
use commands::settings::{Snapshot, settings_in};
use commands::status::{check_saved_login, try_status};
use commands::text::text_in;
use commands::upload::{output, upload_in};

fn main() -> ExitCode {
    let mut cli = Cli::command();
    if io::stdout().is_terminal() {
        cli = cli.before_help(show::banner());
    }
    // Asking for nothing is no mistake: steamship says what it does and exits 0, as with --help.
    // winget's install check starts a package's program so, and fails one that exits otherwise.
    if env::args_os().len() == 1 {
        return if cli.print_help().is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(FAILED)
        };
    }
    let parsed = cli
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let command = match parsed {
        Ok(parsed) => {
            if parsed.times {
                show::time_steps();
            }
            parsed.command
        }
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

/// Prints the completions of `shell` for the command line.
fn run_completions(shell: clap_complete::Shell) -> ExitCode {
    clap_complete::generate(shell, &mut Cli::command(), "steamship", &mut io::stdout());
    ExitCode::SUCCESS
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Completions { shell } => run_completions(shell),
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
            save,
            writing,
            json,
        } => either(achievements_in(app, check, save.as_deref(), writing, json)),
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
        Command::Rollback(request) => either(try_rollback(&request)),
        Command::Branch {
            app,
            branch,
            description,
        } => either(try_branch(&app, &branch, &description)),
        Command::Workshop(request) => either(workshop_in(&request)),
        Command::Login {
            web_api_key: true, ..
        } => either(try_key_login()),
        Command::Login { account, .. } => either(try_login(named(account.as_deref()))),
        Command::Logout => run_logout(),
        Command::Status { account } => either(try_status(named(account.as_deref()))),
        Command::Upload(request) => upload_in(&request),
    }
}

/// `workshop` of the script given, or of the item its fields give, or of the game's example mods
/// when there is neither.
fn workshop_in(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    match (&request.script, &request.item) {
        (Some(script), _) => try_workshop(request, || {
            show::field("script", &script.display().to_string());
            Ok(Given::Script(script))
        }),
        // clap takes `--app` only with `--name`. No name reads as the empty one, which is refused.
        (None, Some(item)) => try_workshop(request, || {
            let name = request.name.as_deref().unwrap_or_default();
            given_fields(item, name, request.visibility)
        }),
        (None, None) => try_game_workshop(request),
    }
}

/// `workshop` in a game: each example mod, or the one `--name` names, from a pushed tree.
fn try_game_workshop(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    let game = game()?;
    let mods = game.mods().map_err(|why| fail(&why, REFUSED))?;
    let chosen: Vec<project::Mod> = match &request.name {
        Some(name) => mods.into_iter().filter(|each| each.name == *name).collect(),
        None => mods,
    };
    if chosen.is_empty() {
        return Err(fail(
            &request.name.as_ref().map_or_else(
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
    // A file that cannot be read is named by the upload itself. Until then no item counts as made.
    let made = fs::read_to_string(&game.items)
        .ok()
        .and_then(|text| workshop::Record::read(&text).ok())
        .map(|record| record.items)
        .unwrap_or_default();
    for each in chosen {
        // Visibility is chosen once, for an item being made: a published item keeps whatever it
        // was given on Steam since.
        let visibility = request
            .visibility
            .filter(|_| !made.contains_key(&each.name));
        let item = cli::WorkshopItem {
            app: app.clone(),
            items: game.items.clone(),
            content: each.folder,
            preview: Some(each.preview),
            title: each.title,
            description: Some(each.description),
            change_note: Some(format!("Version {}", each.version)),
        };
        let uploaded = either(try_workshop(request, || {
            given_fields(&item, &each.name, visibility)
        }));
        if uploaded != ExitCode::SUCCESS {
            return Ok(uploaded);
        }
    }
    Ok(ExitCode::SUCCESS)
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
    let published = itself::sbom(version, |name| env::var(name).ok());
    let url = &published.download;
    let spinner = Spinner::start("release", &format!("reading {url}"), false);
    let sbom = match download::asset_text(published.asset()) {
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
                "{} is not the {target} binary of steamship {version}'s release. It was built \
                 from source (as by cargo install), built elsewhere, or altered since. Install \
                 the release's own binary from {}, or with cargo binstall steamship",
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
    drop(fs::remove_file(&wrapped));
    let spinner = Spinner::start("steam", "wrapping", true);
    let finished = steamcmd::drm_wrap(&account, app_id, &input, &wrapped, mode).run(&home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
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

/// Whether a Workshop item for `app`, whose game's settings `file` holds its items until launch,
/// may go up: only once Steam shows `account` the app released. Answers why not when it may not,
/// and fails when Steam could not be asked.
fn launched(
    home: &Path,
    account: &Account,
    app: u32,
    packed: Option<&ci::Login>,
    file: &Path,
) -> Result<Result<(), String>, ExitCode> {
    let (state, spinner) = release_shown(home, account, app, packed)?;
    let held = format!(
        "{} holds the game's Workshop items until launch. Give --for-launch to the run that \
         readies the launch",
        file.display()
    );
    Ok(match state.as_deref() {
        Some(state) if release::unreleased(state) => {
            spinner.failed(&format!("unreleased, as Steam shows it ({state})"));
            Err(format!("app {app} is unreleased, and {held}"))
        }
        Some(state) => {
            spinner.done(&format!("released, as Steam shows it ({state})"));
            Ok(())
        }
        None => {
            spinner.failed("not shown");
            Err(format!(
                "Steam shows no release state for app {app}, and {held}"
            ))
        }
    })
}

/// Refuses an item for `app` while Steam shows it unreleased, when the game the current folder is
/// in holds its Workshop items until launch and `app` is the game's own.
fn held_until_launch(
    app: u32,
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<(), ExitCode> {
    let here = env::current_dir()
        .map_err(|error| fail(&format!("the current folder: {error}"), FAILED))?;
    let Some(game) = Project::find(&here).map_err(|why| fail(&why, REFUSED))? else {
        return Ok(());
    };
    if !game.held_until_launch || scripts::app_named(&game.script) != Some(app) {
        return Ok(());
    }
    let home = home()?;
    let file = game.root.join(project::FILE);
    refused_unless(&home, packed, || {
        launched(&home, account, app, packed, &file)
    })
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

/// `workshop` for one item, the one `given` reads once the title is shown. `request` gives the
/// account and whether the item may be new, goes up before launch and is summed up as JSON.
fn try_workshop<'command>(
    request: &cli::Workshop,
    given: impl FnOnce() -> Result<Given<'command>, ExitCode>,
) -> Result<ExitCode, ExitCode> {
    if request.json {
        show::divert();
    }
    show::title("workshop");
    let given = given()?;
    let packed = packed_login()?;
    let named = named(request.account.as_deref());
    let account = match (&packed, named) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(named, false)?.0,
    };
    let item = checked_item(&given, request.new)?;
    if !request.for_launch {
        held_until_launch(item.app_id, &account, packed.as_ref())?;
    }
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let copy = match &given {
        Given::Script(script) => workshop::prepare(&home, script, item.app_id),
        Given::Fields(fields, kept) => workshop::prepare_fields(&home, fields, kept.name),
    }
    .map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(&home).map_err(|error| fail(&error, FAILED))?;
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "uploading", true);
    let finished = steamcmd::workshop(&account, &copy).run(&home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
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
                "a new item. Add \"publishedfileid\" \"{published}\" to {} so that later uploads \
                 update it",
                script.display()
            ),
        ),
        (Given::Fields(_, kept), None) => return keep_item(kept, item.app_id, published),
    }
    ExitCode::SUCCESS
}

/// The item `item` gives by its fields, kept as `name` and seen as `visibility` says, with the
/// `--items` file it is kept in, as read.
fn given_fields<'command>(
    item: &'command cli::WorkshopItem,
    name: &'command str,
    visibility: Option<workshop::Visibility>,
) -> Result<Given<'command>, ExitCode> {
    let refused = |why: &dyn Display| fail(why, REFUSED);
    let path = item.items.as_path();
    let app_id = app_named(&item.app)?;
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
        content: item.content.clone(),
        preview: item.preview.clone(),
        title: item.title.clone(),
        description: item.description.clone(),
        change_note: item.change_note.clone(),
        visibility,
    };
    Ok(Given::Fields(Box::new(fields), Kept { path, name, record }))
}

/// Writes `published`, the item just made for `app_id`, into the `--items` file under its name,
/// and says so. When the file cannot be written, it says what to add to it by hand.
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
                    "a new item, kept in {} as {}. Commit it so that later uploads update it",
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
                    "{}: {error}. Add \"{}\": {published} to its \"items\", or the next upload \
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
                 item. Add the ID of the item to update, or pass --new to make one",
                script.display()
            ),
            Given::Fields(_, kept) => format!(
                "{} has no item {}, so every CI run would make a new Workshop item. Make it \
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
    show::field("content", "not checked (each upload checks it)");
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
    if judged != conversation::Login::Taken {
        return Ok(verdict(judged, None));
    }
    let today = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let packed = packed.packed(&ci::day(today));
    let again = "When the login expires, run steamship login and steamship ci again";
    match (setup.output, repository) {
        (Some(output), _) => {
            ci::write_private(output, packed.as_bytes())
                .map_err(|error| fail(&format!("{}: {error}", output.display()), FAILED))?;
            show::done("secret", &format!("written to {}", output.display()));
            show::success(
                "ready for CI",
                &format!(
                    "give your CI the file's contents as {}, then delete the file. {again}",
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
                &format!("add this step after the one that builds the content. {again}"),
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
            &"this folder is in no Git repository. Run steamship ci in the game's, or name the \
              script with --script",
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
                "no app build script in {}. Name one with --script",
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
                    "several app build scripts, {}. Name one with --script",
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
                &"the script's repository has no GitHub origin. Name one with --repo owner/name, \
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
            "{} KB, where a secret holds {} KB",
            packed.len().div_ceil(1024),
            ci::SECRET_LIMIT.div_ceil(1024)
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
/// commit. The release's tag alone when GitHub cannot be asked.
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
