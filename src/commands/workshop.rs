//! `steamship workshop`: a Workshop item checked, uploaded and its ID kept.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use steamship::account::Account;
use steamship::cli::{self, FAILED, REFUSED};
use steamship::install;
use steamship::platform::Platform;
use steamship::project::{self, Project};
use steamship::redact::Redactor;
use steamship::show::{self, Spinner};
use steamship::{ci, dump, release, scripts, steamcmd, workshop};

use super::exit::fail;
use super::game::{app_named, app_or_game, game, script_given, shipped_from};
use super::ready::{
    dump_in_actions, home, login_refused, named, ready, restore, run_as, steamcmd_failed,
};
use super::released::{refused_unless, release_shown};
use super::upload::output;

/// `workshop` of the item its fields give, after the app or the game's own. With no field, a first
/// word that is not a number is an item script, and a number or no word is the game's example mods.
pub(crate) fn workshop_in(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    // clap takes `--items` only with `--name`. No name reads as the empty one, which is refused.
    if let Some(item) = &request.item {
        return try_workshop(request, || {
            let app = app_or_game(request.app.clone(), false)?;
            let name = request.name.as_deref().unwrap_or_default();
            Ok(vec![given_fields(&app, item, name, request.visibility)?])
        });
    }
    match request.app.as_deref() {
        Some(word) if word.parse::<u32>().is_err() => {
            try_workshop(request, || given_script(word, request))
        }
        _ => try_game_workshop(request),
    }
}

/// The item script `word` names, refused when a flag of the game's own mods stands beside it.
fn given_script<'command>(
    word: &'command str,
    request: &cli::Workshop,
) -> Result<Vec<Given<'command>>, ExitCode> {
    let beside = [
        ("--name", request.name.is_some()),
        ("--visibility", request.visibility.is_some()),
    ];
    let refused: Vec<ExitCode> = beside
        .into_iter()
        .filter(|&(_, given)| given)
        .map(|(flag, _)| {
            fail(
                &format!(
                    "{flag} does not go with the item script {word}. Drop {flag}, or leave out \
                     the script to use it for the game's own mods"
                ),
                REFUSED,
            )
        })
        .collect();
    if let Some(&code) = refused.first() {
        return Err(code);
    }
    show::field("script", word);
    Ok(vec![Given::Script(Path::new(word))])
}

/// `workshop` in a game: each example mod, or the one `--name` names, from a pushed tree. A dry run
/// needs no pushed tree, like `rich-presence`'s.
fn try_game_workshop(request: &cli::Workshop) -> Result<ExitCode, ExitCode> {
    let script = script_given(request.app.as_deref(), false)?;
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
    if !request.dry_run {
        shipped_from(&game)?;
    }
    let app = script.display().to_string();
    // A file that cannot be read is named by the upload itself. Until then no item counts as made.
    let made = fs::read_to_string(&game.items)
        .ok()
        .and_then(|text| workshop::Record::read(&text).ok())
        .map_or_default(|record| record.items);
    let each_item: Vec<(cli::WorkshopItem, String, Option<workshop::Visibility>)> = chosen
        .into_iter()
        .map(|each| {
            // Visibility is chosen once, for an item being made: a published item keeps whatever
            // it was given on Steam since.
            let visibility = request
                .visibility
                .filter(|_| !made.contains_key(&each.name));
            let item = cli::WorkshopItem {
                items: game.items.clone(),
                content: each.folder,
                preview_image: Some(each.preview),
                title: each.title,
                description: Some(each.description),
                change_note: Some(format!("Version {}", each.version)),
            };
            (item, each.name, visibility)
        })
        .collect();
    try_workshop(request, || {
        each_item
            .iter()
            .map(|(item, name, visibility)| given_fields(&app, item, name, *visibility))
            .collect()
    })
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

/// Where an item given by its fields keeps its ID: under `name` in the `--items` file at `path`.
struct Kept<'command> {
    path: &'command Path,
    name: &'command str,
}

