//! `steamship upload`.

use std::fs;
use std::iter;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Child;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use crate::common::git::{bare, commit, git, head};
use crate::common::pe::settings_ticking;
use steamship::install;
use steamship::ledger;
use steamship::platform::Platform;
#[cfg(any(any(windows, target_os = "linux"), unix))]
use steamship::run;
use steamship::steamcmd;
use steamship::webapi::STAND_IN;

#[cfg(unix)]
use crate::common;
#[cfg(unix)]
use crate::steamship_command;
use crate::{
    BETAS, BUILDS, CUT_OFF, CUT_OFF_SAID, DEMO_TYPED, DESCRIBED, KEY, NEEDS_VC, SECRETS, SET_LIVE,
    Session, UPDATED, assert_no_secret_in, failure, faked_with, in_game, in_game_with,
    packed_login, project, pushed, pushed_game, released_as, saved_login, shipping_dll, steamship,
    steamship_in, summary_of, upload, web_api, web_api_variables, with_demo, written,
};

#[test]
fn an_upload_in_ci_logs_in_as_the_account_handed_over() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo \"args: $*\"\n",
    );
    let (_project, script) = project(true);
    let script = script.to_str().unwrap();
    let (_, stdout, _) = steamship(
        &["upload", script, "--version", "1.4.0", "--dry-run"],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains(" +login build_bot "), "{stdout}{args}");
    let console = fs::read_to_string(home.path().join("apps/1000/output/steamcmd.log")).unwrap();
    assert!(
        console.contains(" +login [redacted] ") && !console.contains("build_bot"),
        "{stdout}{console}"
    );
    assert!(steamcmd::saved_login(home.path(), Platform::THIS).exists());
}

/// A steamcmd that logs in as the account it was given and prints `console`, a shell line, then
/// builds app 1000 as `BuildID` 18273645, or only previews the build when told to.
fn building_with(console: &str) -> String {
    format!(
        "#!/bin/sh\n\
         for arg in \"$@\"; do [ \"$previous\" = +login ] && account=\"$arg\"; \
         [ \"$previous\" = +run_app_build ] && script=\"$arg\"; previous=\"$arg\"; done\n\
         output=$(sed -n 's/^[[:space:]]*\"BuildOutput\"[[:space:]]*\"\\(.*\\)\"$/\\1/p' \"$script\")\n\
         mkdir -p \"$output\"\n\
         {console}\n\
         echo '[2026-09-28 20:24:13]: Building depot 1001...'\n\
         finished='build (BuildID 18273645).'\n\
         grep -q '\"Preview\"[[:space:]]*\"1\"' \"$script\" && finished='build preview.'\n\
         echo \"Successfully finished AppID 1000 $finished\" > \"$output/app_build_1000.log\"\n"
    )
}

/// The login line steamcmd prints for the account `building_with` was given.
const LOGGING_IN: &str = "echo \"Logging in user '$account' [U:1:1] to Steam Public...OK\"";

/// Whether `text` holds `name` as a whole word, in any case.
fn holds_word(text: &str, name: &str) -> bool {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|word| word.eq_ignore_ascii_case(name))
}

#[test]
fn an_accounts_name_in_steamcmds_lines_changes_no_outcome() {
    for name in ["build", "fully", "view", "273645"] {
        for preview in [false, true] {
            let home = faked_with(&building_with(LOGGING_IN));
            let (_project, script) = project(true);
            let mut args = vec![
                "upload",
                script.to_str().unwrap(),
                "--account",
                name,
                "--version",
                "1.4.0",
            ];
            if preview {
                args.push("--dry-run");
            }
            let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
            assert_eq!(code, Some(0_i32), "{name}: {stdout}{stderr}");
            if preview {
                assert!(
                    stderr.contains("nothing was uploaded, as asked")
                        || stdout.contains("nothing was uploaded, as asked"),
                    "{name}: {stdout}{stderr}"
                );
            } else {
                assert!(stdout.contains("BuildID 18273645"), "{name}: {stdout}");
            }
            let output = home.path().join("apps/1000/output");
            let console = fs::read_to_string(output.join("steamcmd.log")).unwrap();
            assert!(
                console.contains("Logging in user '[redacted]'")
                    && console.contains("Building depot 1001...")
                    && !holds_word(&console, name),
                "{name}: {console}"
            );
            if name != "build" {
                let log = fs::read_to_string(output.join("app_build_1000.log")).unwrap();
                let finished = if preview {
                    "build preview."
                } else {
                    "build (BuildID 18273645)."
                };
                assert_eq!(
                    log,
                    format!("Successfully finished AppID 1000 {finished}\n"),
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn an_anonymous_runs_saved_console_keeps_the_word() {
    let home = faked_with(&building_with(
        "echo 'Connecting anonymously to Steam Public...OK'",
    ));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = steamship(
        &[
            "upload",
            script.to_str().unwrap(),
            "--account",
            "anonymous",
            "--version",
            "1.4.0",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let console = fs::read_to_string(home.path().join("apps/1000/output/steamcmd.log")).unwrap();
    assert!(
        console.contains("Connecting anonymously to Steam Public...OK"),
        "{console}"
    );
}

#[test]
fn a_name_in_another_case_is_redacted_too() {
    let home = faked_with(&building_with(
        "echo \"Logging in user 'Build_Bot' [U:1:1] to Steam Public...OK\"",
    ));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let console = fs::read_to_string(home.path().join("apps/1000/output/steamcmd.log")).unwrap();
    assert!(
        console.contains("Logging in user '[redacted]'") && !console.contains("Build_Bot"),
        "{console}"
    );
}

#[test]
fn nothing_an_upload_leaves_under_apps_names_the_account() {
    let home = faked_with(&building_with(LOGGING_IN));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let files = written(&home.path().join("apps/1000"), &[]);
    for (path, contents) in &files {
        assert!(
            !String::from_utf8_lossy(contents).contains("build_bot"),
            "{path} names the account"
        );
    }
    let console = files
        .iter()
        .find(|(path, _)| path == "output/steamcmd.log")
        .map(|(_, contents)| String::from_utf8_lossy(contents).into_owned())
        .unwrap();
    assert!(console.contains("[redacted]"), "{console}");
}

#[test]
fn a_folder_named_like_a_log_is_left_out_of_the_rewrite() {
    let home = faked_with(BUILT);
    let folder = home.path().join("apps/1000/output/old.log");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("inside"), "build_bot").unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(!stdout.contains("may still hold the token"), "{stdout}");
    assert!(folder.is_dir());
}

#[cfg(unix)]
#[test]
fn a_log_that_cannot_be_rewritten_is_removed_and_the_upload_exits_as_it_would() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = faked_with(BUILT);
    let old = home.path().join("apps/1000/output/old.log");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "build_bot").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o444)).unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("BuildID 4242"), "{stdout}");
    assert!(!old.exists());
}

/// A steamcmd that has Valve build the upload as `BuildID` 777, then fails setting it live: it
/// writes the app and depot logs where it was told to, adds to its own content log, and exits 6.
const BUILT_THEN_FAILED: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    logs=\"$HOME/Steam/logs\"\n\
    [ \"$(uname)\" = Darwin ] && logs=\"$HOME/Library/Application Support/Steam/logs\"\n\
    case \"$(uname)\" in MINGW* | MSYS*) logs=\"$HOME/steamcmd/logs\" ;; esac\n\
    mkdir -p \"$output\" \"$logs\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 777).' > \"$output/app_build_1000.log\"\n\
    echo 'depot 1001: 1 file' > \"$output/depot_build_1001.log\"\n\
    echo 'this run: chunks sent' >> \"$logs/content_log.txt\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    echo '::set-output name=forged::yes'\n\
    echo 'ERROR! Failed to set build live on branch testing'\n\
    exit 6\n";

/// A steamcmd whose build Steam keeps but will not set live, reporting it as steamcmd 1788292693
/// did for Falcon Ridge Overseer's `SetLive` branch before the app had one: no build ID, only
/// "Failed to commit build".
const COMMIT_REFUSED: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo '[..]: ERROR! Failed to commit build for AppID 1000 : Failure' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    exit 6\n";

/// A steamcmd that has Valve build the upload as `BuildID` 4242, set live as the script asks.
const BUILT: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

/// `GetAppBetas` showing `build` live on testing.
fn testing_live(build: u64) -> &'static str {
    Box::leak(
        format!(
            r#"{{"response": {{"result": 1, "betas": {{"testing": {{"BuildID": {build}}}}}}}}}"#
        )
        .into_boxed_str(),
    )
}

#[test]
fn an_upload_with_a_key_at_hand_is_confirmed_live_on_its_branch_or_fails() {
    let home = faked_with(BUILT);
    let (_project, script) = project(true);

    let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("testing: BuildID 4242, as Steam shows it"),
        "{stdout}"
    );
    assert!(
        stdout.contains("app 1000: BuildID 4242, set live on testing"),
        "{stdout}"
    );

    // Steam shows the build it had until the new one shows, as it may just after setting it.
    let (late, _late_asked) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(7)),
        ("200 OK", testing_live(4242)),
    ]);
    let (caught_up, late_out, late_err) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &late)],
    );
    assert_eq!(caught_up, Some(0_i32), "{late_out}{late_err}");
    assert!(
        late_out.contains("testing: BuildID 4242, as Steam shows it"),
        "{late_out}"
    );

    // Steam goes on showing another build, however often it is asked.
    let (other, _asked) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
    ]);
    let (refused, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &other)],
    );
    assert_eq!(refused, Some(5_i32), "{said}{why}");
    assert!(
        said.contains("testing: BuildID 7, as Steam shows it"),
        "{said}"
    );
    for line in [
        "app 1000: BuildID 4242 uploaded, but Steam does not show it live on testing",
        "set it live with steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(why.contains(line), "{line:?} in {why}");
    }
}

#[test]
fn an_upload_asked_for_default_too_is_set_live_there_while_steam_shows_the_app_unreleased() {
    let home = faked_with(&released_as("unavailable"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    // One login: the build's own run asks whether the app is released.
    let runs = fs::read_to_string(home.path().join("runs")).unwrap();
    assert_eq!(runs.lines().count(), 1, "{runs}");
    assert!(
        runs.contains("+app_info_print 1000 +run_app_build "),
        "{runs}"
    );
    assert!(
        stdout.contains("\"default\":true")
            && stdout.contains("\"live\":true")
            && stdout.contains("\"outcome\":\"uploaded\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("unreleased, as Steam shows it (unavailable)")
            && stderr.contains("  default   \u{2713} set live\n")
            && stderr.contains("app 1000: BuildID 4242, set live on testing and default"),
        "{stderr}"
    );
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000&buildid=4242&betakey=public"),
        "{setting}"
    );
}

/// Uploads for default too with a key and the steamcmd `steamcmd`, which builds and shows no
/// release state. The build must go up to its branch alone, with `why` and how to set it live on
/// default said.
fn assert_default_left(steamcmd: &str, why: &str) {
    let home = faked_with(steamcmd);
    saved_login(home.path());
    let (_project, script) = project(true);
    // Only what the upload asks of Steam either way: a third request would fail the run.
    let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"default\":false") && stdout.contains("\"outcome\":\"uploaded\""),
        "{stdout}"
    );
    assert!(
        stderr.contains(&format!(
            "{why}. The build goes up without being set live on default. While the app is \
             unreleased, steamship promote 1000 --build 4242 --branch default does that\n"
        )) && stderr.contains("app 1000: BuildID 4242, set live on testing")
            && !stderr.contains("and default"),
        "{stderr}"
    );
}

#[test]
fn an_upload_asked_for_default_too_whose_app_info_is_cut_off_says_why_default_is_left() {
    assert_default_left(CUT_OFF, CUT_OFF_SAID);
}

#[test]
fn an_upload_asked_for_default_too_whose_steamcmd_printed_no_app_info_says_why_default_is_left() {
    assert_default_left(BUILT, "steamcmd printed no app info for app 1000");
}

