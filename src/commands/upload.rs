//! `steamship upload`: a build sent to Steam, set live and reported.

use std::env;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use steamship::account::Account;
use steamship::cli::{self, FAILED, LIMITED, REFUSED, UNFINISHED};
use steamship::install;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::project;
use steamship::redact::{self, Redactor};
use steamship::show::{self, Hint, Spinner};
use steamship::{
    branch, builds, check, ci, download, dump, green, keychain, ledger, redist, release, run,
    settings, steamcmd, upload, webapi,
};

use super::branch::try_branch;
use super::builds::try_builds;
use super::check::{
    Branch, apart, built, checked, compared, exports_apart, redistributables_shown, snapshot_sound,
};
use super::exit::{either, fail, first_failure, late};
use super::game::{game_as, game_script, script_given, script_named, shipped_from};
use super::key::{KeyFrom, key_at_hand, key_variable, offer_to_keep, web_api};
use super::ready::{
    dump_in_actions, login_refusal, named, ready, restore, run_as, steamcmd_failed,
};
use super::released::{DEFAULT_RELEASED, app_info, default_unreleased, release_judged};
use super::settings::try_settings;

/// `upload` of the script given, or of the game when there is none.
pub(crate) fn upload_in(request: &cli::Upload) -> ExitCode {
    let shipping = Shipping {
        version: request.version.as_deref(),
        preview: request.dry_run,
        account: named(request.account.as_deref()),
        json: request.json,
        demo: request.demo,
        green: green::Policy::from_flag(request.allow_ungreen),
    };
    let default = request.default;
    let script = match request.app.as_deref() {
        Some(app) => match script_given(Some(app), false) {
            Ok(script) => Some(script),
            Err(code) => return code,
        },
        None => None,
    };
    match (script.as_deref(), shipping.version) {
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
    // The build is on Steam. A later failure must not fail the run, or a second run would upload
    // it again.
    if let Some(branch) = &script.set_live {
        show::failures_as_notes(&format!("could not describe {branch}: "));
        let description = game.description.as_deref().unwrap_or(&version);
        let describe_with = |before: &str| {
            show::hint(Hint {
                before,
                command: &format!(
                    "steamship branch {} {branch} --description {description:?}",
                    script.app_id
                ),
                after: "",
            });
        };
        match held_back {
            Some(_) => describe_with(&format!("once it is made, describe {branch} with ")),
            None if try_branch(&app, branch, description) != Ok(ExitCode::SUCCESS) => {
                describe_with(&format!("describe {branch} with "));
                return Ok(ExitCode::SUCCESS);
            }
            None => {}
        }
    }
    show::failures_as_notes(&format!(
        "could not list the builds of app {}: ",
        script.app_id
    ));
    let _listed = try_builds(&app, 10, false);
    Ok(ExitCode::SUCCESS)
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
        hide_in_logs(home, account, app);
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
    let (title, doing, again) = wording(request.preview);
    show::title(title);
    let (packed, account) = run_as(request.account)?;
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
    )
    .inspect_err(|_| hide_in_logs(&home, &account, prepared.app_id))?;
    let saved = prepared.output.join("steamcmd.log");
    let steam = Steam {
        account: &account,
        prepared: &prepared,
        home: &home,
        saved: &saved,
        preview: request.preview,
        info_of: in_run.then_some(prepared.app_id),
    };
    let steamcmd_logs = steamcmd::state_folder(&home, Platform::THIS).join("logs");
    let marks = dump::Marks::take(&steamcmd_logs);
    let Ran {
        outcome,
        spinner,
        took,
        redactor,
        before,
        info,
        log,
        written,
        ..
    } = match build_retrying(&steam, (doing, again)) {
        Ok(ran) => ran,
        Err(halt) => {
            if request.json {
                print_halted(&halt, &prepared, &description);
            }
            return Err(halt.code);
        }
    };
    let (default, spinner) = shown_unreleased(
        default,
        &outcome,
        info.as_deref().map(|info| (info, redactor.is_some())),
        (prepared.app_id, spinner, doing),
    );
    let outcome = found_on_steam(outcome, &prepared, &description);
    let reporting = Reporting {
        spinner,
        took: &took,
        prepared: &prepared,
        log: &log,
        shown: redactor.is_some(),
        redactor: redactor.as_ref().unwrap_or(&before),
        answers: redactor.as_ref(),
        packed: packed.as_ref(),
        default: default.as_ref().map(|(api, _)| api),
    };
    let reported = report(&outcome, reporting);
    record_live(&home, &prepared, (&outcome, reported.live), request.shipped);
    if let Some((api, from)) = default {
        offer_to_keep(&home, &api, from);
    }
    if matches!(
        outcome,
        upload::Outcome::Failed(_) | upload::Outcome::BuiltThenFailed { .. }
    ) && let Some(redactor) = &redactor
    {
        let added = marks.added(&steamcmd_logs);
        dump_failed(written.then_some(saved), &prepared, &added, redactor);
    }
    let after = checked_after(&held, &manifest, &outcome, &prepared);
    if request.json {
        let shown = (&reported, redactor.as_ref().unwrap_or(&before));
        let ended = (&outcome, (reported.code, &after));
        print_ended(ended, shown, (&prepared, &description));
    }
    after.map_err(|(stopped, _)| stopped)?;
    Ok((reported.code, prepared.held_back))
}

