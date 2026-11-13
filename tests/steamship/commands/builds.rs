//! `steamship builds`.

use std::fs;
use std::iter;
use std::path::Path;
use std::time::Duration;

use steamship::keychain;
use steamship::platform::Platform;
use steamship::steamcmd;
use steamship::webapi::STAND_IN;

use crate::common;
use crate::{
    APPS, BETAS, BUILDS, KEY, SET_LIVE, Session, failure, faked_with, forgetting_home, in_game,
    in_game_with, login_with_key, pushed_game, saved_login, steamship, steamship_in, summary_of,
    web_api, web_api_variables,
};

#[test]
fn builds_with_no_key_set_or_kept_and_no_one_to_ask_says_how_to_keep_one_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(stdout.contains("  app       1000010\n"), "{stdout}");
    assert_eq!(failure(&stderr), "no Web API key");
    assert!(
        stderr.ends_with(
            "run steamship login --web-api-key-only at a terminal to keep one, or set \
             STEAMSHIP_WEB_API_KEY\n"
        ),
        "{stderr}"
    );
}

#[test]
fn builds_at_a_terminal_asks_for_the_key_then_offers_to_keep_it() {
    let steamship_program = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship_program} builds 1000010");
    #[cfg(unix)]
    let line = format!("'{steamship_program}' builds 1000010");
    for (answer, shown) in [("\r", "\u{2713} kept in"), ("n\r", "not kept")] {
        let home = forgetting_home();
        let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
        let mut session = Session::start_with(
            &line,
            home.path(),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1"), (STAND_IN, &host)],
        );
        session.wait_for("api key");
        session.type_in(&format!("{KEY}\r"));
        session.wait_for("keep it");
        session.type_in(answer);
        session.wait_for(&format!("  api key   {shown}"));
        let _: Option<i32> = session.end().wait().unwrap();
        let seen = session.seen();
        assert!(seen.contains("1 branch and 1 build"), "{seen}");
        assert!(!seen.contains(KEY), "the key was shown: {seen}");
        let (_, status, _) = steamship(&["status"], Some(home.path()), &[]);
        let kept = status.contains(&format!("  api key   kept in {}\n", keychain::STORE));
        assert_eq!(kept, answer == "\r", "{answer:?}: {status}\nshown: {seen}");
    }
}

#[test]
fn builds_and_promote_use_the_key_set_and_say_what_steam_answered() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
        ("200 OK", SET_LIVE),
    ]);
    let set = web_api_variables(&host);
    let (built, listed, said) = steamship(&["builds", "1000010"], Some(home.path()), &set);
    assert_eq!((built, said.as_str()), (Some(0_i32), ""), "{listed}");
    assert!(
        listed.contains("  api key   from STEAMSHIP_WEB_API_KEY\n"),
        "{listed}"
    );
    assert!(listed.contains("1 branch and 1 build"), "{listed}");
    assert!(
        !listed.contains("keep it"),
        "a key set is not offered for keeping: {listed}"
    );
    let (promoted, live, stderr) = steamship(
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
        Some(home.path()),
        &set,
    );
    assert_eq!((promoted, stderr.as_str()), (Some(0_i32), ""), "{live}");
    assert!(live.contains("  steam     \u{2713} set live\n"), "{live}");
    assert!(
        live.contains("app 1000010: BuildID 7 live on testing"),
        "{live}"
    );
    assert!(!format!("{listed}{live}").contains(KEY), "never shown");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000010&buildid=7&betakey=testing"),
        "{setting}"
    );
}

/// `GetAppBetas` and `GetAppBuilds` as Steam answered them for Falcon Ridge Overseer, the builds
/// trimmed to two and their depots left out.
const SAMPLE_BETAS: &str = r#"{"response":{"betas":{"public":{"BuildID":20000001,"Description":"Public default branch","ReqPassword":0,"ReqLocalCS":0},"testing":{"BuildID":20000003,"Description":"0.1.0 (a1b2c3d)","ReqPassword":1,"ReqLocalCS":0}},"result":1,"message":""}}"#;
const SAMPLE_BUILDS: &str = r#"{"response":{"builds":{"20000003":{"BuildID":20000003,"CreationTime":1791910891,"Description":"0.1.0 a1b2c3d4e5f6","AccountIDCreator":123456789},"20000002":{"BuildID":20000002,"CreationTime":1791906331,"Description":"0.1.0 f6e5d4c3b2a1","AccountIDCreator":123456789}},"result":1,"message":""}}"#;