#[test]
fn an_upload_asked_for_default_too_once_steam_shows_the_app_released_goes_to_its_branch_alone() {
    let home = faked_with(&released_as("released"));
    saved_login(home.path());
    let (_project, script) = project(true);
    // Only what the upload asks of Steam either way: a third request would fail the run.
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"default\":false")
            && stdout.contains("\"live\":true")
            && stdout.contains("\"outcome\":\"uploaded\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("released, as Steam shows it (released)")
            && stderr.contains(
                "app 1000 is released, so its default branch is set live in Steamworks, where \
                 Valve asks for a Steam Mobile confirmation. The build goes up without being set \
                 live on default"
            )
            && stderr.contains("app 1000: BuildID 4242, set live on testing")
            && !stderr.contains("and default"),
        "{stderr}"
    );
    assert!(home.path().join("builds").exists(), "the build went up");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(2)
            .collect();
    assert!(
        asked
            .iter()
            .all(|request| !request.contains("SetAppBuildLive")),
        "{asked:?}"
    );
}

#[test]
fn an_upload_asked_for_default_too_with_no_key_once_steam_shows_the_app_released_asks_for_none() {
    let home = faked_with(&released_as("released"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default"],
        &[("STEAMSHIP_WEB_API_KEY", "")],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let runs = fs::read_to_string(home.path().join("runs")).unwrap();
    assert_eq!(runs.lines().count(), 2, "{runs}");
    assert!(
        runs.lines()
            .next()
            .unwrap()
            .contains("+app_info_print 1000")
            && !runs.lines().next().unwrap().contains("+run_app_build"),
        "Steam is asked before anything goes up: {runs}"
    );
    let said = format!("{stdout}{stderr}");
    assert!(
        said.contains("  steam     \u{2717} released, as Steam shows it (released)\n")
            && said.contains(
                "app 1000 is released, so its default branch is set live in Steamworks, where \
                 Valve asks for a Steam Mobile confirmation. The build goes up without being set \
                 live on default"
            ),
        "{said}"
    );
    assert!(home.path().join("builds").exists(), "the build went up");
}

#[test]
fn an_upload_steam_did_not_set_live_on_default_says_how_to_and_exits_5() {
    let home = faked_with(&released_as("prerelease"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", r#"{"response": {"result": 8}}"#),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["default"], false, "{line}");
    assert_eq!(line["live"], true, "{line}");
    assert_eq!(line["outcome"], "uploaded", "{line}");
    assert_eq!(line["build_id"], 4242, "{line}");
    assert_eq!(line["error"]["code"], "unfinished", "{line}");
    let message = line["error"]["message"].as_str().unwrap();
    assert!(
        message
            .starts_with("Steam holds a build of app 1000, so a second upload would make another")
            && message.contains(
                "app 1000: BuildID 4242 uploaded and live on testing, but Steam did not set it \
                 live on default"
            ),
        "{message}"
    );
    for line in [
        "app 1000: BuildID 4242 uploaded and live on testing, but Steam did not set it live on \
         default",
        "set it live with steamship promote 1000 --build 4242 --branch default",
    ] {
        assert!(stderr.contains(line), "{line:?} in {stderr}");
    }
}

/// `GetAppList` for a key that holds app 1000.
const HOLDS_1000: &str = r#"{"applist": {"apps": {"app": [
    {"appid": 1000, "app_type": "game", "app_name": "Spacewar"}
]}}}"#;

#[test]
fn an_upload_to_an_app_with_no_branch_yet_goes_live_on_default_alone_and_says_how_to_go_on() {
    let home = faked_with(&released_as("unavailable"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", HOLDS_1000),
        ("200 OK", SET_LIVE),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["branch"], "testing", "{line}");
    assert_eq!(line["default"], true, "{line}");
    assert_eq!(line["live"], false, "{line}");
    assert_eq!(line["outcome"], "uploaded", "{line}");
    assert!(line.get("error").is_none(), "{line}");
    for line in [
        "  branch    testing, not on Steam yet\n",
        "app 1000 has no build live on its default branch, and Steam gives it no other branch \
         until it has",
        "app 1000: BuildID 4242, set live on default",
        "not set live on testing: Steam gives app 1000 no branch but default until a build is \
         live there",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(stderr.contains(line), "{line:?} in {stderr}");
    }
    let given = fs::read_to_string(home.path().join("apps/1000/app_build.vdf")).unwrap();
    assert!(!given.contains("SetLive"), "{given}");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000&buildid=4242&betakey=public"),
        "{setting}"
    );

    // Without --default the build is live nowhere, which the upload fails on.
    let (alone, _asked) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", HOLDS_1000),
    ]);
    let (nowhere, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &alone)],
    );
    assert_eq!(nowhere, Some(5_i32), "{said}{why}");
    for line in [
        "app 1000: BuildID 4242 uploaded, but set live nowhere",
        "app 1000 has no branch but default until a build is live there, so steamcmd was not \
         asked to set it live on testing",
        "set it live on default with steamship promote 1000 --build 4242 --branch default",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(why.contains(line), "{line:?} in {why}");
    }
}

#[test]
fn an_upload_asked_for_default_too_cannot_be_a_dry_run() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = project(false);
    let (code, _, stderr) = steamship(
        &[
            "upload",
            script.to_str().unwrap(),
            "--version",
            "1.4.0",
            "--dry-run",
            "--default",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

/// A steamcmd whose first `failing` runs run the shell commands `fail`, which fail the build, and
/// exit 6. Any run after has Valve build the upload as `BuildID` 4242. Each run adds a line to
/// `runs` in the home.
fn failing_then_built(fail: &str, failing: u32) -> String {
    format!(
        "#!/bin/sh\n\
         output=\"$HOME/apps/1000/output\"\n\
         mkdir -p \"$output\"\n\
         echo run >> \"$HOME/runs\"\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n\
         runs=$(wc -l < \"$HOME/runs\")\n\
         if [ $runs -le {failing} ]; then\n\
         {fail}\
         exit 6\n\
         fi\n\
         echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n"
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd 1791918946 did for Tidal Mill Inn 16
/// minutes after the branch's last build, the CDN not serving that build's manifest yet, named
/// here at `cdn`.
fn unserved_then_built(cdn: &str, failing: u32) -> String {
    let manifest = format!(
        "{}/depot/1001/manifest/6076939079705630407/5/7060534466868254360",
        cdn.strip_prefix("http://").unwrap()
    );
    failing_then_built(
        &format!(
            "echo '[2026-10-15 18:05:09]: Building depot 1001...'\n\
             echo '[2026-10-15 18:06:11]: ERROR! Failed to download manifest \"{manifest}\" (HTTP 404)'\n\
             echo '[2026-10-15 18:06:11]: ERROR! Build for depot 1001 failed : Failure' > \"$output/app_build_1000.log\"\n"
        ),
        failing,
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd 1788292693 did once in CI, having
/// reached Steam but not its build server.
fn unconnected_then_built(failing: u32) -> String {
    failing_then_built(UNCONNECTED, failing)
}

/// What the failing runs of [`unconnected_then_built`] do.
const UNCONNECTED: &str = "echo '[2026-10-12 15:35:56]: Building depot 1001...'\n\
    echo '[2026-10-12 15:35:56]: ERROR! Failed to initialize build on server (No Connection) '\n\
    echo '[2026-10-12 15:35:56]: ERROR! Build for depot 1001 failed : Failure' > \"$output/app_build_1000.log\"\n";

/// A steamcmd whose first `failing` runs fail as steamcmd 1791918946 did once in CI, Steam
/// dropping it before it logged in.
fn disconnected_then_built(failing: u32) -> String {
    failing_then_built(
        "echo 'Waiting for user info...'\n\
         echo 'steamcmd has been disconnected from steam with result 3 (No Connection)'\n\
         echo 'ERROR! (Timed out)'\n",
        failing,
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd did on every system of one CI run
/// within the same minute, Steam sending it no client config.
fn no_client_config_then_built(failing: u32) -> String {
    failing_then_built(
        "echo 'Waiting for client config...ERROR! Info request for AppId 7 returned error Timeout.'\n\
         echo 'OK'\n\
         echo 'ERROR! Failed to load platform info'\n",
        failing,
    )
}

/// How many times the steamcmd of `failing_then_built` ran in `home`.
fn runs(home: &Path) -> usize {
    fs::read_to_string(home.join("runs"))
        .unwrap()
        .lines()
        .count()
}

#[test]
fn an_upload_waits_for_the_cdn_to_serve_the_last_manifest_then_uploads_again() {
    let (cdn, requests) = web_api(vec![("200 OK", "manifest")]);
    let home = faked_with(&unserved_then_built(&cdn, 1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    assert!(
        requests.recv().unwrap().starts_with(
            "GET /depot/1001/manifest/6076939079705630407/5/7060534466868254360 HTTP/1.1\r\n"
        ),
        "the manifest steamcmd could not download is asked for"
    );
    for line in [
        "\u{2717} Steam's CDN does not serve depot 1001's last manifest yet\n",
        "  cdn       \u{2713} served after ",
        "  steam     \u{2713} uploaded in ",
        "app 1000: BuildID 4242, set live on testing",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
}

#[test]
fn an_upload_stops_asking_steamcmd_after_four_runs_the_cdn_never_served() {
    let (cdn, _requests) = web_api(vec![("401 Refused", ""); 3]);
    let home = faked_with(&unserved_then_built(&cdn, 99));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 4);
    assert_eq!(stdout.matches("answered 401 after ").count(), 3, "{stdout}");
    for reason in [
        "Build for depot 1001 failed : Failure",
        "Steam's CDN still does not serve depot 1001's last manifest, which steamcmd builds the \
         upload against. Upload again later",
    ] {
        assert!(stderr.contains(reason), "{reason:?} in {stderr}");
    }
}

#[test]
fn an_upload_runs_steamcmd_once_more_when_it_did_not_reach_steams_build_server() {
    let home = faked_with(&unconnected_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} no connection to Steam's build server after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let unreached = faked_with(&unconnected_then_built(99));
    let (failed, said, why) = upload(&script, unreached.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(unreached.path()), 2, "only once more");
    assert!(
        why.contains("Failed to initialize build on server (No Connection)"),
        "{why}"
    );
}

#[test]
fn an_upload_whose_app_info_lost_its_closing_brace_still_reads_why_the_build_failed() {
    // App 1000's info as `released_as` prints it, less its last line.
    let lost = "echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
                printf '\"1000\"\\n{\\n\\t\"common\"\\n\\t{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n\
                printf '\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n\\t}\\n'\n";
    let home = faked_with(&failing_then_built(&format!("{lost}{UNCONNECTED}"), 99));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![("200 OK", BETAS)]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2, "run once more for no connection");
    assert!(
        stderr.contains("Failed to initialize build on server (No Connection)"),
        "{stderr}"
    );
}

#[test]
fn an_upload_runs_steamcmd_once_more_when_steam_disconnects_it() {
    let home = faked_with(&disconnected_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} Steam disconnected steamcmd after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let dropped = faked_with(&disconnected_then_built(99));
    let (failed, said, why) = upload(&script, dropped.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(dropped.path()), 2, "only once more");
    assert!(why.contains("Timed out"), "{why}");
}

#[test]
fn an_upload_runs_steamcmd_once_more_when_steam_sends_it_no_client_config() {
    let home = faked_with(&no_client_config_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} Steam sent steamcmd no client config after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let unsent = faked_with(&no_client_config_then_built(99));
    let (failed, said, why) = upload(&script, unsent.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(unsent.path()), 2, "only once more");
    assert!(why.contains("Failed to load platform info"), "{why}");
}

#[test]
fn an_upload_with_json_prints_only_what_came_of_it_on_standard_output() {
    let home = faked_with(BUILT);
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let mut summary = summary_of(&stdout);
    // The description names the commit the test's repository is at, which differs each run.
    let description = summary
        .as_object_mut()
        .and_then(|fields| fields.remove("description"));
    assert!(
        description
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .is_some_and(|description| description.starts_with("1.4.0 ")),
        "{description:?}"
    );
    assert_eq!(
        summary,
        serde_json::json!({
            "outcome": "uploaded",
            "app": 1000_u32,
            "build_id": 4242_u64,
            "branch": "testing",
            "live": true,
            "default": false,
        })
    );
    assert!(
        stderr.contains("app 1000: BuildID 4242, set live on testing"),
        "what is shown for a person goes to standard error: {stderr}"
    );
}

#[test]
fn an_upload_with_json_draws_its_spinner_on_the_terminal_standard_error_is() {
    // Slow enough that the spinner is drawn before the upload ends.
    let home = faked_with(&format!("{BUILT}sleep 1\n"));
    let (folder, script) = project(true);
    let summary = folder.path().join("summary.json");
    let line = format!(
        "'{}' upload '{}' --version 1.4.0 --account build_bot --json > '{}'; echo \"exited $?\"",
        env!("CARGO_BIN_EXE_steamship"),
        script.display(),
        summary.display()
    );
    let mut session = Session::start(&line, home.path());
    // Drawn with how long it has been, where elsewhere it would be written once, ending "...".
    session.wait_for(" uploading 0 s");
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(!seen.contains("uploading..."), "{seen}");
    assert!(seen.contains("app 1000: BuildID 4242"), "{seen}");
    assert_eq!(session.end().wait().unwrap(), Some(0_i32));
    let written = fs::read_to_string(&summary).unwrap();
    assert!(written.starts_with("{\"app\":1000,"), "{written}");
}

#[test]
fn a_failed_upload_with_json_still_says_what_steam_kept() {
    let home = faked_with(BUILT_THEN_FAILED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let summary = summary_of(&stdout);
    for (field, expected) in [
        ("outcome", serde_json::json!("uploaded")),
        ("build_id", serde_json::json!(777_u64)),
        ("live", serde_json::json!(false)),
    ] {
        assert_eq!(summary.get(field), Some(&expected), "{field}: {summary}");
    }
    assert_eq!(summary["error"]["code"], "unfinished", "{summary}");
    let message = summary["error"]["message"].as_str().unwrap();
    assert!(
        message
            .starts_with("Steam holds a build of app 1000, so a second upload would make another")
            && message.contains("Failed to set build live on branch testing"),
        "{message}"
    );
}

#[test]
fn a_failed_upload_gives_its_reasons_as_json() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n\
         echo 'ERROR! Failed to initialize build on server (Access Denied)'\n\
         exit 1\n",
    );
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["outcome"], serde_json::Value::Null, "{line}");
    assert_eq!(line["build_id"], serde_json::Value::Null, "{line}");
    assert_eq!(line["error"]["code"], "steamcmd-failed", "{line}");
    let message = line["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Failed to initialize build on server (Access Denied)"),
        "{message}"
    );
}

#[test]
fn an_upload_steam_limits_says_login_limited_as_json() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"\n\
         exit 5\n",
    );
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["error"]["code"], "login-limited", "{line}");
}

#[test]
fn a_build_up_before_steamcmd_altered_itself_says_so_as_json() {
    let up = faked_with(&built_then("echo >> \"$0\""));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        up.path(),
        &["--version", "1.4.0", "--json"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["outcome"], "uploaded", "{line}");
    assert_eq!(line["live"], true, "{line}");
    assert_eq!(line["build_id"], 4242, "{line}");
    assert_eq!(line["error"]["code"], "unfinished", "{line}");
    let message = line["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Steam holds a build of app 1000")
            && message.contains("no longer the pinned one"),
        "{message}"
    );

    // A dry run holds nothing on Steam, so the fault is the failure.
    let preview = leaking_with("echo >> \"$0\"");
    let (earlier, said, errors) = upload(
        &script,
        preview.path(),
        &["--version", "1.4.0", "--dry-run", "--json"],
        &[],
    );
    assert_eq!(earlier, Some(4_i32), "{said}{errors}");
    let line = summary_of(&said);
    assert_eq!(line["outcome"], "dry-run", "{line}");
    assert_eq!(line["error"]["code"], "steamcmd-install", "{line}");
    assert!(
        !line["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Steam holds"),
        "{line}"
    );
}

#[test]
fn a_build_steam_would_not_set_live_before_steamcmd_altered_itself_names_both_as_json() {
    let home = faked_with(&BUILT_THEN_FAILED.replace("exit 6\n", "echo >> \"$0\"\nexit 6\n"));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--json"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["outcome"], "uploaded", "{line}");
    assert_eq!(line["build_id"], 777, "{line}");
    assert_eq!(line["live"], false, "{line}");
    assert_eq!(line["error"]["code"], "unfinished", "{line}");
    let message = line["error"]["message"].as_str().unwrap();
    let held = message.find("Failed to set build live on branch testing");
    let altered = message.find("no longer the pinned one");
    assert!(
        held.is_some() && altered.is_some() && held < altered,
        "{message}"
    );
}

#[test]
fn a_failed_upload_whose_steamcmd_then_altered_itself_keeps_its_own_word_as_json() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n\
         echo 'ERROR! Failed to initialize build on server (Access Denied)'\n\
         echo >> \"$0\"\n\
         exit 1\n",
    );
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--json"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["outcome"], serde_json::Value::Null, "{line}");
    assert_eq!(line["error"]["code"], "steamcmd-failed", "{line}");
    let message = line["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Failed to initialize build on server (Access Denied)")
            && !message.contains("no longer the pinned one"),
        "{message}"
    );
}

