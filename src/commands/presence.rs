//! `steamship rich-presence`: each language's file checked and sent to Steam.

use std::fmt::Display;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use steamship::cli::REFUSED;
use steamship::presence;
use steamship::show::{self, Spinner};

use super::exit::fail;
use super::game::{app_given_or, app_named, game_as, shipped_from};
use super::key::{offer_to_keep, web_api, web_api_failed};
use super::ready::home;

/// `rich-presence` with whatever is not given taken from the game: every language's file, sent
/// only from a pushed tree.
pub(crate) fn rich_presence_in(
    app: Option<String>,
    demo: bool,
    mut files: Vec<PathBuf>,
    dry_run: bool,
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
        if !dry_run {
            shipped_from(game)?;
        }
    }
    try_rich_presence(&app_given_or(app, game.as_ref()), &files, dry_run)
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
pub(crate) fn presence_read(files: &[PathBuf]) -> Result<Vec<presence::Language>, String> {
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

pub(crate) fn try_rich_presence(
    app: &str,
    files: &[PathBuf],
    dry_run: bool,
) -> Result<ExitCode, ExitCode> {
    show::title(if dry_run {
        "rich-presence, dry run"
    } else {
        "rich-presence"
    });
    let app_id = app_named(app)?;
    let languages = presence_files(files)?;
    if dry_run {
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