/// What the check of steamcmd's files after a run found: nothing, or the exit code with the fault's
/// text.
type Checked = Result<(), (ExitCode, String)>;

/// Prints the line of an upload whose retry stopped, when steamcmd has run before it.
fn print_halted(halt: &Halt, prepared: &upload::Prepared, description: &str) {
    if let Some(earlier) = &halt.earlier {
        let error = error_of(halt.code, "", Some(&halt.failure), None);
        let line = summary(earlier, Live::default(), prepared, description, error);
        print_line(&line);
    }
}

/// Prints an upload's line once the check after its run is made. `ended` holds the outcome, the
/// run's exit code and what that check found. `reported` holds what the run said. `shown` hides the
/// secrets in the fault.
fn print_ended(
    ended: (&upload::Outcome, (ExitCode, &Checked)),
    (reported, shown): (&Reported, &Redactor),
    (prepared, description): (&upload::Prepared, &str),
) {
    let (outcome, (code, after)) = ended;
    let (exit, later) = match after {
        Ok(()) => (code, None),
        Err((stopped, why)) => (*stopped, Some(redacted(shown, why))),
    };
    let holds = format!(
        "Steam holds a build of app {0}, so a second upload would make another. List it with \
         steamship builds {0}",
        prepared.app_id
    );
    let error = error_of(exit, &holds, reported.failure.as_ref(), later.as_deref());
    print_line(&summary(
        outcome,
        reported.live,
        prepared,
        description,
        error,
    ));
}

/// Prints `line`, an upload's or a Workshop run's JSON, on its own line.
pub(crate) fn print_line(line: &serde_json::Value) {
    show::plain(&format!("{line}\n"));
}

/// The first failure of a run, for the `error` of its `--json` line.
pub(crate) struct Failure {
    /// The word for the failure. Under exit 5 `error_of` reports `unfinished` instead.
    code: &'static str,
    /// What a person reads, redacted.
    message: String,
}

impl Failure {
    /// The failure the word `code` names, said as `message`.
    pub(crate) const fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    /// The failure the word `code` names, said as the `reasons` steamcmd gave that may be shown.
    /// When none may be, the sentence of the `log` line stands in.
    pub(crate) fn of_reasons(code: &'static str, reasons: &[String], log: &str) -> Self {
        let message = if reasons.is_empty() {
            log.strip_prefix("log  ").unwrap_or(log).to_owned()
        } else {
            reasons.join("\n")
        };
        Self { code, message }
    }
}

/// The failure of a login Steam refused, which [`login_refusal`] answered `code` and `message` for.
pub(crate) fn login_failure(code: ExitCode, message: String) -> Failure {
    let word = if code == ExitCode::from(LIMITED) {
        "login-limited"
    } else {
        "not-logged-in"
    };
    Failure::new(word, message)
}

/// `headline` and `reason` as `show::failure` prints them, without its mark.
pub(crate) fn said(headline: &str, reason: &str) -> String {
    if reason.is_empty() {
        headline.to_owned()
    } else {
        format!("{headline}: {reason}")
    }
}