#[test]
fn builds_json_prints_the_branches_and_builds_alone_on_standard_output() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", SAMPLE_BETAS), ("200 OK", SAMPLE_BUILDS)]);
    let set = web_api_variables(&host);
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010", "--count", "2", "--json"],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        summary_of(&stdout),
        serde_json::json!({
            "app": 1_000_010_u32,
            "branches": [
                {"name": "default", "build_id": 20_000_001_u64,
                 "description": "Public default branch", "password": false},
                {"name": "testing", "build_id": 20_000_003_u64,
                 "description": "0.1.0 (a1b2c3d)", "password": true},
            ],
            "builds": [
                {"build_id": 20_000_003_u64, "created": "2026-10-13T17:01:31Z",
                 "description": "0.1.0 a1b2c3d4e5f6", "version": "0.1.0",
                 "commit": "a1b2c3d4e5f6", "live_on": ["testing"]},
                {"build_id": 20_000_002_u64, "created": "2026-10-13T15:45:31Z",
                 "description": "0.1.0 f6e5d4c3b2a1", "version": "0.1.0",
                 "commit": "f6e5d4c3b2a1", "live_on": []},
            ],
        })
    );
    assert!(stderr.contains("2 branches and 2 builds"), "{stderr}");
    assert!(!stderr.contains(KEY), "never shown");
}

/// A home whose saved login is a folder, so that steamcmd's login files cannot be read.
fn home_with_unreadable_login() -> tempfile::TempDir {
    let home = crate::faked();
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(&config).unwrap();
    home
}

#[test]
fn with_unreadable_login_files_steamships_own_advice_for_a_failed_ask_is_still_given() {
    let home = home_with_unreadable_login();
    let (host, _requests) = web_api(vec![("500 Internal Server Error", ""), ("200 OK", APPS)]);
    let (exit, stdout, stderr) = steamship(
        &["builds", "1000030", "--json"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(exit, Some(1_i32), "{stdout}{stderr}");
    let said = summary_of(&stdout);
    assert_eq!(
        said.pointer("/error/message")
            .and_then(|text| text.as_str()),
        Some(
            "Steam answered HTTP 500: this key holds app 1000030, and Steam answers so while no \
             build is live on its default branch. Set one live there in Steamworks under \
             SteamPipe, Builds, then ask again"
        ),
        "{said}"
    );
    assert!(!stderr.contains("withheld"), "{stderr}");
}

#[test]
fn with_unreadable_login_files_a_refused_key_still_says_how_and_steams_words_are_withheld() {
    let home = home_with_unreadable_login();
    let refusing = web_api(vec![("403 Forbidden", "")]).0;
    let (exit, stdout, stderr) = steamship(
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
        Some(home.path()),
        &web_api_variables(&refusing),
    );
    assert_eq!(exit, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with(
            "Steam refused the Web API key. It must be the publisher key of a group that holds \
             the app"
        ),
        "{stderr}"
    );

    let answer = r#"{"response": {"message": "words Steam gave"}}"#.to_owned();
    let (host, _requests) = web_api(vec![("500 Internal Server Error", answer)]);
    let (code, said, why) = steamship(
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(1_i32), "{said}{why}");
    assert!(
        failure(&why)
            .starts_with("Steam answered HTTP 500: withheld, because steamcmd's login files"),
        "{why}"
    );
    assert!(!why.contains("words Steam gave"), "{why}");
}

#[test]
fn with_unreadable_login_files_the_key_check_still_says_a_refused_key_needs_the_publisher_key() {
    let home = home_with_unreadable_login();
    let (host, _requests) = web_api(vec![("403 Forbidden", "")]);
    let (code, stdout, stderr) = login_with_key(
        home.path(),
        &host,
        KEY,
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains(
            "Steam refused the Web API key. It must be the publisher key of a group that holds \
             the app"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("withheld"), "{stderr}");
}

#[test]
fn builds_json_names_an_app_with_no_build_live_apart_from_steam_failing() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
    ]);
    let set = web_api_variables(&host);
    for (app, code) in [("1000030", "no-build-live"), ("480", "http-error")] {
        let (exit, stdout, stderr) = steamship(&["builds", app, "--json"], Some(home.path()), &set);
        assert_eq!(exit, Some(1_i32), "{stdout}{stderr}");
        let said = summary_of(&stdout);
        let app_id: u32 = app.parse().unwrap();
        assert_eq!(said.get("app"), Some(&app_id.into()), "{said}");
        assert_eq!(said.pointer("/error/code"), Some(&code.into()), "{said}");
        let message = said
            .pointer("/error/message")
            .and_then(|text| text.as_str());
        assert!(
            message.is_some_and(|text| text.starts_with("Steam answered HTTP 500")),
            "{said}"
        );
    }
}