#[test]
fn a_build_steam_kept_unnamed_is_uploaded_with_no_id_as_json() {
    let home = faked_with(COMMIT_REFUSED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    assert_eq!(line["outcome"], "uploaded", "{line}");
    assert_eq!(line["build_id"], serde_json::Value::Null, "{line}");
    assert_eq!(line["error"]["code"], "unfinished", "{line}");
}

#[test]
fn an_upload_that_worked_has_no_error_and_exactly_the_documented_fields() {
    let home = faked_with(BUILT);
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let line = summary_of(&stdout);
    let mut keys: Vec<&str> = line
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "app",
            "branch",
            "build_id",
            "default",
            "description",
            "live",
            "outcome"
        ],
        "{line}"
    );
}

#[test]
fn a_build_steam_kept_but_set_live_nowhere_is_said_to_be_there_and_found_with_a_key() {
    let home = faked_with(COMMIT_REFUSED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    for said in [
        "Failed to commit build for AppID 1000 : Failure",
        "the build is on Steam but not live: it is listed in Steamworks under SteamPipe, Builds",
        "\"SetLive\" names \"testing\". Steam sets no build live on a branch the app does not have",
        "see the app's branches with steamship builds 1000",
    ] {
        assert!(stderr.contains(said), "{said:?} in {stderr}");
    }
    let description = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  build     "))
        .unwrap()
        .to_owned();

    // With a key, the build is looked up by the description it was uploaded with.
    let builds: &'static str = Box::leak(
        format!(
            r#"{{"response": {{"builds": {{"20000001": {{"Description": "{description}"}}}}}}}}"#
        )
        .into_boxed_str(),
    );
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", builds)]);
    let (keyed, keyed_out, keyed_err) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(keyed, Some(5_i32), "{keyed_out}{keyed_err}");
    for said in [
        "app 1000: built as BuildID 20000001, then steamcmd failed",
        "\"SetLive\" names \"testing\". Steam sets no build live on a branch the app does not have",
        "set it live with steamship promote 1000 --build 20000001 --branch testing",
    ] {
        assert!(keyed_err.contains(said), "{said:?} in {keyed_err}");
    }
    assert!(!keyed_err.contains("listed in Steamworks"), "{keyed_err}");
    let _branches = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamApps/GetAppBuilds/v1/?") && asked.contains("appid=1000"),
        "{asked}"
    );
}

#[test]
fn an_upload_built_then_failed_keeps_its_build_id_and_shows_steams_logs_in_actions() {
    let home = faked_with(BUILT_THEN_FAILED);
    let logs = steamcmd::state_folder(home.path(), Platform::THIS).join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("content_log.txt"), "an older run\n").unwrap();
    let outputs = home.path().join("github_output");
    fs::write(&outputs, "").unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_OUTPUT", outputs.to_str().unwrap()),
        ],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    for said in [
        "app 1000: built as BuildID 777, then steamcmd failed",
        "Failed to set build live on branch testing",
        "set it live with steamship promote 1000 --build 777 --branch testing",
    ] {
        assert!(stderr.contains(said), "{said:?} in {stderr}");
    }
    assert_eq!(fs::read_to_string(&outputs).unwrap(), "build-id=777\n");
    assert!(stdout.contains("::add-mask::build_bot\n"), "{stdout}");
    for group in [
        "::group::steamcmd's console\n",
        "::group::app_build_1000.log\n",
        "::group::depot_build_1001.log\n",
        "::group::steamcmd's content_log.txt, from this run\n::stop-commands::",
    ] {
        assert!(stdout.contains(group), "{group:?} in {stdout}");
    }
    assert_eq!(
        stdout.matches("::group::").count(),
        4,
        "the console and app log once each, and no other log taken for a depot's: {stdout}"
    );
    assert!(stdout.contains("this run: chunks sent"), "{stdout}");
    assert!(!stdout.contains("an older run"), "{stdout}");
    let forged = stdout.find("::set-output name=forged::yes").unwrap();
    let stopped = stdout.find("::stop-commands::").unwrap();
    assert!(
        stopped < forged,
        "a command in a log is shown only with commands stopped"
    );
}

/// A steamcmd that logs in, fails to reach the build server, says where it is, and then waits as
/// one that has hung. A run that ended so would be run again, were steamship not told to stop.
#[cfg(unix)]
const HUNG: &str = "#!/bin/sh\n\
    echo ran >> \"$HOME/runs\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    echo '[2026-10-12 15:35:56]: ERROR! Failed to initialize build on server (No Connection) '\n\
    echo $$ > \"$HOME/steamcmd.pid\"\n\
    exec sleep 120\n";

#[cfg(unix)]
#[test]
fn an_upload_told_to_stop_stops_steamcmd_shows_what_it_said_and_ends_by_that_signal() {
    use std::os::unix::process::ExitStatusExt as _;

    for (name, number) in [("INT", 2_i32), ("HUP", 1_i32), ("TERM", 15_i32)] {
        let home = faked_with(HUNG);
        let outputs = home.path().join("github_output");
        fs::write(&outputs, "").unwrap();
        let (_project, script) = project(true);
        let mut command = steamship_command();
        let _: &mut Command = command
            .args(["upload", script.to_str().unwrap(), "--account", "build_bot"])
            .args(["--version", "1.4.0"])
            .env_remove("CI")
            .env("STEAMSHIP_HOME", home.path())
            .env("STEAMSHIP_LOGIN", packed_login())
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_OUTPUT", &outputs)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = run::spawn(&mut command).unwrap();
        let pid_file = home.path().join("steamcmd.pid");
        let waiting = Instant::now();
        let steamcmd = loop {
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .filter(|pid| pid.ends_with('\n'))
            {
                break pid.trim().to_owned();
            }
            assert!(
                waiting.elapsed() < Duration::from_secs(60),
                "steamcmd never started"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let pid = started.id().to_string();
        let signalled = run::output(run::command("kill").args(["-s", name, &pid])).unwrap();
        assert!(signalled.status.success(), "{signalled:?}");
        let (ending, endings) = mpsc::channel();
        let waited = thread::spawn(move || ending.send(started.wait_with_output().unwrap()));
        let Ok(ended) = endings.recv_timeout(Duration::from_secs(30)) else {
            drop(run::output(run::command("kill").args(["-s", "KILL", &pid])));
            panic!("{name}: steamship did not end");
        };
        waited.join().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(home.path().join("runs")).unwrap(),
            "ran\n",
            "{name}: steamcmd ran again after steamship was told to stop"
        );
        let stdout = String::from_utf8_lossy(&ended.stdout);
        let stderr = String::from_utf8_lossy(&ended.stderr);
        assert_eq!(
            ended.status.signal(),
            Some(number),
            "{name}: {stdout}{stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "steamship was told to stop by SIG{name}, and stopped steamcmd"
            )) && stderr.contains("Failed to initialize build on server (No Connection)"),
            "{name}: {stderr}"
        );
        assert!(
            stdout.contains("::group::steamcmd's console\n")
                && stdout.contains("Logging in user [redacted] to Steam Public...OK"),
            "{name}: {stdout}"
        );
        let gone = run::output(run::command("kill").args(["-0", &steamcmd])).unwrap();
        assert!(!gone.status.success(), "{name}: steamcmd still runs");
    }
}

