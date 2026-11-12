//! `steamship upload`: a build sent to Steam, set live and reported.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use steamship::account::Account;
use steamship::cli::{self, FAILED, REFUSED};
use steamship::install;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::project;
use steamship::redact::Redactor;
use steamship::show::{self, Hint, Spinner};
use steamship::{
    branch, check, ci, download, dump, green, keychain, ledger, redist, release, run, settings,
    steamcmd, upload, webapi,
};

use super::branch::try_branch;
use super::builds::try_builds;
use super::check::{
    Branch, apart, built, checked, compared, exports_apart, redistributables_shown, snapshot_sound,
};
use super::exit::{either, fail, first_failure};
use super::game::{game_as, game_script, script_named, shipped_from};
use super::key::{KeyFrom, key_at_hand, key_variable, offer_to_keep, web_api};
use super::ready::{
    account, dump_in_actions, login_refused, named, packed_login, ready, restore, steamcmd_failed,
};
use super::released::{DEFAULT_RELEASED, app_info, default_unreleased, release_judged};
use super::settings::try_settings;

/// `upload` of the script given, or of the game when there is none.
pub(crate) fn upload_in(request: &cli::Upload) -> ExitCode {
    let shipping = Shipping {
        version: request.version.as_deref(),
        preview: request.preview,
        account: named(request.account.as_deref()),
        json: request.json,
        demo: request.demo,
        green: green::Policy::from_flag(request.allow_ungreen),
    };
    let default = request.default;
    match (request.script.as_deref(), shipping.version) {
        (Some(script), Some(version)) => {
            run_upload(&Upload {
                script,
                version,
                preview: shipping.preview,
                default,
                account: shipping.account,
                json: shipping.json,
                shipped: None,
            })
            .0
        }
        (Some(script), None) => fail(
            &format!(
                "{} needs a version. Give it with --version, or run steamship upload with no \
                 script in the game's folder. That takes the newest release CHANGES.md heads, or \
                 project.godot's version",
                script.display()
            ),
            REFUSED,
        ),
        (None, _) => either(try_game_upload(&shipping)),
    }
}

/// How `upload` ships a game.
struct Shipping<'command> {
    version: Option<&'command str>,
    preview: bool,
    account: Option<&'command str>,
    json: bool,
    demo: bool,
    green: green::Policy,
}

/// `upload` in a game: the pushed commit, built, checked against Steam, uploaded, and its branch
/// described.
fn try_game_upload(shipping: &Shipping<'_>) -> Result<ExitCode, ExitCode> {
    if shipping.json {
        show::divert();
    }
    let game = game_as(shipping.demo)?;
    shipped_from(&game)?;
    // Before the build, which takes minutes, so that a commit no round has passed costs none.
    let shipped = match &game.green {
        _ if shipping.preview => None,
        Some(branch) => Some(green_commit(&game.root, branch, shipping.green)?),
        None => Some(green::Shipped {
            commit: project::git(&game.root, &["rev-parse", "HEAD"])
                .map_err(|why| fail(&why, FAILED))?,
            green: false,
        }),
    };
    let version = match shipping.version {
        Some(version) => version.to_owned(),
        None => game.version().map_err(|why| fail(&why, REFUSED))?,
    };
    upload::shippable(&version).map_err(|error| fail(&error, REFUSED))?;
    show::field("version", &version);
    let script = game_script(&game)?;
    apart(&game)?;
    exports_apart(&game, &script)?;
    snapshot_sound(&game, &script, true)?;
    built(&game)?;
    let app = script_named(&game);
    let snapshot = game.snapshot();
    let steps = [
        either(try_settings(&app, None, Some(&snapshot), shipping.account)),
        compared(&game, &app)?,
    ];
    let compared = first_failure(steps);
    if compared != ExitCode::SUCCESS {
        return Ok(compared);
    }
    let (uploaded, held_back) = run_upload(&Upload {
        script: &game.script,
        version: &version,
        preview: shipping.preview,
        // A released app's default branch is Valve's to set live with an approval steamship
        // cannot give (https://partner.steamgames.com/doc/store/application/builds), so its
        // builds go to the game's branch alone without asking Steam.
        default: !shipping.preview && !game.released,
        account: shipping.account,
        json: shipping.json,
        shipped: shipped.as_ref(),
    });
    if uploaded != ExitCode::SUCCESS || shipping.preview {
        return Ok(uploaded);
    }
    if let Some(branch) = &script.set_live {
        let description = game.description.as_deref().unwrap_or(&version);
        if held_back.is_some() {
            show::hint(Hint {
                before: &format!("once it is made, describe {branch} with "),
                command: &format!(
                    "steamship branch {} {branch} --description {description:?}",
                    script.app_id
                ),
                after: "",
            });
        } else {
            let described = either(try_branch(&app, branch, description));
            if described != ExitCode::SUCCESS {
                return Ok(described);
            }
        }
    }
    Ok(either(try_builds(&app, 10, false)))
}

