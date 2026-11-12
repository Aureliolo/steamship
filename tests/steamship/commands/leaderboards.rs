//! `steamship leaderboards`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use steamship::webapi::STAND_IN;

use crate::{
    INN_BOARDS, KEY, failure, faked_with, in_game, in_game_with, pushed_game, released_as,
    saved_login, steamship, web_api, web_api_variables,
};

/// `GetLeaderboardsForGame` answering with one leaderboard, shaped as Steam answered in a trial.
const LEADERBOARDS: &str = r#"{"response": {"result": 1, "leaderboards": [
    {"id": 7, "name": "gold", "entries": 12, "sortmethod": "Descending", "displaytype": "Numeric",
     "onlytrustedwrites": true, "onlyfriendsreads": false}
]}}"#;

/// `FindOrCreateLeaderboard` answering that it made `fastest_season`.
const MADE: &str = r#"{"result": {"result": 1, "leaderboard": {"leaderboardName": "fastest_season",
    "leaderBoardID": 8, "leaderBoardEntries": 0, "leaderBoardSortMethod": "Ascending",
    "leaderBoardDisplayType": "Seconds", "onlytrustedwrites": false, "onlyfriendsreads": true}}}"#;

/// A leaderboards file for app 480 with `gold` as Steam has it, or not, and `fastest_season`.
fn leaderboards_file(folder: &Path, gold_trusted: bool) -> String {
    let path = folder.join("leaderboards.json");
    fs::write(
        &path,
        format!(
            r#"{{"app": 480, "leaderboards": [
                {{"name": "gold", "sort": "descending", "display": "numeric",
                  "trusted_writes": {gold_trusted}}},
                {{"name": "fastest_season", "sort": "ascending", "display": "seconds",
                  "friends_only": true}}
            ]}}"#
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn leaderboards_are_listed_and_checked_and_those_missing_are_made_as_the_file_has_them() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", LEADERBOARDS),
        ("200 OK", LEADERBOARDS),
        ("200 OK", MADE),
    ]);
    let set = web_api_variables(&host);

    let (code, listed, _) = steamship(&["leaderboards", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("  steam     \u{2713} 1 leaderboard on Steam\n"),
        "{listed}"
    );
    assert!(
        listed.contains("  gold      descending, numeric, 12 scores, trusted writes only\n"),
        "{listed}"
    );
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamLeaderboards/GetLeaderboardsForGame/v2/?appid=480&"),
        "{asked}"
    );

    // Not made without --create, and a setting that differs is said with where to change it.
    let drifted = leaderboards_file(files.path(), false);
    let (refused, said, why) = steamship(
        &["leaderboards", "480", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains("gold: trusted writes only is \"yes\" on Steam and \"no\" in the file")
            && why.contains("fastest_season: in the file, not on Steam. --create makes it"),
        "{why}"
    );
    assert!(said.contains("Change them in Steamworks"), "{said}");
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());

    let matching = leaderboards_file(files.path(), true);
    let (made, making, stderr) = steamship(
        &["leaderboards", "480", "--check", &matching, "--create"],
        Some(home.path()),
        &set,
    );
    assert_eq!(made, Some(0_i32), "{making}{stderr}");
    assert!(
        making.contains("  steam     \u{2713} made fastest_season\n"),
        "{making}"
    );
    assert!(
        making.contains("Steam's leaderboards match the file"),
        "{making}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let creating = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        creating.starts_with("POST /ISteamLeaderboards/FindOrCreateLeaderboard/v2/ ")
            && creating.ends_with(
                "appid=480&name=fastest_season&sortmethod=Ascending&displaytype=Seconds\
                 &createifnotfound=true&onlytrustedwrites=false&onlyfriendsreads=true"
            ),
        "only the one Steam lacks, as the file has it: {creating}"
    );
}