#[test]
fn a_failed_upload_outside_actions_shows_no_log_groups() {
    let home = faked_with(BUILT_THEN_FAILED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("Failed to set build live on branch testing"),
        "{stdout}{stderr}"
    );
    assert!(!stdout.contains("::group::"), "{stdout}");
}

#[test]
fn upload_refuses_a_version_a_description_cannot_carry_and_exits_2() {
    let (_project, script) = project(true);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(&script, home.path(), &["--version", "1.0\""], &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        failure(&stderr).starts_with("the version \"1.0\\\"\" is not"),
        "{stderr}"
    );
}

#[test]
fn upload_refuses_scripts_outside_a_repository_and_exits_2() {
    let (_project, script) = project(false);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(&script, home.path(), &["--version", "1.0"], &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        stderr
            .trim_end()
            .ends_with("which the build description names"),
        "{stderr}"
    );
}

#[test]
fn upload_says_git_is_missing_rather_than_blaming_the_folder_and_exits_2() {
    let (_project, script) = project(true);
    let home = tempfile::tempdir().unwrap();
    let nowhere = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.0"],
        &[("PATH", nowhere.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(2_i32));
    assert_eq!(
        failure(&stderr).trim_end(),
        "Git is not installed, or not on the PATH, and the build description names the commit \
         the scripts are at. Install Git, or put it on the PATH"
    );
}

#[test]
fn upload_runs_check_first_and_exits_2_on_a_refusal() {
    let (project, script) = project(true);
    fs::write(
        project
            .path()
            .join("gate (x86) 1a2b/content/steam_appid.txt"),
        "1000",
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.0"], &[]);
    assert_eq!(code, Some(2_i32));
    assert_eq!(stdout, "steamship upload\n");
    assert!(failure(&stderr).starts_with("refused: "), "{stderr}");
}

/// A home with a saved login holding [`SECRETS`], where steamcmd is a script that does what the
/// setup-steamcmd action once did (GHSA-mj96-mh85-r574): prints steamcmd's login files, into its
/// console and its build log. Then it reports the build finished. With
/// `STEAMSHIP_FAKE_LOGGED_OUT` set it reports instead that it has no login, and with
/// `STEAMSHIP_FAKE_FAIL` set that the server refused the build.
fn leaking() -> tempfile::TempDir {
    leaking_with("")
}

/// [`leaking`] whose steamcmd also runs `first`, a shell line, once it knows where to write.
fn leaking_with(first: &str) -> tempfile::TempDir {
    let head = "#!/bin/sh\n\
         for arg in \"$@\"; do [ \"$previous\" = +run_app_build ] && script=\"$arg\"; previous=\"$arg\"; done\n\
         output=$(sed -n 's/^[[:space:]]*\"BuildOutput\"[[:space:]]*\"\\(.*\\)\"$/\\1/p' \"$script\")\n";
    let tail = "cat \"$HOME/Steam/config/config.vdf\" \"$HOME\"/Steam/userdata/*/config/localconfig.vdf\n\
         if [ -n \"$STEAMSHIP_FAKE_LOGGED_OUT\" ]; then\n\
           echo 'Cached credentials not found.'\n\
           echo 'FAILED (No cached credentials and @NoPromptForPassword is set)'\n\
           exit 5\n\
         fi\n\
         if [ -n \"$STEAMSHIP_FAKE_FAIL\" ]; then\n\
           echo '[..]: ERROR! Failed to initialize build on server (Access Denied)' \
             > \"$output/app_build_1000.log\"\n\
           exit 6\n\
         fi\n\
         finished='build (BuildID 4242).'\n\
         grep -q '\"Preview\"[[:space:]]*\"1\"' \"$script\" && finished='build preview.'\n\
         { cat \"$HOME/Steam/config/config.vdf\"; \
           echo \"Successfully finished AppID 1000 $finished\"; } \
           > \"$output/app_build_1000.log\"\n";
    let home = faked_with(&format!("{head}{first}\n{tail}"));
    let [token, local] = SECRETS;
    let put = |relative: &str, contents: String| {
        let path = home.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    put(
        "Steam/config/config.vdf",
        format!(
            "\"InstallConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \
             \"ConnectCache\" {{ \"3a1f0c2e\" \"{token}\" }} }} }} }} }}\n"
        ),
    );
    put(
        "Steam/userdata/12345/config/localconfig.vdf",
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{local}\" }} }}\n"),
    );
    home
}

#[test]
fn nothing_upload_prints_or_writes_holds_the_login() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  app       \u{2713} 1000, 1 depot, 1 file, checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\u{2713} app 1000: BuildID 4242, set live on testing\n"),
        "{stdout}"
    );
    assert_no_secret_in("stdout", stdout.as_bytes());
    let except = [
        "Steam/config/config.vdf",
        "Steam/userdata/12345/config/localconfig.vdf",
        "apps/1000/output/app_build_1000.log",
    ];
    let files = written(home.path(), &except);
    assert!(
        files
            .iter()
            .any(|(path, _)| path == "apps/1000/output/steamcmd.log"),
        "steamcmd's output is kept"
    );
    for (path, contents) in files {
        assert_no_secret_in(&path, &contents);
    }
}

#[test]
fn an_upload_in_a_github_actions_step_hands_the_build_id_on_as_its_output() {
    let home = leaking();
    let (project, script) = project(true);
    let outputs = project.path().join("github_output");
    fs::write(&outputs, "earlier=kept\n").unwrap();
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("GITHUB_OUTPUT", outputs.to_str().unwrap())],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        fs::read_to_string(&outputs).unwrap(),
        "earlier=kept\nbuild-id=4242\n"
    );
    let missing = project.path().join("no such folder").join("output");
    let (still, _, said) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("GITHUB_OUTPUT", missing.to_str().unwrap())],
    );
    assert_eq!(still, Some(0_i32), "the upload is done: {said}");
    assert!(
        said.contains("the BuildID could not be handed to the workflow"),
        "{said}"
    );
}

#[test]
fn upload_refuses_a_dll_nothing_installs_before_it_logs_in() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "fmod.dll"], None);
    let (code, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(
            "game.dll: imports fmod.dll, which neither Windows, the build's depots nor a \
             Steamworks Common Redistributable has. Ship it in a depot, or link its library in \
             statically"
        ),
        "{why}"
    );
    assert!(
        !home.path().join(install::FOLDER).exists(),
        "steamcmd was never installed to log in with"
    );
}

/// A steamcmd that, asked for app 1000's info, prints what the home's `app_info` file holds as
/// the settings, and otherwise has Valve build the upload as [`released_as`] does.
const TICKING_ON_STEAM: &str = "#!/bin/sh\n\
    case \" $* \" in *' +app_info_print '*)\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
    cat \"$HOME/app_info\"\n\
    echo 'Unloading Steam API...OK'\n\
    case \" $* \" in *' +run_app_build '*) ;; *) exit 0;; esac;;\n\
    esac\n\
    echo built >> \"$HOME/builds\"\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

#[test]
fn upload_asks_steam_which_redistributables_the_app_ticks_when_the_build_needs_one() {
    let home = faked_with(TICKING_ON_STEAM);
    // Steam is asked whether the branch exists, and once a build is up whether it is live.
    let uploaded = |ticked: &[&str], answers: Vec<(&'static str, &'static str)>| {
        fs::write(home.path().join("app_info"), settings_ticking(ticked)).unwrap();
        let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], None);
        let (host, _requests) = web_api(answers);
        upload(
            &script,
            home.path(),
            &["--version", "1.4.0"],
            &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
        )
    };

    let (refused, said, why) = uploaded(&[], vec![("200 OK", BETAS)]);
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(&format!(
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks"
        )),
        "{why}"
    );
    assert!(!home.path().join("builds").exists(), "nothing went up");

    let (code, shown, _) = uploaded(
        &["228988"],
        vec![("200 OK", BETAS), ("200 OK", testing_live(4242))],
    );
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(
        shown.contains("  redist    \u{2713} VC 2019 Redist, ticked in Steamworks\n"),
        "{shown}"
    );
    assert!(home.path().join("builds").exists(), "the build went up");
}

#[test]
fn an_upload_asking_which_redistributables_the_app_ticks_fails_on_app_info_cut_off() {
    let home = faked_with(TICKING_ON_STEAM);
    let whole = settings_ticking(&["228988"]);
    let (cut, _) = whole.rsplit_once("}\n").unwrap();
    fs::write(home.path().join("app_info"), cut).unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], None);
    let (host, _requests) = web_api(vec![("200 OK", BETAS)]);
    let (code, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(1_i32), "{said}{why}");
    let told = failure(&why);
    assert!(
        told.starts_with("steamcmd printed settings for app 1000 that cannot be read (line ")
            && told.ends_with(": a block is missing its closing }). Run the command again"),
        "{why}"
    );
    assert!(!home.path().join("builds").exists(), "nothing went up");
}

#[test]
fn an_upload_without_a_login_says_to_log_in_and_exits_3() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_LOGGED_OUT", "1")],
    );
    assert_eq!(code, Some(3_i32));
    assert_eq!(
        failure(&stderr),
        "not logged in: steamcmd holds no saved login for the build account"
    );
    assert!(
        stderr.ends_with("log the build account in again with steamship login\n"),
        "{stderr}"
    );
    assert_no_secret_in("stdout", stdout.as_bytes());
    assert_no_secret_in("stderr", stderr.as_bytes());
}

#[test]
fn an_upload_exits_6_when_steam_limits_logins_and_3_when_it_refuses_the_login() {
    let (_project, script) = project(true);
    for (reason, expected) in [("Rate Limit Exceeded", 6_i32), ("Invalid Password", 3_i32)] {
        let home = faked_with(&format!(
            "#!/bin/sh\n\
             echo \"Logging in user 'build_bot' [U:1:0] to Steam Public...FAILED ({reason})\"\n\
             exit 5\n"
        ));
        let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
        assert_eq!(code, Some(expected), "{reason}: {stdout}{stderr}");
    }
}

/// A steamcmd that prints `said` as the account it was given, then exits 5.
fn refusing_as_given(said: &str) -> String {
    format!(
        "#!/bin/sh\n\
         for arg in \"$@\"; do [ \"$previous\" = +login ] && account=\"$arg\"; previous=\"$arg\"; done\n\
         {said}\n\
         exit 5\n"
    )
}

#[test]
fn an_accounts_name_in_a_refusal_changes_neither_its_exit_code_nor_its_message() {
    let (_project, script) = project(true);
    let cases = [
        (
            "limit",
            "echo \"Logging in user '$account' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "steam",
            "echo \"Logging in user '$account' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "cached",
            "echo \"Logging in user '$account' [U:1:0] to Steam Public...\"\n\
             echo 'FAILED (No cached credentials and @NoPromptForPassword is set)'",
            3_i32,
            "steamcmd holds no saved login for the build account",
        ),
    ];
    for (name, said, expected, words) in cases {
        let home = faked_with(&refusing_as_given(said));
        let (code, stdout, stderr) = steamship(
            &[
                "upload",
                script.to_str().unwrap(),
                "--account",
                name,
                "--version",
                "1.4.0",
            ],
            Some(home.path()),
            &[],
        );
        assert_eq!(code, Some(expected), "{name}: {stdout}{stderr}");
        assert_eq!(
            failure(&stderr),
            format!("not logged in: {words}"),
            "{name}"
        );
    }
}