/// Refuses what nothing gives the Windows files of the build `found`, which no setting in
/// Steamworks changes, and answers whether they need a redistributable, which only Steam can say
/// the app ticks.
fn needs_redistributable(found: &check::Report) -> Result<bool, ExitCode> {
    let needs = redist::judged(found, None);
    if !needs.refused.is_empty() {
        redistributables_shown(&needs, true)?;
    }
    Ok(!needs.unchecked.is_empty())
}

/// Judges what the build `found` needs against the redistributables its app ticks, as Steam
/// shows them to `account` through the steamcmd `ready` gave.
fn ticked_on_steam(
    found: &check::Report,
    (held, home, manifest): (&install::Held, &Path, &Manifest),
    account: &Account,
    packed: Option<&ci::Login>,
) -> Result<(), ExitCode> {
    let app = found.app_id.unwrap_or_default();
    let asked = app_info(
        home,
        account,
        app,
        packed,
        "asking which redistributables the app ticks",
    );
    if asked.is_err() {
        install::verify(held, manifest).map_err(|error| steamcmd_failed(&error))?;
    }
    let (console, spinner) = asked?;
    match settings::from_console(&console, app) {
        Ok(shown) => {
            spinner.done("shown");
            redistributables_shown(&redist::judged(found, Some(&shown)), false)
        }
        Err(why) => {
            spinner.failed("none shown");
            Err(fail(&why.and_again(), FAILED))
        }
    }
}

struct Upload<'command> {
    script: &'command Path,
    version: &'command str,
    preview: bool,
    default: bool,
    account: Option<&'command str>,
    json: bool,
    /// The commit a game's upload is made from, to record for a rollback, and whether its green
    /// branch held it.
    shipped: Option<&'command green::Shipped>,
}

/// The commit at `root`, refused unless origin's `branch` holds it or `policy` allows otherwise,
/// and said either way.
fn green_commit(
    root: &Path,
    branch: &str,
    policy: green::Policy,
) -> Result<green::Shipped, ExitCode> {
    let spinner = Spinner::start(
        "green",
        &format!("asking origin whether the commit is on {branch}"),
        false,
    );
    match green::require(root, branch, policy) {
        Ok(shipped) => {
            spinner.done(&if shipped.green {
                format!("{}, on origin's {branch}", project::short(&shipped.commit))
            } else {
                format!(
                    "{}, not on origin's {branch}, uploaded as --allow-ungreen asks",
                    project::short(&shipped.commit)
                )
            });
            Ok(shipped)
        }
        Err(why) => {
            spinner.failed(&format!("the commit is not on origin's {branch}"));
            Err(fail(&why, REFUSED))
        }
    }
}

/// Uploads as `upload` asks, and answers the exit code with the branch the script sets live that
/// the upload held back from, as the app cannot have it yet.
fn run_upload(upload: &Upload<'_>) -> (ExitCode, Option<String>) {
    match try_upload(upload) {
        Ok(uploaded) => uploaded,
        Err(code) => (code, None),
    }
}

