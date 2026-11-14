//! `steamship builds`: an app's branches and newest builds, as Steam has them.

use std::process::ExitCode;

use steamship::builds;
use steamship::show::{self, Spinner};

use super::game::app_named;
use super::key::{offer_to_keep, print_unanswered, web_api, web_api_failed, why_unanswered};
use super::ready::home;

pub(crate) fn try_builds(app: &str, count: u32, json: bool) -> Result<ExitCode, ExitCode> {
    builds_of(app, count, json, None)
}

/// `try_builds` for `check` in an unreleased game or demo, `whose` upload sets its build live on
/// the default branch. Every game has no build live there before its first upload, so Steam's
/// answer for that is no failure.
pub(crate) fn try_builds_before_upload(
    app: &str,
    count: u32,
    whose: &str,
) -> Result<ExitCode, ExitCode> {
    builds_of(app, count, false, Some(whose))
}

fn builds_of(
    app: &str,
    count: u32,
    json: bool,
    setting_default: Option<&str>,
) -> Result<ExitCode, ExitCode> {
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
            if let Some(whose) = setting_default.filter(|_| why.is_some()) {
                spinner.skipped("no build live yet");
                show::aside(&format!(
                    "{whose} next upload sets its build live on the default branch"
                ));
                return Ok(ExitCode::SUCCESS);
            }
            let code = web_api_failed(spinner, &error);
            if let Some(why) = &why {
                show::note(why);
            }
            if json {
                print_unanswered(app_id, &error, why.as_deref());
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