/// Prints a failure of steamship's own that stops steamcmd from running, and answers the exit code
/// and the failure. `redactor` hides the secrets in its text, when one could be built.
pub(crate) fn not_run(error: &dyn Display, redactor: Option<&Redactor>) -> (ExitCode, Failure) {
    let code = fail(error, FAILED);
    let message = error.to_string();
    let message = redactor.map_or_else(|| message.clone(), |redactor| redacted(redactor, &message));
    (
        code,
        Failure {
            code: "steamcmd-not-run",
            message,
        },
    )
}

/// The `error` a run's `--json` line holds, or none when the run exited 0.
///
/// Exit 5 is `unfinished`: its message opens with `held`, what Steam holds and what to do, then
/// the `first` failure and the `later` steamcmd fault. Any other exit gives the word and message
/// of the `first` failure, or `steamcmd-install` with the `later` fault when nothing failed before.
pub(crate) fn error_of(
    exit: ExitCode,
    held: &str,
    first: Option<&Failure>,
    later: Option<&str>,
) -> Option<serde_json::Value> {
    if exit == ExitCode::SUCCESS {
        return None;
    }
    if exit == ExitCode::from(UNFINISHED) {
        let message = [Some(held), first.map(|first| first.message.as_str()), later]
            .into_iter()
            .flatten()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        return Some(builds::failure_json("unfinished", &message));
    }
    Some(match (first, later) {
        (Some(first), _) => builds::failure_json(first.code, &first.message),
        (None, later) => builds::failure_json("steamcmd-install", later.unwrap_or_default()),
    })
}

/// The title of the run, and what the spinner says while a run of steamcmd goes on and when it is
/// run again.
const fn wording(preview: bool) -> (&'static str, &'static str, &'static str) {
    if preview {
        (
            "upload, dry run",
            "computing the build",
            "computing the build again",
        )
    } else {
        ("upload", "uploading", "uploading again")
    }
}

/// Checks steamcmd's files again after the run. A failed check exits 5 when Steam holds a build
/// that a second upload would duplicate, and says so. It answers the exit code and the fault's
/// text.
fn checked_after(
    held: &install::Held,
    manifest: &Manifest,
    outcome: &upload::Outcome,
    prepared: &upload::Prepared,
) -> Checked {
    let Err(error) = install::verify(held, manifest) else {
        return Ok(());
    };
    let kept = outcome.kept_on_steam(prepared.set_live.as_deref());
    let failed = late(steamcmd_failed(&error), kept);
    if kept {
        let app = prepared.app_id;
        show::note(&format!(
            "Steam holds a build of app {app}, so a second upload would make another"
        ));
        show::hint(Hint {
            before: "list it with ",
            command: &format!("steamship builds {app}"),
            after: "",
        });
    }
    Err((failed, error.to_string()))
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
/// `spinner`, `doing` so, waits. Nothing is said for a build that never reached Steam. `info` holds
/// whether a line of it may be shown, and none of it is when it may not.
fn shown_unreleased<Key>(
    default: Option<Key>,
    outcome: &upload::Outcome,
    info: Option<(&str, bool)>,
    (app, spinner, doing): (u32, Spinner, &str),
) -> (Option<Key>, Spinner) {
    let Some((info, may_show)) = info else {
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
                "{}. The build goes up without being set live on default. While the app is \
                 unreleased, steamship promote {app} --build {build_id} --branch default does that",
                if may_show {
                    why.to_string()
                } else {
                    "steamcmd's app info could not be read".to_owned()
                }
            )),
            "not shown".to_owned(),
        ),
    };
    // A line of steamcmd's could hold a secret that nothing could be built to hide.
    let said = if may_show {
        said
    } else {
        said.split_once(" (")
            .map_or(said.as_str(), |(head, _)| head)
            .to_owned()
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

/// The logs of a failed upload, steamcmd's console among them when it was saved at `saved`, as
/// [`dump_in_actions`] shows them with what steamcmd `added` to its own.
fn dump_failed(
    saved: Option<PathBuf>,
    prepared: &upload::Prepared,
    added: &[(String, Vec<u8>)],
    redactor: &Redactor,
) {
    let mut files: Vec<(String, PathBuf)> = saved
        .map(|saved| ("steamcmd's console".to_owned(), saved))
        .into_iter()
        .collect();
    files.push((format!("app_build_{}.log", prepared.app_id), prepared.log()));
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
    /// The redactor built again after the run. None when that failed, and then none of steamcmd's
    /// output may be shown.
    redactor: Option<Redactor>,
    /// The redactor from before the run: all that a refused login's text can hold.
    before: Redactor,
    /// What steamcmd printed of the app's info, when it was asked to.
    info: Option<String>,
    /// Where the run's console and steamcmd's logs are, or which could not be kept.
    log: String,
    /// Whether this run's console was saved.
    written: bool,
}

/// Why an upload's runs of steamcmd stopped short of an outcome.
struct Halt {
    code: ExitCode,
    failure: Failure,
    /// What the run before came to, when steamcmd had run: the upload's line is due then.
    earlier: Option<upload::Outcome>,
}

/// Runs steamcmd's build, saying it is `doing` so, and runs it `again` while it fails in a way
/// that may pass: once Steam's CDN serves a manifest it did not, and once more for no connection
/// to Steam's build server.
fn build_retrying(steam: &Steam<'_>, (doing, again): (&str, &str)) -> Result<Ran, Halt> {
    let mut ran = build(steam, doing, steam.info_of).map_err(|(code, failure)| Halt {
        code,
        failure,
        earlier: None,
    })?;
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
        let earlier = ran.outcome;
        ran = build(steam, again, None).map_err(|(code, failure)| Halt {
            code,
            failure,
            earlier: Some(earlier),
        })?;
    }
    ran.info = info;
    Ok(ran)
}