#[test]
fn builds_json_with_no_key_asks_for_none_and_prints_no_json() {
    let _store = common::store::lock();
    let home = tempfile::tempdir().unwrap();
    let steamship_program = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship_program} builds 1000010 --json");
    #[cfg(unix)]
    let line = format!("'{steamship_program}' builds 1000010 --json");
    let mut session = Session::start(&line, home.path());
    session.wait_for("no Web API key");
    assert_eq!(session.end().wait().unwrap(), Some(2_i32));
    let seen = session.seen();
    assert!(!seen.contains('{'), "{seen}");
}

#[test]
fn builds_and_promote_that_steam_answers_with_an_error_say_its_status_and_exit_1() {
    let home = tempfile::tempdir().unwrap();
    // The second answer is for what `builds` asks after a 500: which apps the key holds.
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let set = web_api_variables(&host);
    for args in [
        &["builds", "1000010"][..],
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
    ] {
        let (code, stdout, stderr) = steamship(args, Some(home.path()), &set);
        assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
        assert!(
            stdout.contains("  steam     \u{2717} HTTP 500\n"),
            "{stdout}"
        );
        assert!(!stdout.contains("live on"), "{stdout}");
    }

    let refusing = web_api(vec![("403 Forbidden", "")]).0;
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &web_api_variables(&refusing),
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  steam     \u{2717} key refused\n"),
        "{stdout}"
    );
    assert!(
        failure(&stderr).starts_with("Steam refused the Web API key. "),
        "{stderr}"
    );
}

#[test]
fn a_500_about_an_app_the_key_holds_is_said_to_be_an_app_with_no_build_yet() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
    ]);
    let set = web_api_variables(&host);
    // As Steam answered for Nocturne's app while no build was live on its default branch.
    let (code, stdout, stderr) = steamship(&["builds", "1000030"], Some(home.path()), &set);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(stderr.contains("Steam answered HTTP 500"), "{stderr}");
    assert!(
        stderr.contains(
            "this key holds app 1000030, and Steam answers so while no build is live on its \
             default branch"
        ),
        "{stderr}"
    );
    // The same answer about an app the key does not hold is left as it came.
    let (other, _, alone) = steamship(&["builds", "480"], Some(home.path()), &set);
    assert_eq!(other, Some(1_i32), "{alone}");
    assert!(alone.contains("Steam answered HTTP 500"), "{alone}");
    assert!(!alone.contains("this key holds"), "{alone}");
}

#[test]
fn a_web_api_key_that_is_not_one_is_refused_without_repeating_it() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", "not_a_key_1234")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_WEB_API_KEY does not hold a publisher Web API key, which is 32 hexadecimal \
         digits"
    );
    assert!(!format!("{stdout}{stderr}").contains("not_a_key"));
}

#[test]
fn a_web_api_key_that_is_not_one_is_refused_beside_a_kept_key() {
    let home = forgetting_home();
    let (keeping, _asked) = web_api(vec![("200 OK", APPS)]);
    let (kept, kept_said, kept_why) = login_with_key(home.path(), &keeping, KEY, &[]);
    assert_eq!(kept, Some(0_i32), "{kept_said}{kept_why}");
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", "not_a_key_1234"),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_WEB_API_KEY does not hold a publisher Web API key, which is 32 hexadecimal \
         digits"
    );
    assert!(!stdout.contains("kept in"), "{stdout}");
    assert!(!format!("{stdout}{stderr}").contains("not_a_key"));
    assert!(
        requests.recv_timeout(Duration::from_secs(1)).is_err(),
        "Steam was asked"
    );
}

#[test]
fn an_app_that_is_neither_an_id_nor_a_script_is_refused_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (zero, _, said) = steamship(&["builds", "0"], Some(home.path()), &[]);
    assert_eq!(
        (zero, failure(&said)),
        (
            Some(2_i32),
            "0 is not an app ID. Give the app's ID from Steamworks"
        )
    );
    let (missing, _, stderr) = steamship(&["builds", "no_such_app.vdf"], Some(home.path()), &[]);
    assert_eq!(missing, Some(2_i32));
    assert_eq!(
        failure(&stderr),
        "refused: no_such_app.vdf: does not exist. Name the app's build script, such as \
         steam/app_build.vdf"
    );
}