/// Records, for a rollback, the build just uploaded on each branch it was set live on.
fn record_live(
    home: &Path,
    prepared: &upload::Prepared,
    (outcome, live): (&upload::Outcome, Live),
    shipped: Option<&green::Shipped>,
) {
    let (upload::Outcome::Built { build_id: built }, Some(shipped)) = (outcome, shipped) else {
        return;
    };
    let build_id = *built;
    let branches = prepared
        .set_live
        .iter()
        .filter(|_| live.branch)
        .map(String::as_str)
        .chain(live.default.then_some("default"));
    for on in branches {
        let kept = ledger::record(
            home,
            ledger::Entry {
                app: prepared.app_id,
                branch: on.to_owned(),
                build_id,
                commit: shipped.commit.clone(),
                green: shipped.green,
                rollback: false,
            },
        );
        if let Err(why) = kept {
            show::note(&format!(
                "BuildID {build_id} on {on} was not recorded, so rollback cannot return to it: {why}"
            ));
        }
    }
}

fn try_upload(request: &Upload<'_>) -> Result<(ExitCode, Option<String>), ExitCode> {
    if request.json {
        show::divert();
    }
    show::title(if request.preview {
        "upload, preview"
    } else {
        "upload"
    });
    let packed = packed_login()?;
    let account = match (&packed, request.account) {
        (Some(packed), None) => packed.account().clone(),
        _ => account(request.account, false)?.0,
    };
    let (found, branch) = checked(request.script, false)?;
    let needing = needs_redistributable(&found)?;
    let description = upload::commit(request.script)
        .and_then(|commit| upload::description(request.version, &commit))
        .map_err(|error| fail(&error, REFUSED))?;
    show::field("build", &description);
    let (held, home, manifest) = ready()?;
    restore(packed.as_ref(), &home)?;
    if needing {
        ticked_on_steam(&found, (&held, &home, &manifest), &account, packed.as_ref())?;
    }
    let mut prepared = upload::prepare(&home, request.script, &description, request.preview)
        .map_err(|error| fail(&error, FAILED))?;
    // steamcmd uploads everything before Steam refuses to commit a build to a branch the app
    // cannot have yet, so the build goes up set live nowhere by steamcmd.
    if branch == Branch::Unbranched && !request.preview {
        prepared
            .hold_back_set_live()
            .map_err(|error| fail(&error, FAILED))?;
    }
    let (default, in_run) = default_key(
        request.default,
        (&held, &home, &manifest),
        (&account, prepared.app_id, packed.as_ref()),
    )?;
    let saved = prepared.output.join("steamcmd.log");
    let steam = Steam {
        account: &account,
        prepared: &prepared,
        home: &home,
        saved: &saved,
        preview: request.preview,
        info_of: in_run.then_some(prepared.app_id),
    };
    let (doing, again) = if request.preview {
        ("computing the build", "computing the build again")
    } else {
        ("uploading", "uploading again")
    };
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let Ran {
        outcome,
        spinner,
        took,
        redactor,
        info,
        ..
    } = build_retrying(&steam, doing, again)?;
    let (default, spinner) = shown_unreleased(
        default,
        &outcome,
        info.as_deref(),
        (prepared.app_id, spinner, doing),
    );
    let outcome = found_on_steam(outcome, &prepared, &description);
    let reporting = Reporting {
        spinner,
        took: &took,
        prepared: &prepared,
        saved: &saved,
        packed: packed.as_ref(),
        default: default.as_ref().map(|(api, _)| api),
    };
    let (code, live) = report(&outcome, reporting);
    record_live(&home, &prepared, (&outcome, live), request.shipped);
    let setting_default = default.is_some();
    if let Some((api, from)) = default {
        offer_to_keep(&home, &api, from);
    }
    if request.json {
        show::plain(&format!(
            "{}\n",
            summary(&outcome, live, setting_default, &prepared, &description)
        ));
    }
    if matches!(
        outcome,
        upload::Outcome::Failed(_) | upload::Outcome::BuiltThenFailed { .. }
    ) {
        dump_failed(saved, &prepared, &marks.added(&steamcmd_logs), &redactor);
    }
    install::verify(&held, &manifest).map_err(|error| steamcmd_failed(&error))?;
    Ok((code, prepared.held_back))
}