/// Runs steamcmd's build once, printing first the info of the app `info_of` names, saying it is
/// `doing` so, and reads what it came to.
fn build(steam: &Steam<'_>, doing: &str, info_of: Option<u32>) -> Result<Ran, (ExitCode, Failure)> {
    upload::clear_log(steam.prepared).map_err(|error| not_run(&error, None))?;
    // A console left by an earlier run must never pass for this one's, so it goes first. One that
    // stays is said, and stops nothing: the run's own outcome matters more.
    let removed = match fs::remove_file(steam.saved) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Some(error),
        Err(_) | Ok(()) => None,
    };
    let name = steam.account.name();
    let before = Redactor::for_home(steam.home)
        .map_err(|error| not_run(&error, None))?
        .and_name(name);
    let spinner = Spinner::start("steam", doing, true);
    let finished = steamcmd::upload(steam.account, &steam.prepared.script, info_of).run(steam.home);
    let took = show::took(spinner.elapsed());
    let finished = match finished {
        Ok(finished) => finished,
        Err(error) => {
            spinner.failed(&format!("could not start after {took}"));
            return Err(not_run(&error, Some(&before)));
        }
    };
    // steamcmd may renew its token as it runs, so what is shown or saved of the run is redacted
    // with the secrets from after it too. Without them nothing of its output is shown or kept.
    let after = Redactor::for_home(steam.home);
    let log = fs::read(steam.prepared.log()).ok();
    // The outcome is judged on the output as steamcmd wrote it: redacting could turn an account
    // named `build` into a failure, or cut a BuildID.
    let mut console = String::from_utf8_lossy(&finished.output).into_owned();
    // The info is read apart, as text anyone can write into an app's settings could pass for a
    // line of the build.
    let info = info_of.map(|app| {
        release::info_at(&console, app).map_or_default(|at| console.drain(at).collect::<String>())
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
    let (redactor, written, line) = match after {
        Ok(after) => {
            let redactor = before.clone().and(after);
            let saved = fs::write(steam.saved, redactor.redact(&finished.output));
            let (written, mut line) = console_line(steam, saved.err(), removed);
            // The logs steamcmd wrote beside the console name the account too.
            if let Err(error) = redactor.redact_logs(&steam.prepared.output) {
                line = format!("{line}. {}", logs_unredacted(&error));
            }
            (Some(redactor), written, line)
        }
        Err(error) => {
            let removed_logs = redact::remove_logs(&steam.prepared.output);
            (None, false, logs_removed(steam, &error, removed_logs.err()))
        }
    };
    Ok(Ran {
        outcome,
        retry,
        spinner,
        took,
        redactor,
        before,
        info,
        log: line,
        written,
    })
}

/// What is said when a log of the build's folder could neither be redacted nor removed.
fn logs_unredacted(error: &io::Error) -> String {
    format!(
        "These logs may still hold the token or the account's name: {error}. Check them before \
         you share them"
    )
}

/// Hides the token and the account's name in the logs `app`'s earlier runs left in the home, for a
/// run of steamcmd that refused before any build: the build's own redaction never runs then.
fn hide_in_logs(home: &Path, account: &Account, app: u32) {
    let output = home.join(upload::APPS).join(app.to_string()).join("output");
    let done = Redactor::for_home(home).map_or_else(
        |_| redact::remove_logs(&output),
        |redactor| redactor.and_name(account.name()).redact_logs(&output),
    );
    if let Err(error) = done {
        show::note(&logs_unredacted(&error));
    }
}

/// Whether steamcmd's console was saved, and the `log` line that says where its logs are. `failed`
/// is why the console could not be written, and `removed` why the earlier one could not go.
fn console_line(
    steam: &Steam<'_>,
    failed: Option<io::Error>,
    removed: Option<io::Error>,
) -> (bool, String) {
    let own = steam.prepared.log();
    let Some(error) = failed else {
        let line = format!(
            "log  {}, and steamcmd's own log in {}",
            steam.saved.display(),
            own.display()
        );
        return (true, line);
    };
    let line = format!(
        "log  steamcmd's own log is in {}. Its console could not be written to {}: {error}{}",
        own.display(),
        steam.saved.display(),
        earlier_kept(removed)
    );
    (false, line)
}

/// The words that say the earlier console could not be removed, or none when it could.
pub(crate) fn earlier_kept(removed: Option<io::Error>) -> String {
    removed.map_or_else(String::new, |why| {
        format!(". The earlier console could not be removed: {why}")
    })
}

/// The `log` line for a run whose login could not be read again, as upload and workshop say it.
pub(crate) fn login_unread(error: &io::Error) -> String {
    format!(
        "log  none kept, since steamcmd's login could not be read again to hide its secrets: \
         {error}"
    )
}

/// The `log` line for a run whose login could not be read again, so that none of its logs may be
/// kept. `removed` is why the logs of the build's folder could not all go.
fn logs_removed(steam: &Steam<'_>, error: &io::Error, removed: Option<io::Error>) -> String {
    let output = steam.prepared.output.display();
    let kept = login_unread(error);
    removed.map_or_else(
        || format!("{kept}. The logs in {output} were removed"),
        |why| format!("{kept}. {why}. Remove the logs in {output}"),
    )
}

/// Asks Steam, when a key is at hand, whether `build_id` is what is live on `branch` now. False
/// only when Steam shows another build there, or no such branch. Asked a few times, since a
/// build just set live may not show at once. A Steam that cannot be asked holds nothing up.
fn confirmed_live(app: u32, branch: &str, build_id: u64, answers: Option<&Redactor>) -> bool {
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
            show::aside(&steams_words(&error, answers));
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

/// What the end of an upload is said with: the spinner, steamcmd's duration, the `log` line,
/// whether steamcmd's lines may be shown and the redactor for them, the login handed over, and the
/// Web API for setting the build live. `answers` is the redactor of the login after the run, none
/// when it could not be read, and the Web API's words are shown only through it.
struct Reporting<'upload> {
    spinner: Spinner,
    took: &'upload str,
    prepared: &'upload upload::Prepared,
    log: &'upload str,
    shown: bool,
    redactor: &'upload Redactor,
    answers: Option<&'upload Redactor>,
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

/// What [`report`] says an upload came to.
struct Reported {
    code: ExitCode,
    /// Where the build is live.
    live: Live,
    /// The failure that gave `code`, if any.
    failure: Option<Failure>,
}

impl Reported {
    /// Exit 5 for a build Steam holds but did not set live everywhere it was asked to. `live`
    /// shows where it is live. The failure's text is the `headline` and `reason` as printed.
    fn unfinished(live: Live, headline: &str, reason: &str) -> Self {
        Self {
            code: ExitCode::from(UNFINISHED),
            live,
            failure: Some(Failure {
                code: "unfinished",
                message: said(headline, reason),
            }),
        }
    }
}

/// Says what `outcome` came to, and answers its exit code, where the build is live and the failure.
fn report(outcome: &upload::Outcome, reporting: Reporting<'_>) -> Reported {
    let Reporting {
        spinner,
        took,
        prepared,
        log: logs,
        shown,
        redactor,
        answers,
        packed,
        default,
    } = reporting;
    let app = prepared.app_id;
    let (code, failure) = match outcome {
        upload::Outcome::Built { build_id } => {
            spinner.done(&format!("uploaded in {took}"));
            hand_on(*build_id);
            let live = prepared.held_back.as_ref().map_or_else(
                || {
                    set_live(
                        app,
                        *build_id,
                        prepared.set_live.as_deref(),
                        default,
                        logs,
                        answers,
                    )
                },
                |branch| first_on_default(app, *build_id, branch, default, logs, answers),
            );
            return live;
        }
        upload::Outcome::Previewed => {
            spinner.done(&format!("dry run finished in {took}"));
            show::success("nothing was uploaded, as asked", logs);
            (ExitCode::SUCCESS, None)
        }
        upload::Outcome::NotLoggedIn(line) => {
            spinner.failed(&format!("refused after {took}"));
            let (code, message) = login_refusal(line, redactor, packed);
            (code, Some(login_failure(code, message)))
        }
        upload::Outcome::BuiltThenFailed { build_id, reasons } => {
            spinner.failed(&format!("failed after {took}"));
            let headline = format!("app {app}: built as BuildID {build_id}, then steamcmd failed");
            show::failure(&headline, "", None);
            let mut lines = vec![headline];
            for reason in reasons.iter().filter(|_| shown) {
                let reason = redacted(redactor, reason);
                show::failure(&reason, "", None);
                lines.push(reason);
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
            show::note(logs);
            hand_on(*build_id);
            let failure = Failure::new("unfinished", lines.join("\n"));
            (ExitCode::from(UNFINISHED), Some(failure))
        }
        upload::Outcome::Failed(reasons) => {
            spinner.failed(&format!("failed after {took}"));
            let lines: Vec<String> = reasons
                .iter()
                .filter(|_| shown)
                .map(|reason| redacted(redactor, reason))
                .collect();
            for line in &lines {
                show::failure(line, "", None);
            }
            let (code, failure) = refused(reasons, &lines, prepared, logs);
            (code, Some(failure))
        }
    };
    Reported {
        code,
        live: Live::default(),
        failure,
    }
}

/// Says what a failed build left, and answers the exit code and the failure. `reasons` are
/// steamcmd's, `lines` the ones that were shown. Exit 5 is for a build Steam kept without setting
/// it live.
fn refused(
    reasons: &[String],
    lines: &[String],
    prepared: &upload::Prepared,
    logs: &str,
) -> (ExitCode, Failure) {
    let kept = upload::commit_refused(reasons, prepared.set_live.as_deref());
    if let Some(why) = &kept {
        show::note(upload::NOT_LIVE);
        show::note(why);
        show::hint(Hint {
            before: "see the app's branches with ",
            command: &format!("steamship builds {}", prepared.app_id),
            after: "",
        });
    }
    show::note(logs);
    let (code, word) = if kept.is_some() {
        (UNFINISHED, "unfinished")
    } else {
        (FAILED, "steamcmd-failed")
    };
    (ExitCode::from(code), Failure::of_reasons(word, lines, logs))
}

/// `error` as a person reads it, with the words Steam gave redacted, or withheld when no redactor
/// could be built.
fn steams_words(error: &webapi::Error, answers: Option<&Redactor>) -> String {
    error.said_with(&|steam| redact::answer(answers, steam))
}

/// `text` with every secret `redactor` knows replaced, for what steamship prints after judging it.
/// The account's name stays: it can be a word of steamship's own sentences.
pub(crate) fn redacted(redactor: &Redactor, text: &str) -> String {
    String::from_utf8_lossy(&redactor.redact_secrets(text.as_bytes())).into_owned()
}

/// Says where `build_id`, just uploaded for `app`, is live: on `branch`, which the script set it
/// live on, once Steam shows it there, and then on default, set through `default` when given.
/// Answers as [`report`] does. `logs` says where to look.
fn set_live(
    app: u32,
    build_id: u64,
    branch: Option<&str>,
    default: Option<&webapi::Api>,
    logs: &str,
    answers: Option<&Redactor>,
) -> Reported {
    if let Some(branch) = branch
        && !confirmed_live(app, branch, build_id, answers)
    {
        let headline = format!(
            "app {app}: BuildID {build_id} uploaded, but Steam does not show it live on {branch}"
        );
        show::failure(
            &headline,
            "",
            Some(Hint {
                before: "set it live with ",
                command: &format!("steamship promote {app} --build {build_id} --branch {branch}"),
                after: "",
            }),
        );
        show::note(logs);
        return Reported::unfinished(Live::default(), &headline, "");
    }
    let mut live = Live {
        branch: branch.is_some(),
        default: false,
    };
    if let Some(api) = default {
        if let Err(why) = live_on_default(api, app, build_id, branch, answers) {
            show::note(logs);
            return Reported::unfinished(live, &why, "");
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
    Reported {
        code: ExitCode::SUCCESS,
        live,
        failure: None,
    }
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
    answers: Option<&Redactor>,
) -> Reported {
    let make = format!(
        "make {branch} in Steamworks under SteamPipe, Builds, and set the build live there with "
    );
    let promote = format!("steamship promote {app} --build {build_id} --branch {branch}");
    let Some(api) = default else {
        let headline = format!("app {app}: BuildID {build_id} uploaded, but set live nowhere");
        let reason = format!(
            "app {app} has no branch but default until a build is live there, so steamcmd was \
             not asked to set it live on {branch}"
        );
        show::failure(
            &headline,
            &reason,
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
        return Reported::unfinished(Live::default(), &headline, &reason);
    };
    let reported = set_live(app, build_id, None, Some(api), logs, answers);
    if reported.live.default {
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
    reported
}

/// Sets `build_id` of `app` live on the default branch through `api`. When Steam did not, prints it
/// and answers the failure's text. The build is already live on `branch`, if the script sets one
/// live.
fn live_on_default(
    api: &webapi::Api,
    app: u32,
    build_id: u64,
    branch: Option<&str>,
    answers: Option<&Redactor>,
) -> Result<(), String> {
    let spinner = Spinner::start("default", "setting it live", false);
    let Err(error) = api.set_live(app, build_id, branch::DEFAULT_KEY) else {
        spinner.done("set live");
        return Ok(());
    };
    spinner.failed("not set live");
    let also = branch.map_or_else(String::new, |branch| format!(" and live on {branch}"));
    let headline = format!(
        "app {app}: BuildID {build_id} uploaded{also}, but Steam did not set it live on default"
    );
    let reason = steams_words(&error, answers);
    show::failure(
        &headline,
        &reason,
        Some(Hint {
            before: "set it live with ",
            command: &format!("steamship promote {app} --build {build_id} --branch default"),
            after: "",
        }),
    );
    Err(said(&headline, &reason))
}

/// What an upload came to, for the program that ran steamship with `--json`: `outcome` (`uploaded`
/// where Steam holds a build, `dry-run` for a dry run that finished, otherwise null), `app`,
/// `build_id` (null where nothing gave it), the `branch` the script sets live, whether the build is
/// `live` there and on `default`, and the build's `description`. A run that failed adds `error`.
fn summary(
    outcome: &upload::Outcome,
    live: Live,
    prepared: &upload::Prepared,
    description: &str,
    error: Option<serde_json::Value>,
) -> serde_json::Value {
    let branch = prepared.set_live.as_ref().or(prepared.held_back.as_ref());
    let build_id = match outcome {
        upload::Outcome::Built { build_id } | upload::Outcome::BuiltThenFailed { build_id, .. } => {
            Some(*build_id)
        }
        upload::Outcome::Previewed
        | upload::Outcome::NotLoggedIn(_)
        | upload::Outcome::Failed(_) => None,
    };
    let label = if *outcome == upload::Outcome::Previewed {
        Some("dry-run")
    } else {
        outcome
            .kept_on_steam(prepared.set_live.as_deref())
            .then_some("uploaded")
    };
    let mut printed = serde_json::json!({
        "outcome": label,
        "app": prepared.app_id,
        "build_id": build_id,
        "branch": branch,
        "live": live.branch,
        "default": live.default,
        "description": description,
    });
    if let (Some(error), Some(fields)) = (error, printed.as_object_mut()) {
        drop(fields.insert("error".to_owned(), error));
    }
    printed
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