#[test]
fn an_account_named_like_steamships_words_leaves_its_sentences_whole_in_a_failed_upload() {
    let (_project, script) = project(true);
    let home = faked_with(&refusing_as_given(
        "echo \"Logging in user '$account' [U:1:0] to Steam Public...OK\"",
    ));
    let (code, stdout, stderr) = steamship(
        &[
            "upload",
            script.to_str().unwrap(),
            "--account",
            "steamcmd",
            "--version",
            "1.4.0",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("steamcmd exited 5 without saying why"),
        "{stderr}"
    );
    assert!(!format!("{stdout}{stderr}").contains("user 'steamcmd'"));
}

#[test]
fn an_account_named_like_a_word_of_steamcmds_errors_changes_no_reason_or_advice() {
    let (_project, script) = project(true);
    for (name, said, words) in [
        (
            "failed",
            "echo 'ERROR! Failed to set build live on branch testing'",
            "Failed to set build live on branch testing",
        ),
        (
            "trusted",
            "echo 'unable to load trusted SSL root certificates'",
            "Install the system's certificate store (ca-certificates)",
        ),
    ] {
        let home = faked_with(&refusing_as_given(said));
        let (code, stdout, stderr) = steamship(
            &[
                "upload",
                script.to_str().unwrap(),
                "--account",
                name,
                "--version",
                "1.4.0",
            ],
            Some(home.path()),
            &[],
        );
        assert_eq!(code, Some(1_i32), "{name}: {stdout}{stderr}");
        assert!(stderr.contains(words), "{name}: {stderr}");
    }
}

#[test]
fn a_redistributables_check_steam_refuses_leaves_no_log_naming_the_account() {
    let home = faked_with(&refusing_as_given(
        "echo \"Logging in user '$account' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"",
    ));
    let old = home.path().join("apps/1000/output/old.log");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "Logging in user 'build_bot'").unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], None);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    assert_eq!(
        fs::read_to_string(&old).unwrap(),
        "Logging in user '[redacted]'"
    );
}

#[cfg(unix)]
#[test]
fn a_log_held_after_a_default_check_steam_refuses_is_named_in_a_note() {
    use std::os::unix::fs::PermissionsExt as _;

    let (_project, script) = project(true);
    let home = faked_with(&refusing_as_given(
        "echo \"Logging in user '$account' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"",
    ));
    let output = home.path().join("apps/1000/output");
    let old = output.join("old.log");
    fs::create_dir_all(&output).unwrap();
    fs::write(&old, "Logging in user 'build_bot'").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&output, fs::Permissions::from_mode(0o555)).unwrap();
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default"],
        &[],
    );
    fs::set_permissions(&output, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    let seen = format!("{stdout}{stderr}");
    assert!(
        seen.contains("These logs may still hold the token or the account's name")
            && seen.contains("old.log"),
        "{seen}"
    );
}

#[test]
fn a_default_check_steam_refuses_leaves_no_log_naming_the_account() {
    let (_project, script) = project(true);
    let home = faked_with(&refusing_as_given(
        "echo \"Logging in user '$account' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"",
    ));
    let old = home.path().join("apps/1000/output/old.log");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "Logging in user 'build_bot'").unwrap();
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default"],
        &[],
    );
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    assert_eq!(
        fs::read_to_string(&old).unwrap(),
        "Logging in user '[redacted]'"
    );
}

#[test]
fn a_dry_run_says_nothing_was_uploaded() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--dry-run", "--json"],
        &[],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("\"outcome\":\"dry-run\""), "{stdout}");
    assert!(
        stderr.starts_with("steamship upload, dry run\n"),
        "{stderr}"
    );
    assert!(stderr.contains("\n  build     1.4.0 "), "{stderr}");
    assert!(stderr.contains("dry run finished in"), "{stderr}");
    assert!(
        stderr.contains("\u{2713} nothing was uploaded, as asked\n"),
        "{stderr}"
    );
    let copy = fs::read_to_string(home.path().join("apps/1000/app_build.vdf")).unwrap();
    assert!(copy.contains("\t\"Preview\"\t\t\"1\"\n"), "{copy}");
}

#[test]
fn a_refused_build_names_every_reason_and_where_the_logs_are_and_exits_1() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, _, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_FAIL", "1")],
    );
    assert_eq!(code, Some(1_i32));
    let output = home.path().join("apps").join("1000").join("output");
    assert_eq!(
        stderr,
        format!(
            "  \u{2717} Failed to initialize build on server (Access Denied)\n    \
             log  {}, and steamcmd's own log in {}\n",
            output.join("steamcmd.log").display(),
            output.join("app_build_1000.log").display()
        )
    );
    assert_no_secret_in("stderr", stderr.as_bytes());
}

/// Whether `condition` comes true within half a minute.
#[cfg(unix)]
fn soon<Condition>(mut condition: Condition) -> bool
where
    Condition: FnMut() -> bool,
{
    let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Sends `which` signal to the process `pid`, and says whether it was there to receive it.
#[cfg(unix)]
fn signal(which: &str, pid: &str) -> bool {
    run::spawn(
        run::command("kill")
            .args([which, pid])
            .stderr(Stdio::null()),
    )
    .and_then(|mut kill| kill.wait())
    .unwrap()
    .success()
}

/// SIGHUP, SIGINT and SIGTERM, the signals that end steamship from outside, as bits of the masks
/// Linux shows a process's signals in: signal n is bit n - 1.
#[cfg(target_os = "linux")]
const HANGUP: u64 = 0x1;
#[cfg(target_os = "linux")]
const INTERRUPT: u64 = 0x2;
#[cfg(target_os = "linux")]
const TERMINATE: u64 = 0x4000;
#[cfg(target_os = "linux")]
const ENDING: u64 = HANGUP | INTERRUPT | TERMINATE;

/// Which of the signals that end steamship the process `pid` catches, and which it ignores.
///
/// Looked at directly, because whether a signal sent too early meets steamship's handler or the
/// default is a race that a test only sending signals could lose either way.
#[cfg(target_os = "linux")]
fn ending_signals(pid: u32) -> (u64, u64) {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let mask = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok())
            .unwrap()
            & ENDING
    };
    (mask("SigCgt:"), mask("SigIgn:"))
}

/// steamship uploading through a steamcmd that runs until it is stopped, started through
/// `launcher` when there is one. Returned once steamcmd has started, with steamcmd's process id
/// and the folders the upload uses, which must outlive it.
#[cfg(unix)]
fn uploading(launcher: Option<&str>) -> (Child, String, tempfile::TempDir, tempfile::TempDir) {
    let home = faked_with("#!/bin/sh\necho $$ > \"$HOME/steamcmd.pid\"\nexec sleep 60\n");
    let (project, script) = project(true);
    let steamship = env!("CARGO_BIN_EXE_steamship");
    let started = run::spawn(
        run::command(launcher.unwrap_or(steamship))
            .args(launcher.map(|_| steamship))
            .envs(common::store::environment())
            .args(["upload", script.to_str().unwrap(), "--version", "1.4.0"])
            .args(["--account", "build_bot"])
            .env("STEAMSHIP_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .unwrap();
    let pid_file = home.path().join("steamcmd.pid");
    assert!(
        soon(|| fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n'))),
        "steamcmd never started"
    );
    let steamcmd = fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    (started, steamcmd, home, project)
}

/// The signal that ended `steamship`, once it has ended.
#[cfg(unix)]
fn ending(steamship: &mut Child) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;

    let mut ended = None;
    assert!(
        soon(|| {
            ended = steamship.try_wait().unwrap();
            ended.is_some()
        }),
        "steamship went on"
    );
    ended?.signal()
}

#[cfg(unix)]
#[test]
fn ctrl_c_during_an_upload_ends_steamcmd_with_steamship() {
    let (mut steamship, steamcmd, _home, _project) = uploading(None);
    #[cfg(target_os = "linux")]
    assert!(
        soon(|| ending_signals(steamship.id()) == (ENDING, 0_u64)),
        "steamship catches every signal that ends it"
    );
    assert!(signal("-INT", &steamship.id().to_string()));
    assert_eq!(ending(&mut steamship), Some(2_i32), "ended by Ctrl+C");
    assert!(
        soon(|| !signal("-0", &steamcmd)),
        "steamcmd outlived steamship"
    );
}

#[cfg(unix)]
#[test]
fn a_signal_steamship_was_started_to_ignore_stays_ignored_and_the_others_still_end_it() {
    let (mut steamship, steamcmd, _home, _project) = uploading(Some("nohup"));
    #[cfg(target_os = "linux")]
    assert!(
        soon(|| ending_signals(steamship.id()) == (INTERRUPT | TERMINATE, HANGUP)),
        "steamship catches the signals that end it but the one nohup ignores"
    );
    let pid = steamship.id().to_string();
    assert!(signal("-HUP", &pid) && signal("-TERM", &pid));
    assert_eq!(
        ending(&mut steamship),
        Some(15_i32),
        "the terminal closing went unheeded, and kill ended it"
    );
    assert!(
        soon(|| !signal("-0", &steamcmd)),
        "steamcmd outlived steamship"
    );
}

/// Commits everything in the game at `root` and pushes it to its origin's `main` alone, so that
/// `green` stays where it was.
fn pushed_ungreen(root: &Path, message: &str) {
    git(root, &["add", "--all"]);
    commit(root, message);
    git(root, &["push", "--quiet", "origin", "main"]);
}

#[test]
fn a_game_naming_a_green_branch_is_refused_unless_origin_holds_its_commit_there() {
    let (_temp, root) = pushed_game(Some("green = \"green\"\n"));
    git(&root, &["push", "--quiet", "origin", "--delete", "green"]);
    // A version of noughts is refused after the green check, so nothing is built or sent.
    let ran = |args: &[&str]| {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        (stdout, failure(&stderr).to_owned())
    };
    let (_, why) = ran(&["upload"]);
    assert!(
        why.contains(
            "is not on origin's green branch (origin has no green branch). Upload it once your \
             tests have moved green to it"
        ),
        "{why}"
    );

    pushed(&root, "Green again");
    pushed_ungreen(&root, "Ahead of green");
    let (_, ahead_why) = ran(&["upload"]);
    assert!(
        ahead_why.contains("is not on origin's green branch (green is at "),
        "{ahead_why}"
    );

    let (allowed_out, allowed_why) = ran(&["upload", "--allow-ungreen", "--version", "0.0.0"]);
    assert!(
        allowed_out.contains("not on origin's green, uploaded as --allow-ungreen asks"),
        "{allowed_out}"
    );
    assert!(
        !allowed_why.contains("origin's green branch"),
        "{allowed_why}"
    );

    let (preview_out, preview_why) = ran(&["upload", "--dry-run", "--version", "0.0.0"]);
    assert!(
        !format!("{preview_out}{preview_why}").contains("origin's green"),
        "a preview uploads nothing and is not held to green: {preview_out}{preview_why}"
    );

    pushed(&root, "Green holds it");
    let (held_out, _) = ran(&["upload", "--version", "0.0.0"]);
    assert!(held_out.contains(", on origin's green"), "{held_out}");
}

#[test]
fn a_game_naming_no_green_branch_is_not_checked_against_one() {
    let (_temp, root) = pushed_game(None);
    git(&root, &["push", "--quiet", "origin", "--delete", "green"]);
    // The version is refused where the green check would otherwise have come first.
    let (code, stdout, stderr) = in_game(&root, &["upload", "--version", "0.0.0"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        !format!("{stdout}{stderr}").contains("origin's green"),
        "{stdout}{stderr}"
    );
    assert!(
        failure(&stderr).contains("0.0.0"),
        "refused for its version: {stderr}"
    );
}

#[test]
fn a_game_checks_the_branch_its_file_names_not_one_called_green() {
    let (_temp, root) = pushed_game(Some("green = \"passed\"\n"));
    let ran = |args: &[&str]| {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        (stdout, failure(&stderr).to_owned())
    };
    // `green` holds the commit, `passed` does not exist.
    let (_, why) = ran(&["upload", "--version", "0.0.0"]);
    assert!(
        why.contains("is not on origin's passed branch (origin has no passed branch)"),
        "{why}"
    );
    git(
        &root,
        &["push", "--quiet", "origin", "main:refs/heads/passed"],
    );
    let (held_out, _) = ran(&["upload", "--version", "0.0.0"]);
    assert!(held_out.contains(", on origin's passed"), "{held_out}");
}

#[test]
fn a_released_game_is_held_to_green_too() {
    let (_temp, root) = pushed_game(Some("released = true\ngreen = \"green\"\n"));
    git(&root, &["push", "--quiet", "origin", "--delete", "green"]);
    let (code, shown, why) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{shown}{why}");
    assert!(
        failure(&why).contains("is not on origin's green branch"),
        "{why}"
    );
}

#[test]
fn a_demo_is_held_to_green_too() {
    let (_temp, root) = pushed_game(Some("green = \"green\"\n[demo]\nreleased = true\n"));
    git(&root, &["push", "--quiet", "origin", "--delete", "green"]);
    let (code, shown, why) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(code, Some(2_i32), "{shown}{why}");
    assert!(
        failure(&why).contains("is not on origin's green branch"),
        "{why}"
    );
}

#[test]
fn an_upload_in_a_game_ships_only_its_pushed_commit_against_its_settings_snapshot() {
    let (_temp, root) = pushed_game(None);
    let refused = |said: &str| {
        let (exit, stdout, stderr) = in_game(&root, &["upload"]);
        assert_eq!(exit, Some(2_i32), "{said}: {stdout}{stderr}");
        assert!(failure(&stderr).contains(said), "{said}: {stderr}");
    };

    fs::write(root.join("stray.txt"), "not committed").unwrap();
    fs::write(root.join(".gitignore"), "export/\nchanged/\n").unwrap();
    refused("the game's folder holds what its commit does not (.gitignore, stray.txt)");
    fs::remove_file(root.join("stray.txt")).unwrap();
    git(&root, &["checkout", "--quiet", "--", ".gitignore"]);

    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "Not pushed"],
    );
    refused(", and origin's main is at ");
    git(&root, &["push", "--quiet", "origin", "main"]);

    fs::remove_file(root.join("steam").join("settings.vdf")).unwrap();
    pushed(&root, "No snapshot");
    refused(
        "settings.vdf is missing. Run steamship settings --save in the game, and commit the file",
    );

    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"Tidal Mill\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "A spaced install folder");
    refused("the install folder is \"Tidal Mill\", and steamship takes");

    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "The install folder as it should be");
    let (unbuilt, stdout, stderr) = in_game(&root, &["upload", "--version", "0.3.2-rc"]);
    assert!(stdout.contains("  version   0.3.2-rc\n"), "{stdout}");
    assert!(
        stdout
            .contains("  build     none named in steamship.toml, so the content goes up as it is"),
        "{stdout}"
    );
    assert_eq!(unbuilt, Some(2_i32), "no build account: {stderr}");
}