#[test]
fn leaderboards_saved_from_steam_are_a_file_that_check_finds_steam_matches() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("200 OK", LEADERBOARDS)]);
    let set = web_api_variables(&host);
    let file = files.path().join("leaderboards.json");
    let path = file.to_str().unwrap();

    let (code, said, stderr) = steamship(
        &["leaderboards", "480", "--save", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!("app 480: 1 leaderboard saved to {path}")),
        "{said}"
    );
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("\"name\": \"gold\""), "{saved}");
    assert!(saved.contains("\"trusted_writes\": true"), "{saved}");
    assert!(!saved.contains("12"), "scores are no setting: {saved}");

    let (checked, matching, why) = steamship(
        &["leaderboards", "480", "--check", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(checked, Some(0_i32), "{matching}{why}");
    assert!(
        matching.contains("Steam's leaderboards match the file"),
        "{matching}"
    );

    let (both, _, refused) = steamship(
        &["leaderboards", "480", "--save", path, "--create"],
        Some(home.path()),
        &set,
    );
    assert_eq!(both, Some(2_i32), "{refused}");
    assert!(refused.contains("cannot be used with"), "{refused}");

    // A file kept by hand is never written over, and Steam is not asked.
    let kept = r#"{"leaderboards": [{"name": "daily", "sort": "ascending", "display": "numeric",
        "period": "day"}]}"#;
    fs::write(&file, kept).unwrap();
    let (held, _, losing) = steamship(
        &["leaderboards", "480", "--save", path],
        Some(home.path()),
        &set,
    );
    assert_eq!(held, Some(2_i32), "{losing}");
    assert_eq!(
        failure(&losing),
        format!(
            "{path} holds \"period\" in leaderboard 1, which --save would lose. Save to another \
             file"
        )
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), kept);

    // A file there that cannot be read is a failure, before Steam is asked.
    let folder = files.path().to_str().unwrap();
    let (unread, _, unreadable) = steamship(
        &["leaderboards", "480", "--save", folder],
        Some(home.path()),
        &web_api_variables("http://127.0.0.1:9"),
    );
    assert_eq!(unread, Some(1_i32), "{unreadable}");
    assert!(
        failure(&unreadable).starts_with(&format!("{folder}: ")),
        "{unreadable}"
    );
}

#[test]
fn leaderboards_json_prints_only_what_steam_holds_or_how_it_failed_on_standard_output() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("403 Forbidden", "")]);
    let set = web_api_variables(&host);
    let (code, stdout, stderr) =
        steamship(&["leaderboards", "480", "--json"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
        serde_json::json!({"app": 480_u32, "leaderboards": [
            {"name": "gold", "sort": "descending", "display": "numeric",
             "trusted_writes": true, "friends_only": false, "scores": 12_u64}
        ]})
    );
    assert!(
        stdout.ends_with("}\n") && stdout.lines().count() == 1,
        "{stdout}"
    );
    assert!(stderr.contains("1 leaderboard on Steam"), "{stderr}");

    let (refused, said, why) =
        steamship(&["leaderboards", "480", "--json"], Some(home.path()), &set);
    assert_eq!(refused, Some(1_i32), "{said}{why}");
    let answer: serde_json::Value = serde_json::from_str(&said).unwrap();
    assert_eq!(
        answer.pointer("/app").and_then(serde_json::Value::as_u64),
        Some(480)
    );
    assert_eq!(
        answer
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str),
        Some("key-refused")
    );

    let (both, _, conflict) = steamship(
        &["leaderboards", "480", "--json", "--check", "x.json"],
        Some(home.path()),
        &set,
    );
    assert_eq!(both, Some(2_i32), "{conflict}");
    assert!(conflict.contains("cannot be used with"), "{conflict}");
}

/// `DeleteLeaderboard` answering that it deleted the one asked for, as Steam answered in a trial.
const DELETED: &str = r#"{"result": {"result": 1}}"#;