/// The key an upload `asked` for the default branch too sets it live with, and whether the build's
/// own steamcmd run is to show first that `app` is unreleased.
///
/// Asked for on every upload by a game's tools, the default branch is left once the game is
/// released, and the upload goes on to its own branch as ever. With a key at hand Steam is asked
/// in the build's run, which saves a login. Without one it is asked here, so a key that is needed
/// is asked for before anything goes up.
fn default_key(
    asked: bool,
    (held, home, manifest): (&install::Held, &Path, &Manifest),
    (account, app, packed): (&Account, u32, Option<&ci::Login>),
) -> Result<(Option<(webapi::Api, KeyFrom)>, bool), ExitCode> {
    if !asked {
        return Ok((None, false));
    }
    if key_variable().is_some() || keychain::has(home).unwrap_or(false) {
        return Ok((Some(web_api(home)?), true));
    }
    let shown = default_unreleased(home, account, app, packed);
    if shown.is_err() {
        install::verify(held, manifest).map_err(|error| steamcmd_failed(&error))?;
    }
    Ok(match shown? {
        Ok(()) => (Some(web_api(home)?), false),
        Err(why) => {
            show::note(&format!(
                "{why}. The build goes up without being set live on default"
            ));
            (None, false)
        }
    })
}

/// The `default` key to set the default branch live with, kept, when the build's own steamcmd run
/// printed the app's `info`, only while that shows `app` unreleased, said while the build's
/// `spinner`, `doing` so, waits. Nothing is said for a build that never reached Steam.
fn shown_unreleased<Key>(
    default: Option<Key>,
    outcome: &upload::Outcome,
    info: Option<&str>,
    (app, spinner, doing): (u32, Spinner, &str),
) -> (Option<Key>, Spinner) {
    let Some(info) = info else {
        return (default, spinner);
    };
    let Some(key) = default else {
        return (None, spinner);
    };
    let (upload::Outcome::Built { build_id } | upload::Outcome::BuiltThenFailed { build_id, .. }) =
        outcome
    else {
        return (None, spinner);
    };
    let (judged, said) = match release::info_from_console(info, app, "app info") {
        Ok(shown) => {
            let (judged, said) = release_judged(release::state(&shown), app, DEFAULT_RELEASED);
            (
                judged.map_err(|why| {
                    format!("{why}. The build goes up without being set live on default")
                }),
                said,
            )
        }
        Err(why) => (
            Err(format!(
                "{why}. The build goes up without being set live on default. While the app is \
                 unreleased, steamship promote {app} --build {build_id} --branch default does that"
            )),
            "not shown".to_owned(),
        ),
    };
    let spinner = spinner.around(
        || match &judged {
            Ok(()) => show::done("steam", &said),
            Err(why) => {
                show::failed("steam", &said);
                show::note(why);
            }
        },
        doing,
        true,
    );
    (judged.is_ok().then_some(key), spinner)
}

/// The logs of a failed upload, steamcmd's console saved at `saved` among them, as
/// [`dump_in_actions`] shows them with what steamcmd `added` to its own.
fn dump_failed(
    saved: PathBuf,
    prepared: &upload::Prepared,
    added: &[(String, Vec<u8>)],
    redactor: &Redactor,
) {
    let mut files = vec![
        ("steamcmd's console".to_owned(), saved),
        (format!("app_build_{}.log", prepared.app_id), prepared.log()),
    ];
    files.extend(depot_logs(&prepared.output));
    dump_in_actions(&files, &[], added, redactor);
}

/// What every run of steamcmd for one upload is given.
struct Steam<'upload> {
    account: &'upload Account,
    prepared: &'upload upload::Prepared,
    home: &'upload Path,
    /// Where steamcmd's console is written, redacted.
    saved: &'upload Path,
    preview: bool,
    /// The app whose info the first run prints before it builds.
    info_of: Option<u32>,
}

/// What one run of steamcmd for an upload came to, its spinner still turning.
struct Ran {
    outcome: upload::Outcome,
    /// Why the build failed, if running steamcmd again may pass.
    retry: Option<upload::Retry>,
    spinner: Spinner,
    took: String,
    redactor: Redactor,
    /// What steamcmd printed of the app's info, when it was asked to.
    info: Option<String>,
}