#[test]
fn an_upload_in_a_game_goes_up_as_the_newest_release_changes_md_heads_and_never_as_noughts() {
    let failing = "build = [\"git\", \"rev-parse\", \"--verify\", \"--quiet\", \"none\"]\n";
    let (_temp, root) = pushed_game(Some(failing));
    fs::write(
        root.join("project.godot"),
        "[application]\nconfig/name=\"Inn\"\nconfig/version=\"0.0.0\"\n",
    )
    .unwrap();
    pushed(&root, "Keep the release in the changelog");
    let stated = in_game(&root, &["upload"]);
    assert_eq!(stated.0, Some(2_i32), "{}{}", stated.1, stated.2);
    assert!(
        failure(&stated.2).ends_with(
            "project.godot's config/version is 0.0.0, which names no release. Head the newest \
             release in CHANGES.md as `## <version>`, or give it with --version"
        ),
        "{}",
        stated.2
    );
    let given = in_game(&root, &["upload", "--version", "0.0.0"]);
    assert_eq!(given.0, Some(2_i32), "{}{}", given.1, given.2);
    assert_eq!(
        failure(&given.2),
        "the version 0.0.0 is noughts alone, which names no release. Give the release going up \
         with --version"
    );
    fs::write(
        root.join("CHANGES.md"),
        "# Changes\n\n## 0.4.0, 2026-10-31\n",
    )
    .unwrap();
    pushed(&root, "Release 0.4.0");
    let headed = in_game(&root, &["upload"]);
    assert_eq!(headed.0, Some(1_i32), "{}{}", headed.1, headed.2);
    assert!(headed.1.contains("  version   0.4.0\n"), "{}", headed.1);
}

#[test]
fn an_upload_in_a_game_refuses_a_build_that_fails() {
    let failing = "build = [\"git\", \"rev-parse\", \"--verify\", \"--quiet\", \"none\"]\n";
    let (_temp, root) = pushed_game(Some(failing));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("git rev-parse --verify --quiet none failed after "),
        "{stderr}"
    );
    assert!(stdout.contains("  version   0.3.1\n"), "{stdout}");
}

#[test]
fn an_upload_in_a_game_refuses_a_build_that_changes_the_tree() {
    let leaving = "build = [\"git\", \"init\", \"--quiet\", \"made\"]\n";
    let (_temp, root) = pushed_game(Some(leaving));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("the game's folder holds what its commit does not (made/)"),
        "{stderr}"
    );
}

/// A build, as `steamship.toml` names it, that writes the game's one file of content again.
const BUILDING: &str =
    "build = [\"git\", \"config\", \"--file\", \"export/game.txt\", \"game.built\", \"now\"]\n";

#[test]
fn an_upload_in_a_game_refuses_a_file_its_build_did_not_write() {
    let (_temp, root) = pushed_game(Some("build = [\"git\", \"--version\"]\n"));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let stale = Path::new("export").join("game.txt");
    assert_eq!(
        failure(&stderr),
        format!(
            "1 file the depots upload was not written by git --version, so is left over from an \
             earlier build: {}. Have the build empty the folders it writes first, or remove what \
             it left",
            stale.display()
        )
    );

    fs::write(root.join("export").join("notes.txt"), "kept by hand").unwrap();
    fs::write(root.join("export").join("old.txt"), "an earlier export").unwrap();
    fs::write(root.join("steamship.toml"), BUILDING).unwrap();
    pushed(&root, "A build that writes the game");
    // Older than the build by more than any file system's clock is coarse, as an export from an
    // earlier build is.
    thread::sleep(Duration::from_millis(50));
    let (rebuilt, said, why) = in_game(&root, &["upload"]);
    assert_eq!(rebuilt, Some(2_i32), "{said}{why}");
    assert!(
        failure(&why).starts_with(&format!(
            "2 files the depots upload were not written by git config --file export/game.txt \
             game.built now, so are left over from an earlier build: {}, {}. ",
            Path::new("export").join("notes.txt").display(),
            Path::new("export").join("old.txt").display(),
        )),
        "{why}"
    );
}

#[test]
fn an_upload_in_a_game_builds_it_then_checks_its_settings_with_steam() {
    let (_temp, root) = pushed_game(Some(BUILDING));
    let (built, stdout, stderr) = in_game(&root, &["upload", "--dry-run"]);
    assert!(
        stdout.contains("  build     \u{2713} built in "),
        "{stdout}{stderr}"
    );
    assert!(stdout.contains("steamship settings"), "{stdout}");
    assert_eq!(
        built,
        Some(2_i32),
        "no build account to ask Steam with: {stderr}"
    );
    assert!(
        stderr.contains("no build account is named. Name it with --account or STEAMSHIP_ACCOUNT"),
        "{stderr}"
    );
}

#[test]
fn an_upload_naming_its_script_needs_the_version_it_goes_up_as() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["upload", "steam/app_build.vdf"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "steam/app_build.vdf needs a version. Give it with --version, or run steamship upload \
         with no script in the game's folder. That takes the newest release CHANGES.md heads, \
         or project.godot's version"
    );
}

#[test]
fn an_upload_in_a_game_asks_origin_and_refuses_when_origin_cannot_say_or_git_is_missing() {
    let (temp, root) = pushed_game(None);
    let nowhere = tempfile::tempdir().unwrap();
    let (missing, stdout, stderr) = in_game_with(
        &root,
        &["upload"],
        &[("PATH", nowhere.path().to_str().unwrap())],
    );
    assert_eq!(missing, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "Git is not installed, or not on the PATH, and steamship ships only a pushed commit. \
         Install Git, or put it on the PATH"
    );

    bare(temp.path(), "empty.git");
    let empty = temp.path().join("empty.git");
    git(
        &root,
        &["remote", "set-url", "origin", &empty.to_string_lossy()],
    );
    let (headless, said, why) = in_game(&root, &["upload"]);
    assert_eq!(headless, Some(2_i32), "{said}{why}");
    assert_eq!(
        failure(&why),
        "origin names no default branch, which is the one a game is shipped from"
    );

    git(&root, &["remote", "remove", "origin"]);
    let (unasked, shown, reason) = in_game(&root, &["upload"]);
    assert_eq!(unasked, Some(2_i32), "{shown}{reason}");
    let unasked_why = failure(&reason);
    assert!(
        unasked_why.starts_with("git ls-remote --symref origin HEAD failed in ")
            && unasked_why.ends_with(": fatal: 'origin' does not appear to be a git repository"),
        "Git's own first line: {reason}"
    );
}

#[test]
fn an_upload_in_a_game_says_when_its_build_cannot_be_started() {
    let (_temp, root) = pushed_game(Some("build = [\"steamship-no-such-builder\"]\n"));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("steamship-no-such-builder could not be started: "),
        "{stderr}"
    );
}

