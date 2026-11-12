//! `steamship assets`: store artwork checked against Valve's sizes.

use std::io;
use std::path::Path;
use std::process::ExitCode;

use steamship::assets;
use steamship::cli::REFUSED;
use steamship::show;

use super::exit::fail;

pub(crate) fn run_assets(folder: &Path) -> ExitCode {
    show::title("assets");
    show::field("folder", &folder.display().to_string());
    let store = match assets::check(folder) {
        Ok(store) => store,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return fail(
                &format!(
                    "{} does not exist. Name the folder the artwork is kept in, such as steam/store",
                    folder.display()
                ),
                REFUSED,
            );
        }
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
                "{}: holds no PNG, JPEG, icon or zip, directly or in a language folder. Name the folder the artwork is kept in",
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