/// Runs steamcmd's build, saying it is `doing` so, and runs it `again` while it fails in a way
/// that may pass: once Steam's CDN serves a manifest it did not, and once more for no connection
/// to Steam's build server.
fn build_retrying(steam: &Steam<'_>, doing: &str, again: &str) -> Result<Ran, ExitCode> {
    let mut ran = build(steam, doing, steam.info_of)?;
    let info = ran.info.take();
    let mut tries = upload::Tries::default();
    while let Some(retry) = ran.retry.take() {
        let next = tries.next(&retry);
        match retry {
            upload::Retry::Unserved(unserved) => {
                let depot = unserved.depot;
                let upload::Next::Wait(patience) = next else {
                    upload::still_unserved(&mut ran.outcome, depot);
                    break;
                };
                ran.spinner.failed(&format!(
                    "Steam's CDN does not serve depot {depot}'s last manifest yet"
                ));
                let waiting = Spinner::start(
                    "cdn",
                    &format!("waiting for depot {depot}'s last manifest"),
                    true,
                );
                let served = download::served(&unserved.url, patience);
                tries.spent(waiting.elapsed());
                let waited = show::took(waiting.elapsed());
                match served {
                    download::Served::Yes => waiting.done(&format!("served after {waited}")),
                    // The CDN answers 401 for a request code it does not take, as one that has
                    // expired, and steamcmd's next run is given a new one.
                    download::Served::Otherwise(status) => {
                        waiting.done(&format!(
                            "answered {status} after {waited} and is asked again"
                        ));
                    }
                    download::Served::No => {
                        ran.spinner = waiting;
                        ran.took = waited;
                        upload::still_unserved(&mut ran.outcome, depot);
                        break;
                    }
                }
            }
            lost @ (upload::Retry::NoConnection
            | upload::Retry::Disconnected
            | upload::Retry::NoClientConfig) => {
                if next == upload::Next::GiveUp {
                    break;
                }
                let what = match lost {
                    upload::Retry::Disconnected => "Steam disconnected steamcmd",
                    upload::Retry::NoClientConfig => "Steam sent steamcmd no client config",
                    upload::Retry::NoConnection | upload::Retry::Unserved(_) => {
                        "no connection to Steam's build server"
                    }
                };
                ran.spinner.failed(&format!("{what} after {}", ran.took));
            }
        }
        ran = build(steam, again, None)?;
    }
    ran.info = info;
    Ok(ran)
}

/// Runs steamcmd's build once, printing first the info of the app `info_of` names, saying it is
/// `doing` so, and reads what it came to.
fn build(steam: &Steam<'_>, doing: &str, info_of: Option<u32>) -> Result<Ran, ExitCode> {
    upload::clear_log(steam.prepared).map_err(|error| fail(&error, FAILED))?;
    let before = Redactor::for_home(steam.home).map_err(|error| fail(&error, FAILED))?;
    let spinner = Spinner::start("steam", doing, true);
    let finished = steamcmd::upload(steam.account, &steam.prepared.script, info_of).run(steam.home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(fail(&error, FAILED));
        }
    };
    let redactor =
        before.and(Redactor::for_home(steam.home).map_err(|error| fail(&error, FAILED))?);
    let console = redactor.redact(&finished.output);
    fs::write(steam.saved, &console)
        .map_err(|error| fail(&format!("{}: {error}", steam.saved.display()), FAILED))?;
    let log = fs::read(steam.prepared.log())
        .ok()
        .map(|log| redactor.redact(&log));
    let mut console = String::from_utf8_lossy(&console).into_owned();
    // The info is read apart, as text anyone can write into an app's settings could pass for a
    // line of the build.
    let info = info_of.map(|app| {
        release::info_at(&console, app)
            .map(|at| console.drain(at).collect::<String>())
            .unwrap_or_default()
    });
    let log = log.as_deref().map(String::from_utf8_lossy);
    let outcome = upload::judge(
        steam.prepared.app_id,
        finished.code,
        &console,
        log.as_deref(),
        steam.preview,
    );
    let retry = if matches!(outcome, upload::Outcome::Failed(_)) && run::told().is_none() {
        upload::retry(&console, log.as_deref())
    } else {
        None
    };
    Ok(Ran {
        outcome,
        retry,
        spinner,
        took,
        redactor,
        info,
    })
}