/// A leaderboards file for app 1000 holding `fastest_season` alone, as [`MADE`] makes it.
fn without_gold(folder: &Path) -> String {
    let path = folder.join("leaderboards.json");
    fs::write(
        &path,
        r#"{"app": 1000, "leaderboards": [
            {"name": "fastest_season", "sort": "ascending", "display": "seconds",
             "friends_only": true}
        ]}"#,
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_leaderboard_the_file_no_longer_holds_is_deleted_only_while_the_app_is_unreleased() {
    let files = tempfile::tempdir().unwrap();
    let file = without_gold(files.path());
    let command = [
        "leaderboards",
        "1000",
        "--check",
        &file,
        "--create",
        "--delete-missing",
    ];
    let unreleased = faked_with(&released_as("prerelease"));
    saved_login(unreleased.path());
    let (host, requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", DELETED),
        ("200 OK", MADE),
    ]);
    let set = web_api_variables(&host);
    let (code, said, stderr) = steamship(&command, Some(unreleased.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    for line in [
        "unreleased, as Steam shows it (prerelease)",
        "  steam     \u{2713} deleted gold\n",
        "  steam     \u{2713} made fastest_season\n",
        "Steam's leaderboards match the file",
    ] {
        assert!(said.contains(line), "{line:?} in {said}");
    }
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let deleting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        deleting.starts_with("POST /ISteamLeaderboards/DeleteLeaderboard/v1/ ")
            && deleting.ends_with("appid=1000&name=gold"),
        "{deleting}"
    );

    let released = faked_with(&released_as("released"));
    saved_login(released.path());
    let (asked, _unasked) = web_api(vec![("200 OK", LEADERBOARDS)]);
    let (refused, kept, why) =
        steamship(&command, Some(released.path()), &web_api_variables(&asked));
    assert_eq!(refused, Some(2_i32), "{kept}{why}");
    assert_eq!(
        failure(&why),
        "app 1000 is released, so deleting gold would take its players' scores with it. Delete \
         it in Steamworks, under Stats & Achievements, Leaderboards, or put it back in the file"
    );
    assert!(!kept.contains("deleted"), "{kept}");

    let (refusing, _refused_requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", r#"{"result": {"result": 2}}"#),
    ]);
    let (failed, _, unsaid) = steamship(
        &command,
        Some(unreleased.path()),
        &web_api_variables(&refusing),
    );
    assert_eq!(failed, Some(1_i32), "{unsaid}");
    assert!(unsaid.contains("result 2"), "{unsaid}");
}

#[test]
fn a_dated_board_a_template_made_is_checked_against_it_and_never_made_or_deleted() {
    // No steamcmd is installed in the home: asking whether the app is released would fail.
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("boards.json");
    fs::write(
        &records,
        r#"[{"type": "leaderboard", "steam": "gold", "sort": "descending", "display": "numeric",
             "trusted_writes": true},
            {"type": "leaderboard", "steam": "DAILY", "sort": "descending", "display": "numeric",
             "period": "day", "details": 5}]"#,
    )
    .unwrap();
    let records = records.to_string_lossy().into_owned();
    let dated = r#"{"response": {"result": 1, "leaderboards": [
        {"id": 7, "name": "gold", "entries": 12, "sortmethod": "Descending",
         "displaytype": "Numeric", "onlytrustedwrites": true, "onlyfriendsreads": false},
        {"id": 9, "name": "DAILY_20261025", "entries": 3, "sortmethod": "Descending",
         "displaytype": "Numeric", "onlytrustedwrites": false, "onlyfriendsreads": false}
    ]}}"#;
    let (host, requests) = web_api(vec![("200 OK", dated)]);
    let (code, said, stderr) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &records,
            "--create",
            "--delete-missing",
        ],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!("{records}, 1 leaderboard, 1 dated template\n"))
            && said.contains("Steam's leaderboards match the file")
            && !said.contains("made")
            && !said.contains("deleted"),
        "{said}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    assert!(
        requests.recv_timeout(Duration::from_secs(1)).is_err(),
        "Steam is asked for the leaderboards alone"
    );
}