/// How the item to upload was given.
enum Given<'command> {
    Script(&'command Path),
    Fields(Box<workshop::Fields>, Kept<'command>),
}

/// `workshop` for the items of one run. Every item is checked before the account is asked for, and
/// a dry run ends there. The items share one app, so the account, the release question and the home
/// are asked once.
fn try_workshop<'command>(
    request: &cli::Workshop,
    given: impl FnOnce() -> Result<Vec<Given<'command>>, ExitCode>,
) -> Result<ExitCode, ExitCode> {
    if request.json {
        show::divert();
    }
    show::title("workshop");
    let given = given()?;
    let items = given
        .iter()
        .map(|each| checked_item(each, request.new))
        .collect::<Result<Vec<_>, _>>()?;
    if request.dry_run {
        if request.json {
            for (each, item) in given.iter().zip(&items) {
                show::plain(&format!("{}\n", item_summary(None, item, each)));
            }
        }
        if let Some(first) = items.first() {
            show::success(
                &format!(
                    "app {}: {} checked, nothing uploaded",
                    first.app_id,
                    show::counted(items.len(), "item")
                ),
                "",
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    let Some(app_id) = items.first().map(|item| item.app_id) else {
        return Ok(ExitCode::SUCCESS);
    };
    let named = named(request.account.as_deref());
    let (packed, account) = run_as(named)?;
    if !request.for_launch {
        held_until_launch(app_id, &account, packed.as_ref())?;
    }
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    let mut code = ExitCode::SUCCESS;
    for (each, item) in given.iter().zip(&items) {
        code = upload_item(request, (&account, packed.as_ref(), &home), each, item)?;
        if code != ExitCode::SUCCESS {
            break;
        }
    }
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok(code)
}

/// Uploads `item`, given as `given`, and says what came of it. `run` holds the account, the login
/// handed over and the home the run readied.
fn upload_item(
    request: &cli::Workshop,
    run: (&Account, Option<&ci::Login>, &Path),
    given: &Given<'_>,
    item: &workshop::Item,
) -> Result<ExitCode, ExitCode> {
    let (account, packed, home) = run;
    let copy = match given {
        Given::Script(script) => workshop::prepare(home, script, item.app_id),
        Given::Fields(fields, kept) => workshop::prepare_fields(home, fields, kept.name),
    }
    .map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?;
    let steamcmd_logs = steamcmd::state_folder(home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let spinner = Spinner::start("steam", "uploading", true);
    let finished = steamcmd::workshop(account, &copy).run(home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
        }
    };
    let redactor = before.and(Redactor::for_home(home).map_err(|error| fail(&error, FAILED))?);
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
            item_summary(Some((&outcome, &saved)), item, given)
        ));
    }
    let code = match outcome {
        workshop::Outcome::Published { published } => {
            spinner.done(&format!("uploaded in {took}"));
            output("published-file-id", published, "the item's ID");
            item_published(given, item, published, &log)
        }
        workshop::Outcome::NotLoggedIn(reason) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(&reason, packed)
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
    Ok(code)
}

/// What came of `item`, given as `given`, as `--json` prints it. An upload gives its outcome with
/// `log`, where steamcmd's console was saved. A dry run gives `None`, and the ID the item already
/// has.
fn item_summary(
    uploaded: Option<(&workshop::Outcome, &Path)>,
    item: &workshop::Item,
    given: &Given<'_>,
) -> serde_json::Value {
    let (said, published) = match uploaded.map(|(outcome, _)| outcome) {
        None => ("dry-run", item.published),
        Some(workshop::Outcome::Published { published }) if item.published.is_none() => {
            ("made", Some(*published))
        }
        Some(workshop::Outcome::Published { published }) => ("updated", Some(*published)),
        Some(workshop::Outcome::NotLoggedIn(_)) => ("not-logged-in", item.published),
        Some(workshop::Outcome::Failed(_)) => ("failed", item.published),
    };
    let log = uploaded.map(|(_, log)| log.to_string_lossy());
    let name = match given {
        Given::Script(_) => None,
        Given::Fields(_, kept) => Some(kept.name),
    };
    serde_json::json!({
        "outcome": said,
        "app": item.app_id,
        "item": published,
        "name": name,
        "log": log,
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

/// The `--items` file at `path` for `app_id`. No file is an empty record, and a record for another
/// app is refused.
fn record_in(path: &Path, app_id: u32) -> Result<workshop::Record, String> {
    let record = match fs::read_to_string(path) {
        Ok(text) => {
            workshop::Record::read(&text).map_err(|why| format!("{}: {why}", path.display()))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => workshop::Record {
            app_id,
            items: BTreeMap::new(),
        },
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    if record.app_id != app_id {
        return Err(format!(
            "{}: is for app {}, not app {app_id}",
            path.display(),
            record.app_id
        ));
    }
    Ok(record)
}

/// The item `item` gives by its fields for `app`, kept as `name` and seen as `visibility` says.
/// Its ID is the one the `--items` file holds under `name`.
fn given_fields<'command>(
    app: &str,
    item: &'command cli::WorkshopItem,
    name: &'command str,
    visibility: Option<workshop::Visibility>,
) -> Result<Given<'command>, ExitCode> {
    let refused = |why: &dyn Display| fail(why, REFUSED);
    let path = item.items.as_path();
    let app_id = app_named(app)?;
    workshop::named(name).map_err(|why| refused(&why))?;
    show::field("items", &path.display().to_string());
    let record = record_in(path, app_id).map_err(|why| refused(&why))?;
    let fields = workshop::Fields {
        app_id,
        published: record.items.get(name).copied(),
        content: item.content.clone(),
        preview: item.preview_image.clone(),
        title: item.title.clone(),
        description: item.description.clone(),
        change_note: item.change_note.clone(),
        visibility,
    };
    Ok(Given::Fields(Box::new(fields), Kept { path, name }))
}

/// Writes `published`, the item just made for `app_id`, into the `--items` file under its name,
/// and says so. The file is read again first, so an item an earlier one of the run kept stays.
/// When the file cannot be read or written, it says what to add to it by hand.
fn keep_item(kept: &Kept<'_>, app_id: u32, published: u64) -> ExitCode {
    let written = record_in(kept.path, app_id).and_then(|mut record| {
        let _: Option<u64> = record.items.insert(kept.name.to_owned(), published);
        kept.path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(kept.path, record.text()))
            .map_err(|error| format!("{}: {error}", kept.path.display()))
    });
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
                    "{error}. Add \"{}\": {published} to its \"items\", or the next upload makes \
                     another",
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
            if !workshop::is_item_script(script) {
                show::note(&format!(
                    "{} is not an item script. Give the path of one, or the app's ID first, then \
                     `--items`, `--name`, `--content` and `--title`. In a game's folder, leave the \
                     word out to upload the example mods",
                    script.display()
                ));
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
