//! `steamship rich-presence`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use steamship::webapi::STAND_IN;

use crate::{
    KEY, failure, in_game, in_game_with, presence_file, pushed_game, steamship, web_api,
    web_api_variables,
};

/// A form's body as it was meant, its `+` and `%XX` escapes undone.
fn unformed(body: &str) -> String {
    let mut bytes = Vec::with_capacity(body.len());
    let mut rest = body.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        rest = after;
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let (hex, beyond) = rest.split_at(2);
                bytes.push(u8::from_str_radix(&String::from_utf8_lossy(hex), 16).unwrap());
                rest = beyond;
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn rich_presence_is_sent_whole_for_every_file_and_a_result_steam_refuses_is_a_failure() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let english = presence_file(
        files.path(),
        "english",
        "\t\t\"#menu\"\t\"At the menu\"\n\t\t\"#season_rainy\"\t\"Autumn\"\n\
         \t\t\"#running\"\t\"In {#season_%season%}\"\n",
    );
    let german = presence_file(files.path(), "german", "\t\t\"#menu\"\t\"Im Men\u{fc}\"\n");
    let (host, requests) = web_api(vec![
        ("200 OK\r\nX-eresult: 1", r#"{"response": {}}"#),
        (
            "200 OK\r\nX-eresult: 8\r\nX-error_message: Invalid token",
            r#"{"response": {}}"#,
        ),
    ]);
    let set = web_api_variables(&host);
    let command = ["rich-presence", "480", &english, &german];

    let (code, stdout, stderr) = steamship(&command, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  english   3 tokens, from "), "{stdout}");
    assert!(stdout.contains("  german    1 token, from "), "{stdout}");
    assert!(
        stdout.contains("app 480: rich presence replaced for english, german"),
        "{stdout}"
    );
    let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        request.starts_with("POST /IProductInfoService/SetRichPresenceLocalization/v1/ ")
            && request
                .to_ascii_lowercase()
                .contains(&format!("x-webapi-key: {KEY}")),
        "{request}"
    );
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    assert_eq!(
        unformed(body),
        "input_json={\"appid\":480,\"languages\":[\
         {\"language\":\"english\",\"tokens\":[\
         {\"token\":\"#menu\",\"value\":\"At the menu\"},\
         {\"token\":\"#season_rainy\",\"value\":\"Autumn\"},\
         {\"token\":\"#running\",\"value\":\"In {#season_%season%}\"}]},\
         {\"language\":\"german\",\"tokens\":[{\"token\":\"#menu\",\"value\":\"Im Men\u{fc}\"}]}]}",
        "every token of every file, and no SteamID"
    );

    let (refused, _, why) = steamship(&command, Some(home.path()), &set);
    assert_eq!(refused, Some(1_i32), "{why}");
    assert!(why.contains("EResult 8, Invalid token"), "{why}");
}

#[test]
fn rich_presence_files_are_checked_before_steam_and_a_dry_run_sends_nothing() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 0 or 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let english = presence_file(files.path(), "english", "\t\t\"#menu\"\t\"At the menu\"\n");

    let (previewed, stdout, stderr) = steamship(
        &["rich-presence", "480", &english, "--dry-run"],
        Some(home.path()),
        &set,
    );
    assert_eq!(previewed, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.starts_with("steamship rich-presence, dry run\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  english   1 token, from "), "{stdout}");
    assert!(stdout.contains("nothing was sent"), "{stdout}");

    let unnamed = presence_file(
        files.path(),
        "french",
        "\t\t\"#running\"\t\"In {#season_%s%}\"\n",
    );
    let unfillable = presence_file(
        files.path(),
        "german",
        "\t\t\"#running\"\t\"Zu 50% fertig\"\n",
    );
    let again = files.path().join("again.vdf");
    let _: u64 = fs::copy(&english, &again).unwrap();
    let missing = files.path().join("missing.vdf");
    for (file, why) in [
        (unnamed, "no token starts #season_"),
        (
            unfillable,
            "#running: \"% fertig\" holds a % that starts no %field%",
        ),
        (
            again.to_string_lossy().into_owned(),
            "is a second file for english",
        ),
        (missing.to_string_lossy().into_owned(), "missing.vdf: "),
    ] {
        let (code, _, said) = steamship(
            &["rich-presence", "480", &english, &file],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{said}");
        assert!(said.contains(why), "{why:?} in {said}");
    }
}

#[test]
fn a_games_rich_presence_is_read_beside_its_scripts() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["rich-presence", "--dry-run"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  english   1 token, from "), "{stdout}");
    assert!(stdout.contains("nothing was sent"), "{stdout}");
}

#[test]
fn a_games_rich_presence_is_sent_from_its_pushed_tree_and_one_without_any_is_refused() {
    let (_temp, root) = pushed_game(None);
    let (host, requests) = web_api(vec![("200 OK\r\nX-eresult: 1", r#"{"response": {}}"#)]);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["rich-presence"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("as origin's main has it"), "{stdout}");
    assert!(
        stdout.contains("app 1000050: rich presence replaced for english"),
        "{stdout}"
    );
    let sent = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        sent.starts_with("POST /IProductInfoService/SetRichPresenceLocalization/v1/ "),
        "{sent}"
    );

    fs::remove_file(root.join("steam").join("rich_presence_english.vdf")).unwrap();
    let (none, said, why) = in_game(&root, &["rich-presence", "--dry-run"]);
    assert_eq!(none, Some(2_i32), "{said}{why}");
    let refused = failure(&why);
    assert!(
        refused.starts_with("the game holds no rich_presence_<language>.vdf beside ")
            && refused.ends_with(&format!(
                "{}. Write rich_presence_english.vdf there, or name the files to send",
                Path::new("steam").join("app_build.vdf").display()
            )),
        "{why}"
    );
    assert!(said.starts_with("steamship rich-presence\n"), "{said}");
}
