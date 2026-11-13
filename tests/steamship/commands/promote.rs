//! `steamship promote`.

use std::fs;
use std::time::Duration;

use steamship::platform::Platform;
use steamship::steamcmd;
use steamship::webapi::STAND_IN;

use crate::{
    KEY, SECRETS, SET_LIVE, failure, faked_with, packed_login, project, released_as, saved_login,
    steamship, web_api, web_api_variables,
};

#[test]
fn a_secret_in_steams_answer_is_hidden_from_a_failed_promote() {
    let home = faked_with(&released_as("prerelease"));
    let [token, _] = SECRETS;
    let config = home.path().join("Steam/config/config.vdf");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        config,
        format!(
            "\"InstallConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \
             \"ConnectCache\" {{ \"3a1f0c2e\" \"{token}\" }} }} }} }} }}\n"
        ),
    )
    .unwrap();
    saved_login(home.path());
    let (_project, script) = project(false);
    let answer = format!(r#"{{"response": {{"message": "{token}"}}}}"#);
    let (host, _requests) = web_api(vec![("500 Internal Server Error", answer)]);
    let (code, stdout, stderr) = steamship(
        &[
            "promote",
            script.to_str().unwrap(),
            "--build",
            "1234",
            "--branch",
            "public",
        ],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        !stdout.contains(token) && !stderr.contains(token),
        "a secret was printed"
    );
    assert!(stderr.contains("[redacted]"), "{stderr}");
}

#[test]
fn promote_sets_the_default_branch_live_only_while_steam_shows_the_app_unreleased() {
    let (_project, script) = project(false);
    let script = script.to_str().unwrap();
    let unreleased = faked_with(&released_as("prerelease"));
    saved_login(unreleased.path());
    let (host, requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (code, stdout, stderr) = steamship(
        &["promote", script, "--build", "1234", "--branch", "public"],
        Some(unreleased.path()),
        &web_api_variables(&host),
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  app       1000\n")
            && stdout.contains("  branch    default\n")
            && stdout.contains("unreleased, as Steam shows it (prerelease)")
            && stdout.contains("app 1000: BuildID 1234 live on default"),
        "{stdout}"
    );
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.ends_with("appid=1000&buildid=1234&betakey=public"),
        "{setting}"
    );

    // In CI, Steam is asked as the account the login handed over is for, which no home remembers.
    let handed_over = faked_with(&released_as("unavailable"));
    let (ci_host, ci_requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (from_ci, ci_said, ci_why) = steamship(
        &["promote", script, "--build", "1234", "--branch", "default"],
        Some(handed_over.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_LOGIN", &packed_login()),
            (STAND_IN, &ci_host),
        ],
    );
    assert_eq!(from_ci, Some(0_i32), "{ci_said}{ci_why}");
    assert!(
        steamcmd::saved_login(handed_over.path(), Platform::THIS).exists(),
        "the login handed over was put where steamcmd looks"
    );
    assert!(
        ci_said.contains("app 1000: BuildID 1234 live on default"),
        "{ci_said}"
    );
    // Steam knows the default branch only as public, whichever name was typed.
    let ci_setting = ci_requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        ci_setting.ends_with("appid=1000&buildid=1234&betakey=public"),
        "{ci_setting}"
    );

    let released = faked_with(&released_as("released"));
    saved_login(released.path());
    // A stand-in that answers nothing: a request would fail with 1, not be refused with 2.
    let (silent, _unasked) = web_api::<&str>(vec![]);
    let (refused, said, why) = steamship(
        &["promote", script, "--build", "1234", "--branch", "default"],
        Some(released.path()),
        &web_api_variables(&silent),
    );
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert_eq!(
        failure(&why),
        "app 1000 is released, so its default branch is set live in Steamworks, where Valve asks \
         for a Steam Mobile confirmation"
    );
}
