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
    assets, branch, builds, ci, conversation, digest, download, drm, dump, fetch, init, itself,
    keychain, leaderboards, ledger, magic, po, presence, release, run, scripts, settings, steamcmd,
    upload, vdf, webapi, workshop,
};
use zeroize::Zeroizing;

#[expect(
    clippy::redundant_pub_crate,
    reason = "rustc's unreachable_pub refuses `pub` on an item of a private module, and these are the binary's alone"
)]
mod commands {
    pub(crate) mod achievements;
    pub(crate) mod check;
    pub(crate) mod exit;
    pub(crate) mod game;
    pub(crate) mod key;
    pub(crate) mod login;
    pub(crate) mod logout;
    pub(crate) mod ready;
    pub(crate) mod released;
    pub(crate) mod status;
    pub(crate) mod text;
    pub(crate) mod upload;
}

use commands::achievements::{achievements_in, written_over};
use commands::check::{check_in, checked};
use commands::exit::{either, fail};
use commands::game::{
    app_given_or, app_named, app_or_game, game, game_as, script_named, shipped_from,
};
use commands::key::{offer_to_keep, web_api, web_api_failed, why_unanswered};
use commands::login::{try_key_login, try_login};
use commands::logout::run_logout;
use commands::ready::{
    Source, account, dump_in_actions, home, installed, login_refused, named, packed_login, ready,
    restore, steamcmd_failed, verdict,
};
use commands::released::{
    DEFAULT_RELEASED, app_info, refused_once_released, refused_unless, release_shown,
};
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

/// How `leaderboards` with no files to check gives what Steam holds.
#[derive(Debug, Clone, Copy)]
enum Listing<'command> {
    /// A line for each.
    Lines,
    /// One line of JSON on standard output.
    Json,
    /// Written to this file, as `--check` reads it.
    Saved(&'command Path),
}

/// `leaderboards` with whatever is not given taken from the game.
fn leaderboards_in(request: cli::Leaderboards) -> Result<ExitCode, ExitCode> {
    let cli::Leaderboards {
        app,
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
    try_leaderboards(&app_given_or(app, game.as_ref()), &files, listing, changes)
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
                    "the game holds no rich_presence_<language>.vdf beside {}. Write \
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
                    builds::error_json(app_id, &error, why.as_deref())
                ));
            }
            return Err(code);
        }
    };
    spinner.done(&builds::summary(&overview));
    show::fields(&builds::lines(&overview));
    offer_to_keep(&home, &api, from);
    if json {
        show::plain(&format!("{}\n", builds::json(app_id, &overview)));
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
    let shown = branch::shown_name(&branch.name);
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
                 {} live on {shown}. If a build was set live meanwhile, fetch again",
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
                    "holds app {app}'s {} branch. Fetch {branch} into a folder of its own",
                    held.branch
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some()) {
                return Err(refused(&format!(
                    "holds files and no build of app {app}. Fetch into an empty folder, or one a \
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
    let shown = branch::shown_name(branch);
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
                    "app {app}'s {shown} branch takes a password. Set {} to it, or run fetch \
                     without --json and type it when asked",
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

/// The password kept for `app`'s `branch`, if one is and the store can be read. A store that
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
        "Enter keeps the password in {} for next time, and n does not",
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
            return Err(fail(&error, FAILED));
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
    let branch = branch::key(&fetching.branch.name);
    let system = fetch::system_name(fetching.system);
    if fetching.password.is_some() && !fetching.on_command_line {
        return steamcmd::fetch_console(account, system, fetching.folder).typed(
            home,
            &fetch::typed_command(fetching.app, branch, secret),
            secret,
        );
    }
    steamcmd::fetch(
        account,
        fetching.app,
        branch,
        fetching.password.map(|_| secret.as_str()),
        system,
        fetching.folder,
    )
    .run(home)
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
            "the password kept in {} is forgotten. Fetch again to give the one the branch takes \
             now",
            keychain::STORE
        )),
        Err(error) => show::failure(&error.to_string(), "", None),
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
        show::success("the files are sound, and nothing was sent, as asked", "");
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

/// The settings snapshot at `path`, refused before steamcmd is run when it is not `app`'s.
fn settings_file(path: &Path, app: u32) -> Result<vdf::Block, ExitCode> {
    let refused = |why: &dyn Display| fail(&format!("{}: {why}", path.display()), REFUSED);
    let text = fs::read_to_string(path).map_err(|error| refused(&error))?;
    let wanted = settings::read(&text, app).map_err(|why| refused(&why))?;
    show::field("file", &path.display().to_string());
    Ok(wanted)
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

fn try_leaderboards(
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

fn try_branch(app: &str, branch: &str, description: &str) -> Result<ExitCode, ExitCode> {
    show::title("branch");
    let app_id = app_named(app)?;
    if branch::is_default(branch) {
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
                "app {app_id} has no branch \"{branch}\". {}",
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
    let default = branch::is_default(&request.branch);
    let branch = branch::shown_name(&request.branch);
    let key = branch::key(&request.branch);
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

/// Sets `branch` of `app` back to the build the record shows live on it before the current one.
fn try_rollback(request: &cli::Rollback) -> Result<ExitCode, ExitCode> {
    show::title("rollback");
    let app_id = app_named(&request.app)?;
    let home = home()?;
    let entries = ledger::load(&home, app_id).map_err(|why| fail(&why, FAILED))?;
    let default = branch::is_default(&request.branch);
    let name = branch::shown_name(&request.branch);
    let (current, before) =
        ledger::previous(&entries, app_id, name).map_err(|why| fail(&why, REFUSED))?;
    show::field("branch", name);
    show::field(
        "from",
        &format!(
            "BuildID {}, commit {}",
            current.build_id,
            project::short(&current.commit)
        ),
    );
    show::field(
        "to",
        &format!(
            "BuildID {}, commit {}",
            before.build_id,
            project::short(&before.commit)
        ),
    );
    let (api, from) = web_api(&home)?;
    if default {
        refused_once_released(
            &home,
            app_id,
            named(request.account.as_deref()),
            DEFAULT_RELEASED,
        )?;
    }
    let key = branch::key(&request.branch);
    let spinner = Spinner::start("steam", "setting it live", false);
    if let Err(error) = api.set_live(app_id, before.build_id, key) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("set live");
    offer_to_keep(&home, &api, from);
    let back = ledger::Entry {
        app: app_id,
        branch: name.to_owned(),
        build_id: before.build_id,
        commit: before.commit.clone(),
        green: before.green,
        rollback: true,
    };
    if let Err(why) = ledger::record(&home, back) {
        show::note(&format!(
            "the rollback is live on Steam but was not recorded, so another rollback of {name} \
             would set the same build again: {why}"
        ));
    }
    show::success(
        &format!(
            "app {app_id}: {name} is back on BuildID {}",
            before.build_id
        ),
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