/// Asks Steam, when a key is at hand, whether `build_id` is what is live on `branch` now. False
/// only when Steam shows another build there, or no such branch. Asked a few times, since a
/// build just set live may not show at once. A Steam that cannot be asked holds nothing up.
fn confirmed_live(app: u32, branch: &str, build_id: u64) -> bool {
    const APART: Duration = Duration::from_secs(2);
    let Ok(Some(api)) = key_at_hand() else {
        return true;
    };
    let spinner = Spinner::start("branch", "asking Steam what is live", false);
    let seen = upload::live_asked(APART, || {
        api.branches(app)
            .map(|branches| upload::live_now(&branches, branch, build_id))
    });
    match seen {
        Ok(upload::Live::Confirmed) => {
            spinner.done(&format!("{branch}: BuildID {build_id}, as Steam shows it"));
            true
        }
        Ok(upload::Live::Other(other)) => {
            spinner.failed(&format!("{branch}: BuildID {other}, as Steam shows it"));
            false
        }
        Ok(upload::Live::Missing) => {
            spinner.failed(&format!("{branch}: no such branch on Steam"));
            false
        }
        Err(error) => {
            spinner.failed("not confirmed");
            show::aside(&error.to_string());
            true
        }
    }
}

/// A build Steam kept but set live nowhere, found by its description when a key is at hand, as
/// the build it is: steamcmd reports no build ID when Steam refuses to set one live.
fn found_on_steam(
    outcome: upload::Outcome,
    prepared: &upload::Prepared,
    description: &str,
) -> upload::Outcome {
    let upload::Outcome::Failed(reasons) = &outcome else {
        return outcome;
    };
    if upload::commit_refused(reasons, prepared.set_live.as_deref()).is_none() {
        return outcome;
    }
    let Ok(Some(api)) = key_at_hand() else {
        return outcome;
    };
    // Enough to reach past uploads made while this one ran.
    let found = api
        .builds(prepared.app_id, 10)
        .ok()
        .and_then(|builds| upload::uploaded_as(&builds, description));
    let Some(build_id) = found else {
        return outcome;
    };
    upload::Outcome::BuiltThenFailed {
        build_id,
        reasons: reasons.clone(),
    }
}

/// What an upload's ending is said with: the spinner still turning, how long steamcmd took,
/// where its console was saved, the login handed over if any, the Web API to set the build live
/// on the default branch with, when asked to.
struct Reporting<'upload> {
    spinner: Spinner,
    took: &'upload str,
    prepared: &'upload upload::Prepared,
    saved: &'upload Path,
    packed: Option<&'upload ci::Login>,
    default: Option<&'upload webapi::Api>,
}

/// Where an upload's build ended up live.
#[derive(Clone, Copy, Default)]
struct Live {
    /// On the branch the script sets live.
    branch: bool,
    /// On the default branch.
    default: bool,
}