#[test]
fn delete_missing_with_nothing_to_delete_asks_steamcmd_nothing() {
    // No steamcmd is installed in the home: asking whether the app is released would fail.
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let file = leaderboards_file(files.path(), true);
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("200 OK", MADE)]);
    let (code, said, stderr) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &file,
            "--create",
            "--delete-missing",
        ],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains("Steam's leaderboards match the file") && !said.contains("deleted"),
        "{said}"
    );
}

#[test]
fn games_kits_leaderboard_records_are_checked_and_a_leaderboard_in_two_files_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("200 OK", LEADERBOARDS)]);
    let set = web_api_variables(&host);
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "stat", "id": "base:gold", "steam": "gold_banked"},
            {"type": "leaderboard", "id": "base:gold", "steam": "gold", "name": "Gold",
             "sort": "descending", "display": "numeric", "upload": "keep_best", "details": 1,
             "trusted_writes": true}]"#,
    )
    .unwrap();
    let records = records.to_string_lossy().into_owned();

    let (code, said, stderr) = steamship(
        &["leaderboards", "480", "--check", &records],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!("{records}, 1 leaderboard\n"))
            && said.contains("Steam's leaderboards match the file")
            && !said.contains("match the files"),
        "{said}"
    );

    let stats = files.path().join("stats.json");
    fs::write(&stats, r#"[{"type": "stat", "steam": "floors"}]"#).unwrap();
    let stats = stats.to_string_lossy().into_owned();
    let (both, matching, unsaid) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &records,
            "--check",
            &stats,
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(both, Some(0_i32), "{matching}{unsaid}");
    assert!(
        matching.contains(&format!("{stats}, 0 leaderboards\n"))
            && matching.contains("Steam's leaderboards match the files"),
        "{matching}"
    );

    let other = leaderboards_file(files.path(), true);
    let (refused, _, why) = steamship(
        &["leaderboards", "480", "--check", &records, &other],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!("gold is in both {records} and {other}")),
        "{why}"
    );
}

#[test]
fn a_leaderboards_file_that_is_not_one_is_refused_before_steam_and_create_needs_check() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let broken = files.path().join("broken.json");
    fs::write(
        &broken,
        r#"{"leaderboards": [{"name": "gold", "sort": "up"}]}"#,
    )
    .unwrap();
    let other = leaderboards_file(files.path(), true);
    for (file, why) in [
        (
            broken.to_string_lossy().into_owned(),
            "gold: \"sort\" is not ascending or descending",
        ),
        (other, "is for app 480, not app 1000010"),
    ] {
        let (code, _, stderr) = steamship(
            &["leaderboards", "1000010", "--check", &file, "--create"],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{stderr}");
        assert!(stderr.contains(why), "{why:?} in {stderr}");
    }
    let (code, _, stderr) = steamship(
        &["leaderboards", "1000010", "--create"],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("there is no game in "), "{stderr}");
}

#[test]
fn a_games_leaderboard_records_are_checked_and_one_without_any_is_refused() {
    let (_temp, root) = pushed_game(None);
    let (none, stdout, stderr) = in_game(&root, &["leaderboards", "--create"]);
    assert_eq!(none, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "the game holds no leaderboard records to check or make"
    );

    fs::write(
        root.join("mods")
            .join("base")
            .join("data")
            .join("boards.json"),
        INN_BOARDS,
    )
    .unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS)]);
    let (code, said, why) = in_game_with(
        &root,
        &["leaderboards", "--check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{said}{why}");
    assert!(said.contains("boards.json, 1 leaderboard\n"), "{said}");
    assert!(
        said.contains("Steam's leaderboards match the file"),
        "{said}"
    );
}
