//! The command line.

use std::env;
use std::io::{self, IsTerminal as _};
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _};
use steamship::cli::{self, Cli, Command, FAILED};
use steamship::platform::Platform;
use steamship::show;
use steamship::update;
use steamship::{drm, po, run};

#[expect(
    clippy::redundant_pub_crate,
    reason = "rustc's unreachable_pub refuses `pub` on an item of a private module, and these are the binary's alone"
)]
mod commands {
    pub(crate) mod achievements;
    pub(crate) mod assets;
    pub(crate) mod branch;
    pub(crate) mod builds;
    pub(crate) mod check;
    pub(crate) mod ci;
    pub(crate) mod drm;
    pub(crate) mod exit;
    pub(crate) mod fetch;
    pub(crate) mod game;
    pub(crate) mod init;
    pub(crate) mod install;
    pub(crate) mod itself;
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
    pub(crate) mod workshop;
}

use commands::achievements::achievements_in;
use commands::assets::run_assets;
use commands::branch::try_branch;
use commands::builds::try_builds;
use commands::check::check_in;
use commands::ci::{Setup, try_ci};
use commands::drm::try_drm_wrap;
use commands::exit::either;
use commands::fetch::try_fetch;
use commands::game::app_or_game;
use commands::init::run_init;
use commands::install::run_install;
use commands::itself::try_self_check;
use commands::leaderboards::leaderboards_in;
use commands::login::{try_key_login, try_login};
use commands::logout::run_logout;
use commands::presence::rich_presence_in;
use commands::promote::promote_in;
use commands::ready::{installed, named};
use commands::rollback::try_rollback;
use commands::settings::{Snapshot, settings_in};
use commands::status::try_status;
use commands::text::text_in;
use commands::upload::upload_in;
use commands::workshop::workshop_in;

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

/// Whether a newer steamship is out, found out while the command runs, when a person at a
/// terminal would be told.
fn update_check() -> Option<update::Check> {
    if !update::wanted(|name| env::var_os(name), io::stderr().is_terminal()) {
        return None;
    }
    let home = Platform::THIS.home(|name| env::var_os(name)).ok()?;
    Some(update::Check::start(home))
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