#[test]
fn a_demo_without_the_full_games_steam_input_defaults_is_refused_until_it_has_them() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/", DEMO_TYPED);
    // As a game's snapshot holds them, with a gamepad and a touch default chosen.
    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
         \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
         \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "A game with Steam Input defaults and a demo without");
    let (refused, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(refused, Some(2_i32), "{shown}{reason}");
    for line in [
        "app 1000070 has no Default Controller Configuration, and the full game, app 1000050, \
         has \"Generic Gamepad\". Choose the full game's on app 1000070's Steam Input page in \
         Steamworks",
        "app 1000070 has no Default Touch Configuration, and the full game, app 1000050, has \
         \"Mouse point and click\"",
    ] {
        assert!(reason.contains(line), "{line:?} in {reason}");
    }

    fs::write(
        root.join("steam").join("demo").join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{DEMO_TYPED}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\
             \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
             \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
    pushed(&root, "The demo with the game's Steam Input defaults");
    let (passed, building, stopped) = in_game(&root, &["upload", "--demo"]);
    assert!(!stopped.contains("Steam Input"), "{building}{stopped}");
    assert!(
        building.contains("  build     none named under [demo] in steamship.toml"),
        "on to the upload: {building}{stopped}"
    );
    assert_eq!(passed, Some(2_i32), "stopped before steamcmd: {stopped}");
}

#[test]
fn a_demo_bundling_its_own_steam_input_configuration_goes_up_only_with_its_action_manifest() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/", DEMO_TYPED);
    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
         \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
         \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}\n}\n",
    )
    .unwrap();
    // As a demo's snapshot holds them once its Steam Input page bundles a Custom Configuration.
    fs::write(
        root.join("steam").join("demo").join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{DEMO_TYPED}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\
             \t\t\"steamcontrollertemplateindex\"\t\t\"13\"\n\
             \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
             \t\t\"steamdecktouchscreen\"\t\t\"1\"\n\
             \t\t\"steaminputmanifestpath\"\t\t\"steam_input/action_manifest.vdf\"\n\
             \t\t\"steamconfigurator3rdpartynative\"\t\t\"65535\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
    pushed(&root, "A demo bundling its Steam Input configuration");
    let (stopped, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(stopped, Some(2_i32), "{shown}{reason}");
    assert!(
        reason.contains("no build account is named") && !reason.contains("Steam Input"),
        "past the demo's Steam Input defaults: {shown}{reason}"
    );

    let script = Path::new("steam").join("demo").join("app_build.vdf");
    let script = script.to_str().unwrap();
    let (refused, checking, unshipped) = in_game(&root, &["check", script]);
    assert_eq!(refused, Some(2_i32), "{checking}{unshipped}");
    assert!(
        unshipped.contains(
            "names \"steam_input/action_manifest.vdf\" as app 1000070's Steam Input action \
             manifest, and the build installs no file there. Ship the manifest at that path in \
             the install folder"
        ),
        "{checking}{unshipped}"
    );

    let manifest = root.join("export").join("steam_input");
    fs::create_dir_all(&manifest).unwrap();
    fs::write(
        manifest.join("action_manifest.vdf"),
        "\"Action Manifest\"\n{\n}\n",
    )
    .unwrap();
    let (passed, checked, said) = in_game(&root, &["check", script]);
    assert_eq!(passed, Some(0_i32), "{checked}{said}");
}

#[test]
fn a_game_in_no_repository_is_refused_saying_how_to_ship_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("game");
    fs::create_dir_all(root.join("steam")).unwrap();
    fs::write(
        root.join("steam").join("app_build.vdf"),
        r#""AppBuild" { "AppID" "480" "SetLive" "testing" "Depots" { "481" "depot.vdf" } }"#,
    )
    .unwrap();
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let refused = failure(&stderr);
    assert!(
        refused.ends_with(
            " is in no Git repository, and steamship ships a game only from a commit pushed to \
             origin. Make it a repository and push it, or name the app build script and its \
             version, as steamship upload <script> --version <version>"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("fatal:"), "{stderr}");
}

/// A program that prints how many processors it may run on.
#[cfg(windows)]
const COUNTING: [&str; 4] = [
    "powershell",
    "-NoProfile",
    "-Command",
    "[Convert]::ToString([int64][Diagnostics.Process]::GetCurrentProcess().ProcessorAffinity, 2).Replace('0', '').Length",
];
#[cfg(target_os = "linux")]
const COUNTING: [&str; 1] = ["nproc"];

/// The `cores` line an upload in a game held to `cores` shows, and how many processors its build
/// found it could run on, which the build prints.
#[cfg(any(windows, target_os = "linux"))]
fn shipped_on(cores: usize) -> (String, usize) {
    let (_temp, root) = pushed_game(Some(&format!("cores = {cores}\nbuild = {COUNTING:?}\n")));
    let (_code, stdout, stderr) = in_game(&root, &["upload"]);
    let shown = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  cores     "))
        .unwrap_or_default()
        .to_owned();
    let found: Option<usize> = stderr.lines().find_map(|line| line.trim().parse().ok());
    assert!(
        found.is_some(),
        "the build printed no count: {stdout}{stderr}"
    );
    (shown, found.unwrap_or_default())
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn shipping_a_game_holds_the_build_and_steamcmd_to_its_cores() {
    let (program, args) = COUNTING.split_first().unwrap();
    let counted = run::output(run::command(program).args(args)).unwrap();
    let machine: usize = String::from_utf8_lossy(&counted.stdout)
        .trim()
        .parse()
        .unwrap();
    let (unheld, every) = shipped_on(4096);
    assert_eq!(every, machine, "{unheld}");
    assert_eq!(
        unheld,
        format!("all {every}, within the 4096 steamcmd and the build may run on")
    );
    let (held, found) = shipped_on(1);
    assert_eq!(found, 1, "{held}");
    if every > 1 {
        assert_eq!(
            held,
            format!("1 of {every}, the most steamcmd and the build run on")
        );
    } else {
        assert_eq!(
            held,
            "all 1, within the 1 steamcmd and the build may run on"
        );
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
#[test]
fn shipping_a_game_on_macos_says_its_cores_are_not_held() {
    let (_temp, root) = pushed_game(Some("cores = 1\n"));
    let (_code, stdout, stderr) = in_game(&root, &["upload"]);
    assert!(
        stdout.contains("\n  cores     every one, as macOS holds no program to fewer\n"),
        "{stdout}{stderr}"
    );
}

/// A steamcmd for Tidal Mill Inn, still unreleased, that prints the app's info when asked for it,
/// and otherwise has Valve build the upload as `BuildID` 4242, adding a line to `builds` in the
/// home, or finish the preview the script asks for.
const INN_STEAMCMD: &str = "#!/bin/sh\n\
    case \" $* \" in *' +app_info_print '*)\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'AppID : 1000050, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
    printf '\"1000050\"\\n{\\n\\t\"common\"\\n\\t{\\n\\t\\t\"name\"\\t\\t\"Tidal Mill Inn\"\\n'\n\
    printf '\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n\\t}\\n'\n\
    printf '\\t\"config\"\\n\\t{\\n\\t\\t\"installdir\"\\t\\t\"TidalMillInn\"\\n\\t}\\n}\\n'\n\
    echo 'Unloading Steam API...OK'\n\
    case \" $* \" in *' +run_app_build '*) ;; *) exit 0;; esac;;\n\
    esac\n\
    for arg in \"$@\"; do [ \"$previous\" = +run_app_build ] && script=\"$arg\"; previous=\"$arg\"; done\n\
    finished='build (BuildID 4242).'\n\
    if grep -q '\"Preview\"[[:space:]]*\"1\"' \"$script\"; then finished='build preview.'; \
    else echo built >> \"$HOME/builds\"; fi\n\
    output=\"$HOME/apps/1000050/output\"\n\
    mkdir -p \"$output\"\n\
    echo \"Successfully finished AppID 1000050 $finished\" > \"$output/app_build_1000050.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

/// The game [`pushed_game`] makes with `toml`, without achievements, beside a home whose
/// steamcmd is [`INN_STEAMCMD`] and whose settings snapshot is Steam's own, pushed.
fn settled_inn(toml: &str) -> (tempfile::TempDir, PathBuf, tempfile::TempDir) {
    let (temp, root) = pushed_game(Some(toml));
    fs::remove_file(
        root.join("mods")
            .join("base")
            .join("data")
            .join("achievements.json"),
    )
    .unwrap();
    let home = faked_with(INN_STEAMCMD);
    saved_login(home.path());
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];

    let (saved, said, why) =
        steamship_in(&root, &["settings", "--save"], Some(home.path()), &quiet);
    assert_eq!(saved, Some(0_i32), "{said}{why}");
    pushed(&root, "The settings as Steam has them");
    let (checked, shown, reason) =
        steamship_in(&root, &["settings", "--check"], Some(home.path()), &quiet);
    assert_eq!(checked, Some(0_i32), "{shown}{reason}");
    assert!(shown.contains("Steam's settings match the file"), "{shown}");
    (temp, root, home)
}

/// `steamship upload` in the game at `root`, with a Web API that sets build 4242 live and
/// describes `testing`, and the first five requests it was sent.
fn uploaded_inn(root: &Path, home: &Path) -> (Option<i32>, String, String, Vec<String>) {
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
    ]);
    let (code, stdout, stderr) =
        steamship_in(root, &["upload"], Some(home), &web_api_variables(&host));
    let asked = if code == Some(0_i32) {
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(5)
            .collect()
    } else {
        Vec::new()
    };
    (code, stdout, stderr, asked)
}

#[test]
fn a_game_is_built_checked_uploaded_live_on_default_too_and_its_branch_described() {
    let (_temp, root, home) = settled_inn(BUILDING);
    let (code, stdout, stderr, asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    for line in [
        "  version   0.3.1\n",
        "  build     \u{2713} built in ",
        "app 1000050: BuildID 4242, set live on testing and default",
        "app 1000050: testing described as \"0.3.1\"",
        "steamship builds\n",
    ] {
        assert!(stdout.contains(line), "{line} in {stdout}{stderr}");
    }
    assert_eq!(
        fs::read_to_string(home.path().join("builds")).unwrap(),
        "built\n"
    );
    let describing = asked.last().unwrap();
    assert!(
        describing.starts_with("POST /ISteamApps/UpdateAppBranchDescription/v1/ ")
            && describing.ends_with("appid=1000050&betakey=testing&description=0.3.1"),
        "{asked:?}"
    );

    let (again, _again_asked) = web_api(vec![("200 OK", BETAS); 4]);
    let (previewed, shown_preview, preview_why) = steamship_in(
        &root,
        &["upload", "--dry-run"],
        Some(home.path()),
        &web_api_variables(&again),
    );
    assert_eq!(previewed, Some(0_i32), "{shown_preview}{preview_why}");
    assert!(
        shown_preview.contains("dry run finished in")
            && shown_preview.contains("\u{2713} nothing was uploaded, as asked\n")
            && !shown_preview.contains("steamship branch")
            && !shown_preview.contains("steamship builds"),
        "a preview ends with Valve's dry run: {shown_preview}"
    );
}

#[test]
fn a_released_game_is_set_live_on_its_branch_alone_and_the_build_is_recorded() {
    let (_temp, root, home) =
        settled_inn(&format!("released = true\ngreen = \"green\"\n{BUILDING}"));
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("app 1000050: BuildID 4242, set live on testing\n")
            && !stdout.contains("and default"),
        "{stdout}"
    );
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(4)
            .collect();
    assert!(
        !asked
            .iter()
            .any(|request| request.contains("SetAppBuildLive")),
        "a released game's default branch is never set: {asked:?}"
    );
    let kept = ledger::load(home.path(), 1_000_050).unwrap();
    assert_eq!(
        kept,
        vec![ledger::Entry {
            app: 1_000_050,
            branch: "testing".to_owned(),
            build_id: 4242,
            commit: head(&root),
            green: true,
            rollback: false,
        }]
    );
}

#[test]
fn an_unreleased_game_naming_no_green_branch_records_its_build_on_testing_and_default() {
    let (_temp, root, home) = settled_inn(BUILDING);
    let (code, stdout, stderr, _asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(!stdout.contains("  green "), "{stdout}");
    let kept = ledger::load(home.path(), 1_000_050).unwrap();
    let branches: Vec<(&str, u64, bool)> = kept
        .iter()
        .map(|each| (each.branch.as_str(), each.build_id, each.green))
        .collect();
    assert_eq!(
        branches,
        [("testing", 4242, false), ("default", 4242, false)]
    );
}

#[test]
fn a_game_whose_steamship_toml_names_a_description_has_its_branch_described_so() {
    let (_temp, root, home) = settled_inn(&format!("description = \"nightly\"\n{BUILDING}"));
    let (code, stdout, stderr, asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("app 1000050: testing described as \"nightly\""),
        "{stdout}{stderr}"
    );
    assert!(
        asked
            .last()
            .unwrap()
            .ends_with("appid=1000050&betakey=testing&description=nightly"),
        "{asked:?}"
    );
}

#[test]
fn a_games_first_upload_to_an_app_with_no_branch_yet_succeeds_and_says_how_to_describe_it() {
    let (_temp, root, home) = settled_inn(&format!("description = \"testing\"\n{BUILDING}"));
    let (host, requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        (
            "200 OK",
            r#"{"applist": {"apps": {"app": [
                {"appid": 1000050, "app_type": "game", "app_name": "Tidal Mill Inn"}
            ]}}}"#,
        ),
        ("200 OK", SET_LIVE),
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    for line in [
        "app 1000050: BuildID 4242, set live on default",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000050 --build 4242 --branch testing",
        "once it is made, describe testing with steamship branch 1000050 testing --description \
         \"testing\"",
        "steamship builds\n",
    ] {
        assert!(shown.contains(line), "{line:?} in {shown}");
    }
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(5)
            .collect();
    assert!(
        !asked
            .iter()
            .any(|request| request.contains("UpdateAppBranchDescription")),
        "a branch Steam does not have yet is not described: {asked:?}"
    );
}

/// Whether a line of `stderr` opens with the cross of a failure and holds `text`.
fn crossed(stderr: &str, text: &str) -> bool {
    stderr
        .lines()
        .any(|line| line.trim_start().starts_with('\u{2717}') && line.contains(text))
}