#[test]
fn a_count_that_is_not_a_number_says_what_it_must_be() {
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(&["builds", "480", "--count", "x"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        stderr.contains("'x' is not a number. Give how many builds to list, such as 10"),
        "{stderr}"
    );
}

#[test]
fn a_game_pinning_another_steamship_is_refused_by_every_command_run_in_it() {
    let (_temp, root) = pushed_game(Some("steamship = \"0.0.1\"\n"));
    for args in [
        ["builds"].as_slice(),
        ["check"].as_slice(),
        ["upload"].as_slice(),
        ["achievements", "--write"].as_slice(),
    ] {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        assert!(
            failure(&stderr).ends_with(&format!(
                "pins steamship 0.0.1, and this is {}. Install 0.0.1, or change the pin",
                env!("CARGO_PKG_VERSION")
            )),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn a_steamship_toml_saying_what_steamship_does_not_take_is_refused_naming_the_file() {
    let (_temp, root) = pushed_game(Some("[workshop]\nmod = \"examples\"\n"));
    let (code, stdout, stderr) = in_game(&root, &["builds"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    // Ends with, as macOS gives the child its temporary folder through /private.
    assert!(
        failure(&stderr).ends_with(&format!(
            "{}: `workshop.mod` is no setting steamship takes. It takes `workshop.mods`, \
             `workshop.items`, `workshop.footer`, `workshop.held_until_launch`",
            Path::new("inn").join("steamship.toml").display()
        )),
        "{stderr}"
    );
}

#[test]
fn outside_a_game_a_command_needing_one_says_to_start_one_or_name_the_app() {
    let empty = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_in(
        empty.path(),
        &["builds"],
        Some(empty.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).ends_with(
            ": no steamship.toml or steam/app_build.vdf here or in a folder above. Run \
             steamship in the game's folder, run `steamship init <app ID>` to start one, or \
             name the app and its files"
        ),
        "{stderr}"
    );
    // An app named does not stand for a file not named: the game's own is still looked for.
    for args in [
        ["settings"].as_slice(),
        ["settings", "--check"].as_slice(),
        ["settings", "480", "--check"].as_slice(),
        ["settings", "480", "--save"].as_slice(),
        ["achievements", "480", "--check"].as_slice(),
        ["leaderboards", "--check"].as_slice(),
        ["leaderboards", "480", "--check"].as_slice(),
        ["rich-presence"].as_slice(),
        ["rich-presence", "480"].as_slice(),
        ["promote", "--build", "7", "--branch", "testing"].as_slice(),
        ["check"].as_slice(),
        ["upload"].as_slice(),
        ["text"].as_slice(),
        ["workshop"].as_slice(),
    ] {
        let (refused, said, why) = steamship_in(
            empty.path(),
            args,
            Some(empty.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(refused, Some(2_i32), "{args:?}: {said}{why}");
        assert!(
            failure(&why).starts_with("there is no game in "),
            "{args:?}: {why}"
        );
    }
}

#[test]
fn a_games_builds_settings_and_promotion_are_asked_for_its_own_app() {
    let (_temp, root) = pushed_game(None);
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["builds"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(asked.contains("appid=1000050"), "{asked}");

    let (refusing, sent) = web_api(vec![("403 Forbidden", "{}"); 4]);
    let (promoted, said, why) = in_game_with(
        &root,
        &["promote", "--build", "7", "--branch", "testing"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &refusing)],
    );
    assert_eq!(promoted, Some(1_i32), "Steam refused it: {said}{why}");
    assert!(
        said.contains("  app       1000050\n  build     7\n  branch    testing\n")
            && format!("{said}{why}").contains(
                "Steam refused the Web API key. It must be the publisher key of a group that \
                 holds the app"
            ),
        "{said}{why}"
    );
    let promotion = sent.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(promotion.contains("appid=1000050"), "{promotion}");

    let steamcmd = home_recording_steamcmd();
    for args in [
        ["settings"].as_slice(),
        ["settings", "--save"].as_slice(),
        ["settings", "--check"].as_slice(),
    ] {
        let (asked_for, shown, reason) = steamship_in(
            &root,
            args,
            Some(steamcmd.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(asked_for, Some(1_i32), "{args:?}: {shown}{reason}");
        assert!(
            format!("{shown}{reason}")
                .contains("steamcmd printed no settings for app 1000050. Run the command again"),
            "{args:?}: {shown}{reason}"
        );
        let given = fs::read_to_string(steamcmd.path().join("args")).unwrap();
        assert!(
            given.contains("+app_info_print 1000050 "),
            "{args:?}: {given}"
        );
        fs::remove_file(steamcmd.path().join("args")).unwrap();
    }
}

/// A home with a saved login whose steamcmd keeps what it was started with in `args` and shows
/// no app information.
fn home_recording_steamcmd() -> tempfile::TempDir {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n",
    );
    saved_login(home.path());
    home
}
