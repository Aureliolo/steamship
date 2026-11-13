//! `steamship branch`.

use std::time::Duration;

use crate::{DESCRIBED, KEY, UPDATED, steamship, web_api, web_api_variables};

#[test]
fn a_branch_is_described_only_when_its_description_differs() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", DESCRIBED),
        ("200 OK", DESCRIBED),
    ]);
    let set = web_api_variables(&host);

    let (code, stdout, stderr) = steamship(
        &[
            "branch",
            "1000010",
            "testing",
            "--description",
            "0.3.0 (abc1234)",
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  steam     \u{2713} described as \"test1\"\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("app 1000010: testing described as \"0.3.0 (abc1234)\""),
        "{stdout}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/UpdateAppBranchDescription/v1/ ")
            && setting.ends_with("appid=1000010&betakey=testing&description=0.3.0+(abc1234)"),
        "{setting}"
    );

    // Already so: nothing is sent, which the stand-in's next answer being a listing shows.
    let (same, said, _) = steamship(
        &["branch", "1000010", "testing", "--description", "test1"],
        Some(home.path()),
        &set,
    );
    assert_eq!(same, Some(0_i32), "{said}");
    assert!(said.contains("testing is already described so"), "{said}");

    let (missing, _, why) = steamship(
        &["branch", "1000010", "beta", "--description", "x"],
        Some(home.path()),
        &set,
    );
    assert_eq!(missing, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "app 1000010 has no branch \"beta\". Create the branch in Steamworks under SteamPipe, \
             Builds first"
        ),
        "{why}"
    );
}

#[test]
fn a_branch_named_in_another_case_is_described_under_steams_name() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", DESCRIBED),
    ]);
    let set = web_api_variables(&host);

    let (code, stdout, stderr) = steamship(
        &["branch", "1000010", "Testing", "--description", "x"],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("app 1000010: testing described as \"x\""),
        "{stdout}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.ends_with("appid=1000010&betakey=testing&description=x"),
        "{setting}"
    );

    let (refused, said, why) = steamship(
        &["branch", "1000010", "test", "--description", "x"],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains("\u{2717} app 1000010 has no branch \"test\"."),
        "a command's own refusal keeps its cross: {why}"
    );
}

#[test]
fn the_default_branch_is_described_in_steamworks_and_a_refusal_says_steams_reason() {
    let home = tempfile::tempdir().unwrap();
    // No stand-in for the first: the default branch is refused before Steam is asked.
    let (code, _, stderr) = steamship(
        &["branch", "1000010", "default", "--description", "x"],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        stderr.contains("the default branch's description is set in Steamworks"),
        "{stderr}"
    );

    let (host, _requests) = web_api(vec![
        ("200 OK", DESCRIBED),
        (
            "404 Not Found",
            r#"{"response": {"result": 2, "message": "Unable to find specificed betakey testing"}}"#,
        ),
    ]);
    let (refused, _, why) = steamship(
        &["branch", "1000010", "testing", "--description", "x"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(refused, Some(1_i32), "{why}");
    assert!(
        why.contains("Steam answered HTTP 404: Unable to find specificed betakey testing"),
        "{why}"
    );
}