#[test]
fn a_game_whose_branch_steam_will_not_describe_is_uploaded_all_the_same() {
    let (_temp, root, home) = settled_inn(&format!("released = true\n{BUILDING}"));
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload", "--json"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(
        stdout.contains("\"outcome\":\"uploaded\"")
            && stdout.contains("\"build_id\":4242")
            && !stdout.contains("\"error\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("could not describe testing: Steam answered HTTP 500"),
        "{stderr}"
    );
    assert!(!crossed(&stderr, "Steam answered HTTP 500"), "{stderr}");
    assert!(
        stderr.contains("steamship branch 1000050 testing --description \"0.3.1\""),
        "{stderr}"
    );
    assert!(
        !stderr.contains("steamship builds") && !stderr.contains("could not list"),
        "a failed description ends the run before the builds list: {stderr}"
    );
}

#[test]
fn a_branch_gone_once_the_build_is_live_is_a_note() {
    let (_temp, root, home) = settled_inn(&format!("released = true\n{BUILDING}"));
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        (
            "200 OK",
            r#"{"response": {"result": 1, "betas": {"public": {"BuildID": 7}}}}"#,
        ),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload", "--json"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"outcome\":\"uploaded\"") && !stdout.contains("\"error\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("could not describe testing: app 1000050 has no branch \"testing\""),
        "{stderr}"
    );
    assert!(!crossed(&stderr, "has no branch \"testing\""), "{stderr}");
    assert!(
        stderr.contains("steamship branch 1000050 testing --description \"0.3.1\""),
        "{stderr}"
    );
}

#[test]
fn a_builds_list_steam_will_not_give_is_a_note() {
    let (_temp, root, home) = settled_inn(BUILDING);
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("testing described as"), "{stdout}");
    assert!(
        stderr.contains("could not list the builds of app 1000050: Steam answered HTTP 500"),
        "{stderr}"
    );
    assert!(!crossed(&stderr, "Steam answered HTTP 500"), "{stderr}");
    assert!(!stderr.contains("--description"), "{stderr}");
}

#[test]
fn a_game_whose_script_names_its_branch_in_another_case_has_it_described() {
    let (_temp, root, home) = settled_inn(BUILDING);
    let script = root.join("steam").join("app_build.vdf");
    let written = fs::read_to_string(&script).unwrap();
    fs::write(
        &script,
        written.replace("\"SetLive\" \"testing\"", "\"SetLive\" \"Testing\""),
    )
    .unwrap();
    pushed(&root, "The script names Testing");
    let (code, stdout, stderr, asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("testing described as \"0.3.1\""),
        "{stdout}{stderr}"
    );
    assert!(
        asked
            .last()
            .unwrap()
            .ends_with("betakey=testing&description=0.3.1"),
        "{asked:?}"
    );
}

/// [`BUILT`] with the shell line `then` run last.
fn built_then(then: &str) -> String {
    format!("{BUILT}{then}\n")
}

#[test]
fn a_steamcmd_altered_after_the_run_exits_5_once_a_build_is_up_and_4_otherwise() {
    let altering = "echo >> \"$0\"";
    let up = faked_with(&built_then(altering));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        up.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(stdout.contains("BuildID 4242"), "{stdout}");
    assert!(
        stderr.contains("Steam holds a build of app 1000, so a second upload would make another"),
        "{stderr}"
    );
    assert!(
        stderr.contains("list it with steamship builds 1000"),
        "{stderr}"
    );
    let fresh = faked_with(&format!(
        "#!/bin/sh\necho 'Logging in user build_bot to Steam Public...OK'\n{altering}\nexit 1\n"
    ));
    let (earlier, said, errors) = upload(
        &script,
        fresh.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(earlier, Some(4_i32), "{said}{errors}");
    assert!(!errors.contains("Steam holds"), "{errors}");
}

#[test]
fn a_build_steam_kept_unnamed_before_steamcmd_altered_itself_exits_5() {
    let home = faked_with(&COMMIT_REFUSED.replace("exit 6\n", "echo >> \"$0\"\nexit 6\n"));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(stderr.contains("steamship builds 1000"), "{stderr}");
}

#[test]
fn a_dry_run_whose_steamcmd_altered_itself_exits_4() {
    let home = leaking_with("echo >> \"$0\"");
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--dry-run"],
        &[],
    );
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    assert!(stdout.contains("dry run finished"), "{stdout}");
    assert!(!format!("{stdout}{stderr}").contains("Steam holds"));
}

/// The folder a [`leaking`] home's upload writes to.
fn output_of(home: &Path) -> PathBuf {
    home.join("apps").join("1000").join("output")
}

#[test]
fn a_console_that_cannot_be_saved_leaves_every_exit_code_as_it_was() {
    let home = leaking();
    let blocked = output_of(home.path()).join("steamcmd.log");
    fs::create_dir_all(&blocked).unwrap();
    fs::write(blocked.join("kept"), "in the way").unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(shown.contains("BuildID 4242"), "{shown}");
    for said in [
        "app_build_1000.log",
        "could not be written to",
        "The earlier console could not be removed",
    ] {
        assert!(shown.contains(said), "{said:?} in {shown}");
    }
    for (variable, expected) in [
        ("STEAMSHIP_FAKE_FAIL", 1_i32),
        ("STEAMSHIP_FAKE_LOGGED_OUT", 3_i32),
    ] {
        let (failed, said, errors) = upload(
            &script,
            home.path(),
            &["--version", "1.4.0"],
            &[(variable, "1")],
        );
        assert_eq!(failed, Some(expected), "{variable}: {said}{errors}");
    }
}

#[test]
fn a_console_that_cannot_be_saved_says_nothing_of_an_earlier_one_when_there_was_none() {
    let home = leaking_with("mkdir -p \"$output/steamcmd.log\"");
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(shown.contains("could not be written to"), "{shown}");
    assert!(!shown.contains("could not be removed"), "{shown}");
}

#[test]
fn a_failed_save_in_actions_shows_no_earlier_console() {
    let home = leaking_with("mkdir -p \"$output/steamcmd.log\"");
    fs::create_dir_all(output_of(home.path())).unwrap();
    fs::write(
        output_of(home.path()).join("steamcmd.log"),
        "an earlier console",
    )
    .unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_FAIL", "1"), ("GITHUB_ACTIONS", "true")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(stderr.contains("could not be written to"), "{stderr}");
    assert!(stdout.contains("::group::app_build_1000.log"), "{stdout}");
    assert!(!stdout.contains("::group::steamcmd's console"), "{stdout}");
    assert!(!stdout.contains("an earlier console"), "{stdout}");
}

#[cfg(windows)]
#[test]
fn an_earlier_console_held_open_is_not_shown_as_this_runs() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt as _;

    let home = leaking();
    let console = output_of(home.path()).join("steamcmd.log");
    fs::create_dir_all(output_of(home.path())).unwrap();
    fs::write(&console, "an earlier console").unwrap();
    // Shared for reading only, so that it can neither be removed nor written.
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&console)
        .unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_FAIL", "1"), ("GITHUB_ACTIONS", "true")],
    );
    drop(held);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("The earlier console could not be removed"),
        "{stderr}"
    );
    assert!(!stdout.contains("::group::steamcmd's console"), "{stdout}");
    assert!(!stdout.contains("an earlier console"), "{stdout}");
}

/// The shell line that leaves the login unreadable on every system: `config.vdf` becomes a folder.
const UNREADABLE: &str =
    "rm -f \"$HOME/Steam/config/config.vdf\"; mkdir -p \"$HOME/Steam/config/config.vdf\"";

#[test]
fn a_refused_login_never_shows_a_secret_of_its_reason_when_the_login_is_unreadable_after() {
    let secret = "Expired Login Auth Code";
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         mkdir -p \"$HOME/apps/1000/output\"\n\
         echo \"Logging in user 'build_bot' to Steam Public...FAILED ({secret})\"\n\
         {UNREADABLE}\n\
         exit 5\n"
    ));
    let local = home
        .path()
        .join("Steam/userdata/12345/config/localconfig.vdf");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(
        local,
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{secret}\" }} }}\n"),
    )
    .unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert!(!stdout.contains(secret) && !stderr.contains(secret));
}

#[test]
fn a_login_unreadable_after_the_run_still_sets_default_live_and_shows_none_of_steamcmds_lines() {
    let home = faked_with(&format!("{}{UNREADABLE}\n", released_as("unavailable")));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("\"default\":true"), "{stdout}");
    assert!(
        stderr.contains("app 1000: BuildID 4242, set live on testing and default"),
        "{stderr}"
    );
    let shown = format!("{stdout}{stderr}");
    for line in ["unavailable", "Logging in user", "AppID : 1000"] {
        assert!(!shown.contains(line), "{line:?} in {shown}");
    }
}

/// A steamcmd that ends leaving its login file unreadable, so that the redactor cannot be read
/// again.
#[cfg(target_os = "linux")]
fn unreadable_after(fake: &str) -> String {
    fake.replace(
        "exit 6\n",
        "chmod 000 \"$HOME/Steam/config/config.vdf\"\nexit 6\n",
    )
}

#[cfg(target_os = "linux")]
#[test]
fn a_login_unreadable_after_the_run_shows_no_line_of_steamcmds_and_keeps_the_outcome() {
    let cases = [
        (
            built_then("chmod 000 \"$HOME/Steam/config/config.vdf\""),
            0_i32,
            "BuildID 4242",
        ),
        (
            unreadable_after(BUILT_THEN_FAILED),
            5_i32,
            "steamship promote 1000 --build 777 --branch testing",
        ),
    ];
    for (fake, expected, said) in cases {
        let home = faked_with(&fake);
        let (_project, script) = project(true);
        let (code, stdout, stderr) = upload(
            &script,
            home.path(),
            &["--version", "1.4.0"],
            &[("STEAMSHIP_LOGIN", &packed_login())],
        );
        let shown = format!("{stdout}{stderr}");
        assert_eq!(code, Some(expected), "{shown}");
        assert!(shown.contains(said), "{said:?} in {shown}");
        let output = output_of(home.path());
        let left = fs::read_dir(&output).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "log"))
                .count()
        });
        assert_eq!(left, 0, "no log is left in {}", output.display());
        assert!(shown.contains("none kept"), "{shown}");
        assert!(shown.contains(&output.display().to_string()), "{shown}");
        for line in ["Logging in user", "forged", "Failed to set build live"] {
            assert!(!shown.contains(line), "{line:?} in {shown}");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_refused_login_names_its_reason_and_never_the_account_once_the_login_is_unreadable() {
    let home = faked_with(
        "#!/bin/sh\n\
         mkdir -p \"$HOME/apps/1000/output\"\n\
         echo \"Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)\"\n\
         chmod 000 \"$HOME/Steam/config/config.vdf\"\n\
         exit 5\n",
    );
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(3_i32), "{shown}");
    assert!(
        shown.contains("Steam refused the saved login (Expired Login Auth Code)"),
        "{shown}"
    );
    assert!(!stdout.contains("build_bot"), "{stdout}");
    assert!(!stderr.contains("build_bot"), "{stderr}");
    assert!(!output_of(home.path()).join("steamcmd.log").exists());
}

#[test]
fn a_reason_that_holds_a_secret_is_printed_redacted() {
    let home = leaking();
    let secret = "Failed to initialize build on server";
    fs::write(
        home.path()
            .join("Steam/userdata/12345/config/localconfig.vdf"),
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{secret}\" }} }}\n"),
    )
    .unwrap();
    let (_project, script) = project(true);
    for json in [false, true] {
        let mut extra = vec!["--version", "1.4.0"];
        extra.extend(json.then_some("--json"));
        let (code, stdout, stderr) = upload(
            &script,
            home.path(),
            &extra,
            &[("STEAMSHIP_FAKE_FAIL", "1")],
        );
        assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
        assert!(stderr.contains("[redacted] (Access Denied)"), "{stderr}");
        assert!(!stdout.contains(secret) && !stderr.contains(secret));
    }
}
