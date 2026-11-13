//! `steamship rollback`.

use std::fs;
use std::time::Duration;

use steamship::ledger;

use crate::{
    SET_LIVE, failure, faked_with, in_game_with, pushed_game, released_as, saved_login, steamship,
    web_api, web_api_variables,
};

#[test]
fn rollback_sets_a_branch_back_to_the_build_before_the_current_one_and_records_it() {
    let home = tempfile::tempdir().unwrap();
    let entry = |branch: &str, build_id: u64| ledger::Entry {
        app: 1_000_010,
        branch: branch.to_owned(),
        build_id,
        commit: format!("{build_id:040x}"),
        green: true,
        rollback: false,
    };
    for each in [
        entry("testing", 5),
        entry("default", 6),
        entry("testing", 7),
    ] {
        ledger::record(home.path(), each).unwrap();
    }
    let (host, requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (code, stdout, stderr) = steamship(
        &["rollback", "1000010", "testing"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    for line in [
        "  from      BuildID 7, commit 000000000000\n",
        "  to        BuildID 5, commit 000000000000\n",
        "app 1000010: testing is back on BuildID 5",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000010&buildid=5&betakey=testing"),
        "{setting}"
    );
    let kept = ledger::load(home.path(), 1_000_010).unwrap();
    assert_eq!(kept.last().map(|each| each.build_id), Some(5));
    assert!(kept.last().is_some_and(|each| each.rollback));
    assert_eq!(kept.len(), 4);
}

#[test]
fn rollback_refuses_a_branch_with_no_earlier_recorded_build_and_asks_steam_nothing() {
    let home = tempfile::tempdir().unwrap();
    ledger::record(
        home.path(),
        ledger::Entry {
            app: 1_000_010,
            branch: "testing".to_owned(),
            build_id: 7,
            commit: "7".repeat(40),
            green: true,
            rollback: false,
        },
    )
    .unwrap();
    // A stand-in that answers nothing: a request would fail with 1, not be refused with 2.
    let (silent, _unasked) = web_api::<&str>(vec![]);
    for (branch, said) in [
        (
            "testing",
            "steamship recorded only BuildID 7 on testing for app 1000010",
        ),
        (
            "beta",
            "steamship has no recorded upload to beta for app 1000010. A rollback works only on \
             the machine that uploaded from the game",
        ),
    ] {
        let (code, stdout, stderr) = steamship(
            &["rollback", "1000010", branch],
            Some(home.path()),
            &web_api_variables(&silent),
        );
        assert_eq!(code, Some(2_i32), "{branch}: {stdout}{stderr}");
        assert!(failure(&stderr).contains(said), "{branch}: {stderr}");
    }
}

#[test]
fn rollback_sets_the_default_branch_back_only_while_steam_shows_the_app_unreleased() {
    let recorded = |released: &str| {
        let home = faked_with(&released_as(released));
        saved_login(home.path());
        for build_id in [5_u64, 7] {
            ledger::record(
                home.path(),
                ledger::Entry {
                    app: 1000,
                    branch: "default".to_owned(),
                    build_id,
                    commit: format!("{build_id:040x}"),
                    green: true,
                    rollback: false,
                },
            )
            .unwrap();
        }
        home
    };
    let unreleased = recorded("prerelease");
    let (host, requests) = web_api(vec![("200 OK", SET_LIVE)]);
    // Typed as default, sent as public: Steam has no branch named default.
    let (set, said, why) = steamship(
        &["rollback", "1000", "default"],
        Some(unreleased.path()),
        &web_api_variables(&host),
    );
    assert_eq!((set, why.as_str()), (Some(0_i32), ""), "{said}");
    assert!(said.contains("default is back on BuildID 5"), "{said}");
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(setting.ends_with("buildid=5&betakey=public"), "{setting}");
    let kept = ledger::load(unreleased.path(), 1000).unwrap();
    assert_eq!(
        kept.last().map(|each| each.branch.as_str()),
        Some("default")
    );

    let home = recorded("released");
    let (silent, _unasked) = web_api::<&str>(vec![]);
    let (code, stdout, stderr) = steamship(
        &["rollback", "1000", "public"],
        Some(home.path()),
        &web_api_variables(&silent),
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("app 1000 is released, so "),
        "{stderr}"
    );
}

#[test]
fn a_games_folder_needs_no_app() {
    let (_temp, root) = pushed_game(None);
    let home = root.parent().unwrap().join("home");
    fs::create_dir_all(&home).unwrap();
    for build_id in [5_u64, 7] {
        ledger::record(
            &home,
            ledger::Entry {
                app: 1_000_050,
                branch: "testing".to_owned(),
                build_id,
                commit: format!("{build_id:040x}"),
                green: true,
                rollback: false,
            },
        )
        .unwrap();
    }
    let (host, requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (code, stdout, stderr) =
        in_game_with(&root, &["rollback", "testing"], &web_api_variables(&host));
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("app 1000050: testing is back on BuildID 5"),
        "{stdout}"
    );
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.ends_with("appid=1000050&buildid=5&betakey=testing"),
        "{setting}"
    );
}