/// Says what `outcome` came to, and answers its exit code and where the build is live.
fn report(outcome: &upload::Outcome, reporting: Reporting<'_>) -> (ExitCode, Live) {
    let Reporting {
        spinner,
        took,
        prepared,
        saved,
        packed,
        default,
    } = reporting;
    let app = prepared.app_id;
    let logs = format!(
        "log  {}, and steamcmd's own in {}",
        saved.display(),
        prepared.log().display()
    );
    let code = match outcome {
        upload::Outcome::Built { build_id } => {
            spinner.done(&format!("uploaded in {took}"));
            hand_on(*build_id);
            if let Some(branch) = &prepared.held_back {
                return first_on_default(app, *build_id, branch, default, &logs);
            }
            return set_live(app, *build_id, prepared.set_live.as_deref(), default, &logs);
        }
        upload::Outcome::Previewed => {
            spinner.done(&format!("preview finished in {took}"));
            show::success("nothing was uploaded, as asked", &logs);
            ExitCode::SUCCESS
        }
        upload::Outcome::NotLoggedIn(line) => {
            spinner.failed(&format!("refused after {took}"));
            login_refused(line, packed)
        }
        upload::Outcome::BuiltThenFailed { build_id, reasons } => {
            spinner.failed(&format!("failed after {took}"));
            show::failure(
                &format!("app {app}: built as BuildID {build_id}, then steamcmd failed"),
                "",
                None,
            );
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(&why);
            }
            let branches = prepared
                .set_live
                .iter()
                .map(String::as_str)
                .chain(default.map(|_| "default"));
            for branch in branches {
                show::hint(Hint {
                    before: "set it live with ",
                    command: &format!(
                        "steamship promote {app} --build {build_id} --branch {branch}"
                    ),
                    after: "",
                });
            }
            show::note(&logs);
            hand_on(*build_id);
            ExitCode::from(FAILED)
        }
        upload::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            for reason in reasons {
                show::failure(reason, "", None);
            }
            if let Some(why) = upload::commit_refused(reasons, prepared.set_live.as_deref()) {
                show::note(upload::NOT_LIVE);
                show::note(&why);
                show::hint(Hint {
                    before: "see the app's branches with ",
                    command: &format!("steamship builds {app}"),
                    after: "",
                });
            }
            show::note(&logs);
            ExitCode::from(FAILED)
        }
    };
    (code, Live::default())
}

/// Says where `build_id`, just uploaded for `app`, is live: on `branch`, which the script set it
/// live on, once Steam shows it there, and then on default, set through `default` when given.
/// Answers the upload's exit code and where the build is live. `logs` says where to look.
fn set_live(
    app: u32,
    build_id: u64,
    branch: Option<&str>,
    default: Option<&webapi::Api>,
    logs: &str,
) -> (ExitCode, Live) {
    if let Some(branch) = branch
        && !confirmed_live(app, branch, build_id)
    {
        show::failure(
            &format!(
                "app {app}: BuildID {build_id} uploaded, but Steam does not show it live on \
                 {branch}"
            ),
            "",
            Some(Hint {
                before: "set it live with ",
                command: &format!("steamship promote {app} --build {build_id} --branch {branch}"),
                after: "",
            }),
        );
        show::note(logs);
        return (ExitCode::from(FAILED), Live::default());
    }
    let mut live = Live {
        branch: branch.is_some(),
        default: false,
    };
    if let Some(api) = default {
        if !live_on_default(api, app, build_id, branch) {
            show::note(logs);
            return (ExitCode::from(FAILED), live);
        }
        live.default = true;
    }
    let set = match (branch, live.default) {
        (Some(branch), true) => format!(", set live on {branch} and default"),
        (Some(branch), false) => format!(", set live on {branch}"),
        (None, true) => ", set live on default".to_owned(),
        (None, false) => String::new(),
    };
    show::success(&format!("app {app}: BuildID {build_id}{set}"), logs);
    (ExitCode::SUCCESS, live)
}

/// Says where `build_id`, just uploaded for `app`, is live while the app has no branch but
/// default, so that `branch`, which the script sets live, cannot be made yet: on default, set
/// through `default` when given. Answers as [`set_live`] does.
fn first_on_default(
    app: u32,
    build_id: u64,
    branch: &str,
    default: Option<&webapi::Api>,
    logs: &str,
) -> (ExitCode, Live) {
    let make = format!(
        "make {branch} in Steamworks under SteamPipe, Builds, and set the build live there with "
    );
    let promote = format!("steamship promote {app} --build {build_id} --branch {branch}");
    let Some(api) = default else {
        show::failure(
            &format!("app {app}: BuildID {build_id} uploaded, but set live nowhere"),
            &format!(
                "app {app} has no branch but default until a build is live there, so steamcmd \
                 was not asked to set it live on {branch}"
            ),
            Some(Hint {
                before: "set it live on default with ",
                command: &format!("steamship promote {app} --build {build_id} --branch default"),
                after: "",
            }),
        );
        show::hint(Hint {
            before: &make,
            command: &promote,
            after: "",
        });
        show::note(logs);
        return (ExitCode::from(FAILED), Live::default());
    };
    let (code, live) = set_live(app, build_id, None, Some(api), logs);
    if live.default {
        show::note(&format!(
            "not set live on {branch}: Steam gives app {app} no branch but default until a build \
             is live there"
        ));
        show::hint(Hint {
            before: &make,
            command: &promote,
            after: "",
        });
    }
    (code, live)
}

