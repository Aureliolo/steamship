//! `steamship achievements`: the files against Steam, and the editor's import written and checked.

use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::cli::{FAILED, REFUSED, Writing};
use steamship::project::Project;
use steamship::show::{self, Spinner};
use steamship::{achievements, builds, editor, likeness, picture, po, receipt, saved, webapi};

use super::exit::{fail, refusals};
use super::game::{app_given_or, app_named, game, game_achievements};
use super::key::{offer_to_keep, web_api, web_api_failed};
use super::ready::home;

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
pub(crate) fn achievements_in(
    app: Option<String>,
    check: Option<Vec<PathBuf>>,
    save: Option<&Path>,
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
            let receipt = game.import.join(receipt::RECEIPT);
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
    try_achievements(
        &app_given_or(app, game.as_ref()),
        &files,
        save,
        &writing,
        json,
    )
}

/// The achievements file at `path`, read, and refused when it is not one or names another app
/// than `app`: before Steam is asked anything.
pub(crate) fn achievements_file(path: &Path, app: u32) -> Result<achievements::Listed, ExitCode> {
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
    // records are kept. A mod's folder is the one holding its manifest.
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
pub(crate) fn catalogue_file(
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
/// catalogue, as one. None when there are none to check against. When `checking` them
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
    achievements::translate(&mut listed, &catalogues).map_err(|wrong| refusals(&wrong))?;
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

pub(crate) fn try_achievements(
    app: &str,
    check: &[PathBuf],
    save: Option<&Path>,
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
    if let Some(path) = save {
        written_over(path, &achievements::SAVED)?;
    }
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
                show::plain(&format!("{}\n", builds::error_json(app_id, &error, None)));
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
    if let Some(path) = save {
        return save_achievements(app_id, &held, path);
    }
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
        .map_or_default(|stats| achievements::compare_stats(stats, &held.stats, totalled.as_ref()));
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

/// Writes `held`, app `app_id`'s achievements and stats on Steam, to `path` as `--check` reads
/// them.
fn save_achievements(
    app_id: u32,
    held: &webapi::Schema,
    path: &Path,
) -> Result<ExitCode, ExitCode> {
    let text = achievements::saved(app_id, &held.achievements, &held.stats)
        .map_err(|why| fail(&format!("app {app_id}: {why}"), REFUSED))?;
    fs::write(path, text).map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    show::success(
        &format!(
            "app {app_id}: {} and {} saved to {}",
            show::counted(held.achievements.len(), "achievement"),
            show::counted(held.stats.len(), "stat"),
            path.display()
        ),
        "steamship achievements --check compares Steam's with them",
    );
    Ok(ExitCode::SUCCESS)
}

/// Why Steam can differ from an import saved in Steamworks: its Web API and image host give the
/// achievements, stats and icons as last published, where a saved import changes only the editor
/// until then.
const PUBLISHED: &str = "Steam answers with what is published. An import saved in Steamworks \
                         counts here once it is published";

/// What Steam says of whether it adds up over all players each stat it `held` that the `listed`
/// files say it does or does not. Nothing is asked when no file says of any.
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
            show::plain(&format!(
                "{}\n",
                achievements::json(app, held, &webapi::Global::default())
            ));
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
                show::plain(&format!("{}\n", builds::error_json(app, &error, None)));
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
        show::plain(&format!("{}\n", achievements::json(app, held, &global)));
    }
    Ok(ExitCode::SUCCESS)
}

/// Each icon the `listed` achievements give, read and named as the achievement editor puts it on
/// its achievement. Refused unless it is a square PNG of 64 pixels or more.
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
                    achievements::icon_word(locked),
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
fn receipt_in(out: &Path, app: u32) -> Result<receipt::Receipt, ExitCode> {
    let path = out.join(receipt::RECEIPT);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(fail(
                &format!(
                    "{}: has no {}. Import and publish the files --write wrote there, then run \
                     --imported",
                    out.display(),
                    receipt::RECEIPT
                ),
                REFUSED,
            ));
        }
        Err(error) => return Err(fail(&format!("{}: {error}", path.display()), FAILED)),
    };
    let receipt = receipt::Receipt::read(&text)
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

/// The names of the files in `out` of the kinds written for `app`, in order. None when there is
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
    let tables = editor::tables(app_id, &listed);
    let mut package: Vec<(String, Vec<u8>)> = tables
        .files()
        .map(|table| (table.name.clone(), table.text.clone().into_bytes()))
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
    let tables_before = [&tables.definitions, &tables.localisation]
        .map(|table| fs::read_to_string(out.join(&table.name)));
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
             every CSV and PNG file in {folder} and {SAVED}. Then drop \
             {} again on its own the same way, since the first import applies it only in \
             part. Publish, and run this again with --imported",
            tables.localisation.name
        ),
    );
    import_notes(&listed);
    changed_notes(&package, &tables, &tables_before, &stale_names);
    Ok(ExitCode::SUCCESS)
}

