//! `steamship branch`: a branch's description set on Steam.

use std::process::ExitCode;

use steamship::cli::REFUSED;
use steamship::show::{self, Spinner};
use steamship::{branch, upload};

use super::exit::fail;
use super::game::app_named;
use super::key::{offer_to_keep, web_api, web_api_failed, why_unanswered};
use super::ready::home;

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
