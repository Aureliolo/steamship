//! `steamship promote`: a build set live on a branch.

use std::process::ExitCode;

use steamship::branch;
use steamship::cli;
use steamship::show::{self, Spinner};

use super::game::{app_named, app_or_game};
use super::key::{offer_to_keep, web_api, web_api_failed};
use super::ready::{home, named};
use super::released::{DEFAULT_RELEASED, refused_once_released};

/// `promote` of the app named, or else the game's.
pub(crate) fn promote_in(mut request: cli::Promote) -> Result<ExitCode, ExitCode> {
    request.app = Some(app_or_game(request.app.take(), request.demo)?);
    try_promote(&request)
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