/// Names what an import of `package` changes that Steam's Web API cannot show, comparing its
/// definitions and localisation, the `tables`, with what they held `before`: each progress bar
/// that moves, and each achievement whose meaning changes while its icons, among the `stale`
/// files written afresh, do not. Definitions from before that are not UTF-8 text are said, since
/// nothing can be compared with them.
fn changed_notes(
    package: &[(String, Vec<u8>)],
    tables: &editor::Tables,
    before: &[io::Result<String>; 2],
    stale: &[&String],
) {
    if before[0]
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::InvalidData)
    {
        show::note(&format!(
            "{} was not UTF-8 text before this write, so its progress bars and wording cannot be \
             compared. Look at each achievement's bar and icons in the editor once imported, \
             before publishing",
            tables.definitions.name
        ));
    }
    let [defined_before, localised_before] = before
        .each_ref()
        .map(|table| table.as_ref().map_or("", String::as_str));
    let [defined, localised] = [&tables.definitions, &tables.localisation].map(|table| {
        package
            .iter()
            .find(|(name, _)| *name == table.name)
            .map_or_default(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
    });
    for (api_name, was, is) in editor::rebarred(defined_before, &defined) {
        show::note(&format!(
            "{api_name}'s progress bar goes from {was} to {is}, which Steam's Web API cannot \
             show. Look at it in the editor once imported, and set it there by hand if it has \
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
        if changed.is_empty() {
            continue;
        }
        let changed = show::listed(&changed, " and ");
        show::note(&format!(
            "{api_name}'s {changed} changed and its icons did not. Look at them, so that they \
             still show what it asks for"
        ));
    }
}

/// How an import is saved so that all of it is applied: a page left before Pending Changes
/// cleared had applied only some of its icons.
const SAVED: &str = "Save imported data, then stay on the page until Pending Changes is gone";

/// Records in `out` the image Steam made of each of `app`'s `icons`, once they are imported and
/// published. Refused when Steam lacks one or plainly has not taken the import yet.
fn try_achievements_imported(
    app: u32,
    wanted: &[achievements::Wanted],
    icons: &[editor::Icon],
    out: &Path,
) -> Result<ExitCode, ExitCode> {
    let path = out.join(receipt::RECEIPT);
    // Only a readable receipt of this app tells an import Steam has not taken. Any other is
    // replaced.
    let last = fs::read_to_string(&path)
        .ok()
        .and_then(|text| receipt::Receipt::read(&text).ok())
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
    let receipt = receipt::Receipt::of(app, icons, &held.achievements);
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
                    "give each of those achievements its \"icon\" in the files, and run it without \
                     --imported to write {folder}. Then import and publish what it writes, and \
                     run --imported again"
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
                 the PNG files in {} again and {SAVED}. Then publish, and run --imported again",
                out.display()
            ),
        );
        show::note(PUBLISHED);
        return Ok(ExitCode::from(REFUSED));
    }
    fs::write(&path, receipt.text())
        .map_err(|error| fail(&format!("{}: {error}", path.display()), FAILED))?;
    show::field("wrote", receipt::RECEIPT);
    show::success(
        &format!(
            "{} in {} records the {} Steam has published, each as its file",
            receipt::RECEIPT,
            out.display(),
            show::counted(icons.len(), "icon")
        ),
        "",
    );
    show::field(
        "next",
        &format!(
            "keep {} with the files. --check with --out then names each icon that changes",
            receipt::RECEIPT
        ),
    );
    Ok(ExitCode::SUCCESS)
}

/// Each of the `icons` Steam, among its achievements `held`, has not taken: its achievement or
/// itself missing, or, by the `last` receipt, still the image of an import before the file.
fn untaken_icons(
    icons: &[editor::Icon],
    held: &[webapi::Achievement],
    last: Option<&receipt::Receipt>,
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
            achievements::Drift::NoIcon {
                api_name: icon.api_name.clone(),
                which: achievements::icon_word(icon.locked),
            }
            .to_string()
        } else if recorded
            .is_some_and(|imported| imported.file_changed(icon) && imported.still_on_steam(address))
        {
            format!(
                "{}: Steam still has the {} the last import made, where the file changed since",
                icon.api_name,
                achievements::icon_word(icon.locked)
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

/// Each of `app`'s `icons` whose image on Steam, among its achievements `held`, is not its file
/// pixel by pixel, or could not be compared. Each says how.
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
        let name = receipt::steam_image(icon.on_steam(held));
        let compared = api
            .icon_image(app, name)
            .and_then(|served| likeness::compare(&icon.bytes, &served));
        match compared {
            Ok(likeness) if likeness.same() => {}
            Ok(likeness) => unlike.push(format!(
                "{}: Steam's {} is not the file's: {likeness}",
                icon.api_name,
                achievements::icon_word(icon.locked)
            )),
            Err(why) => unlike.push(format!(
                "{}: Steam's {} could not be compared with the file: {why}",
                icon.api_name,
                achievements::icon_word(icon.locked)
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

/// Says whether the `count` files `--write` makes are in `out` as it would write them, naming each
/// of them `stale` and each of those it `left` there from `before`. Answers the exit code for it.
pub(crate) fn unchanged_or_not(
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
            "no locked icon for {}. Tick Automatically generate unachieved icons, and Steam \
             greys the icon",
            show::counted(unlocked_only, "achievement")
        ));
    }
}

/// Refuses `path` to `--save` when it is there and holds more than the `lists` it writes.
pub(crate) fn written_over(path: &Path, lists: &[saved::List]) -> Result<(), ExitCode> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(fail(&format!("{}: {error}", path.display()), FAILED)),
    };
    saved::replaceable(&text, lists).map_err(|why| {
        fail(
            &format!("{} {why}. Save to another file", path.display()),
            REFUSED,
        )
    })
}