/// Sets `build_id` of `app` live on the default branch through `api`, and says whether Steam
/// did. The build is already live on `branch`, if the script sets one live.
fn live_on_default(api: &webapi::Api, app: u32, build_id: u64, branch: Option<&str>) -> bool {
    let spinner = Spinner::start("default", "setting it live", false);
    let Err(error) = api.set_live(app, build_id, branch::DEFAULT_KEY) else {
        spinner.done("set live");
        return true;
    };
    spinner.failed("not set live");
    let also = branch.map_or_else(String::new, |branch| format!(" and live on {branch}"));
    show::failure(
        &format!(
            "app {app}: BuildID {build_id} uploaded{also}, but Steam did not set it live on \
             default"
        ),
        &error.to_string(),
        Some(Hint {
            before: "set it live with ",
            command: &format!("steamship promote {app} --build {build_id} --branch default"),
            after: "",
        }),
    );
    false
}

/// What an upload came to, for the program that ran steamship with `--json`: the outcome, the
/// app, the build Steam made of it if any, the branch the script sets live and whether the build
/// is live there and on default, and the build's description. `default` is whether it was to go
/// live on default too: asked to, while the app is unreleased.
fn summary(
    outcome: &upload::Outcome,
    live: Live,
    default: bool,
    prepared: &upload::Prepared,
    description: &str,
) -> serde_json::Value {
    let branch = prepared.set_live.as_ref().or(prepared.held_back.as_ref());
    let (said, build_id) = match outcome {
        upload::Outcome::Built { build_id } => {
            let said = if (branch.is_some() && !live.branch) || (default && !live.default) {
                "not-live"
            } else {
                "uploaded"
            };
            (said, Some(*build_id))
        }
        upload::Outcome::Previewed => ("previewed", None),
        upload::Outcome::NotLoggedIn(_) => ("not-logged-in", None),
        upload::Outcome::BuiltThenFailed { build_id, .. } => ("built-then-failed", Some(*build_id)),
        upload::Outcome::Failed(_) => ("failed", None),
    };
    serde_json::json!({
        "outcome": said,
        "app": prepared.app_id,
        "build_id": build_id,
        "branch": branch,
        "live": live.branch,
        "default": live.default,
        "description": description,
    })
}

/// Each depot's build log in `output`, by name, in name order.
fn depot_logs(output: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(output)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let log = Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log"));
            (name.starts_with("depot_build_") && log).then(|| (name, entry.path()))
        })
        .collect();
    found.sort();
    found
}

/// Hands `build_id` on as the `build-id` output of the GitHub Actions step steamship runs in.
fn hand_on(build_id: u64) {
    output("build-id", build_id, "the BuildID");
}

/// Sets the output `name` of the GitHub Actions step steamship runs in to `value`, through the
/// file GitHub names for a step's outputs. The upload is done by then, so a file that cannot be
/// written is said, as `what`, and does not fail it.
pub(crate) fn output(name: &str, value: u64, what: &str) {
    let Some(outputs) = env::var_os("GITHUB_OUTPUT").filter(|path| !path.is_empty()) else {
        return;
    };
    let written = fs::OpenOptions::new()
        .append(true)
        .open(&outputs)
        .and_then(|mut file| {
            io::Write::write_all(&mut file, format!("{name}={value}\n").as_bytes())
        });
    if let Err(error) = written {
        show::note(&format!(
            "{what} could not be handed to the workflow: {error}"
        ));
    }
}
