//! `steamship text`: a game's English and catalogues as the text Steam shows.

use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use steamship::cli::{self, FAILED, REFUSED};
use steamship::project::{self, Project};
use steamship::show::{self, Spinner};
use steamship::{achievements, scripts, storefront, text};

use super::achievements::{achievements_file, catalogue_file, unchanged_or_not};
use super::exit::{fail, refusals};
use super::game::{game, game_script};

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
pub(crate) fn text_in(request: cli::Text) -> Result<ExitCode, ExitCode> {
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
    let mut english = text_english(&game, &folder)?;
    english.achievements = text_achievements(&game)?;
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
                "type the recommended ones from {} on the store page's Basic Info form. Upload the \
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
/// `shown` holds, and how many it lacks, each named. Those Steam rewords are added to `reworded`.
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

/// The template's entries for the game's achievements and sets that no record `id` keys, which
/// the game's own template does not give. None when it keeps no achievements.
fn text_achievements(game: &Project) -> Result<Vec<text::Entry>, ExitCode> {
    let paths = game
        .records(project::ACHIEVEMENTS)
        .map_err(|why| fail(&why, REFUSED))?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let app = game_script(game)?.app_id;
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        files.push((path.display().to_string(), achievements_file(path, app)?));
    }
    let listed = achievements::together(files).map_err(|why| fail(&why, REFUSED))?;
    Ok(text::achievement_entries(&listed))
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
                 Basic Info form, as the upload leaves them out. Once the page is public, \
                 steamship text --steam finds any missed",
                text::BASIC_INFO
            ),
        );
    }
    if wrote(text::SCREENSHOTS) {
        show::field(
            "next",
            &format!(
                "type each language's screenshot alt text in {} into the screenshots' Edit \
                 Accessibility Text panel on the Store Page tab, one language at a time. The \
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
