//! `steamship rollback`: a branch set back to the build before the current one.

use std::process::ExitCode;

use steamship::cli::{self, FAILED, REFUSED};
use steamship::project;
use steamship::show::{self, Spinner};
use steamship::{branch, ledger};

use super::exit::fail;
use super::game::app_named;
use super::key::{offer_to_keep, web_api, web_api_failed};
use super::ready::{home, named};
use super::released::{DEFAULT_RELEASED, refused_once_released};

/// Sets `branch` of `app` back to the build the record shows live on it before the current one.
pub(crate) fn try_rollback(request: &cli::Rollback) -> Result<ExitCode, ExitCode> {
    show::title("rollback");
    let app_id = app_named(&request.app)?;
    let home = home()?;
    let entries = ledger::load(&home, app_id).map_err(|why| fail(&why, FAILED))?;
    let default = branch::is_default(&request.branch);
    let name = branch::shown_name(&request.branch);
    let (current, before) =
        ledger::previous(&entries, app_id, name).map_err(|why| fail(&why, REFUSED))?;
    show::field("branch", name);
    show::field(
        "from",
        &format!(
            "BuildID {}, commit {}",
            current.build_id,
            project::short(&current.commit)
        ),
    );
    show::field(
        "to",
        &format!(
            "BuildID {}, commit {}",
            before.build_id,
            project::short(&before.commit)
        ),
    );
    let (api, from) = web_api(&home)?;
    if default {
        refused_once_released(
            &home,
            app_id,
            named(request.account.as_deref()),
            DEFAULT_RELEASED,
        )?;
    }
    let key = branch::key(&request.branch);
    let spinner = Spinner::start("steam", "setting it live", false);
    if let Err(error) = api.set_live(app_id, before.build_id, key) {
        return Err(web_api_failed(spinner, &error));
    }
    spinner.done("set live");
    offer_to_keep(&home, &api, from);
    let back = ledger::Entry {
        app: app_id,
        branch: name.to_owned(),
        build_id: before.build_id,
        commit: before.commit.clone(),
        green: before.green,
        rollback: true,
    };
    if let Err(why) = ledger::record(&home, back) {
        show::note(&format!(
            "the rollback is live on Steam but was not recorded, so another rollback of {name} \
             would set the same build again: {why}"
        ));
    }
    show::success(
        &format!(
            "app {app_id}: {name} is back on BuildID {}",
            before.build_id
        ),
        "",
    );
    Ok(ExitCode::SUCCESS)
}
