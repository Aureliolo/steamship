//! `steamship branch`: a branch's description set on Steam.

use std::process::ExitCode;

use steamship::cli::REFUSED;
use steamship::show::{self, Spinner};
use steamship::{branch, upload};

use super::exit::fail;
use super::game::{app_named, app_or_game};
use super::key::{offer_to_keep, web_api, web_api_failed, why_unanswered};
use super::ready::home;

/// `branch` of the app given, or else of the game's own.
pub(crate) fn branch_in(
    app: Option<String>,
    branch: &str,
    description: &str,
) -> Result<ExitCode, ExitCode> {
    try_branch(&app_or_game(app, false)?, branch, description)
}

pub(crate) fn try_branch(app: &str, branch: &str, description: &str) -> Result<ExitCode, ExitCode> {
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
    let Some(found) = branch::find(&branches, branch) else {
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
    let name = found.name.as_str();
    if found.description == description {
        show::success(&format!("app {app_id}: {name} is already described so"), "");
        return Ok(ExitCode::SUCCESS);
    }
    let setting = Spinner::start("steam", "setting the description", false);
    if let Err(error) = api.describe_branch(app_id, name, description) {
        return Err(web_api_failed(setting, &error));
    }
    setting.done("set");
    show::success(
        &format!("app {app_id}: {name} described as \"{description}\""),
        "",
    );
    Ok(ExitCode::SUCCESS)
}
