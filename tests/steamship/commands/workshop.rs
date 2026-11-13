//! `steamship workshop`.

use std::fs;
use std::path::{Path, PathBuf};

use steamship::platform::Platform;
use steamship::steamcmd;

use crate::{
    at, failure, faked_with, icon_png, in_game, packed_login, pushed, pushed_game, saved_login,
    steamship, steamship_in, text_at, workshop_item,
};

/// A home whose steamcmd, given a Workshop item's script, says `said`, exits `code`, and gives
/// the item ID 777 in that script when it had none, as steamcmd does with an item it makes.
fn publishing(said: &str, code: u8) -> tempfile::TempDir {
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         grep -q publishedfileid \"$script\" || \
         printf '\"workshopitem\"\\n{{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}}\\n' > \"$script\"\n\
         printf '%s\\r\\n' '{said}'\n\
         exit {code}\n"
    ));
    saved_login(home.path());
    home
}

#[test]
fn a_new_workshop_item_is_uploaded_and_its_id_given_to_add_to_the_script() {
    let home = publishing("Success.", 0);
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.starts_with(&format!(
            "steamship workshop\n  script    {}\n",
            script.display()
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("  item      \u{2713} app 480, a new item, 1 file, checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  \u{2713} app 480: Workshop item 777\n    a new item. Add \"publishedfileid\" \"777\" \
             to {} so that later uploads update it\n",
            script.display()
        )),
        "{stdout}"
    );
    assert!(
        !fs::read_to_string(&script).unwrap().contains("777"),
        "the script is left for its author"
    );
    let console = fs::read_to_string(home.path().join("workshop/480/new/steamcmd.log")).unwrap();
    assert!(console.contains("Success."), "{console}");
}

#[test]
fn a_workshop_upload_in_ci_logs_in_as_the_account_handed_over() {
    let home = publishing("Success.", 0);
    fs::remove_file(home.path().join("account")).unwrap();
    let saved = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::remove_file(&saved).unwrap();
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains(" +login build_bot "), "{args}");
    assert!(
        saved.exists(),
        "the login is put back where steamcmd reads it"
    );
}

#[test]
fn a_new_workshop_item_in_ci_is_refused_unless_asked_for() {
    let home = tempfile::tempdir().unwrap();
    let (_temp, script) = workshop_item("");
    for ci in [("CI", "true"), ("GITHUB_ACTIONS", "true")] {
        let (code, stdout, stderr) = steamship(
            &[
                "workshop",
                script.to_str().unwrap(),
                "--account",
                "build_bot",
            ],
            Some(home.path()),
            &[ci],
        );
        assert_eq!(code, Some(2_i32), "{stdout}");
        assert_eq!(
            failure(&stderr),
            format!(
                "refused: {}: there is no \"publishedfileid\", so every CI run would make a new \
                 Workshop item. Add the ID of the item to update, or pass --new to make one",
                script.display()
            )
        );
    }
}

#[test]
fn a_new_workshop_item_asked_for_in_actions_hands_its_id_to_the_workflow() {
    let home = publishing("Success.", 0);
    let (_temp, script) = workshop_item("");
    let outputs = home.path().join("github_output");
    fs::write(&outputs, "").unwrap();
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--new"],
        Some(home.path()),
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_OUTPUT", outputs.to_str().unwrap()),
        ],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(fs::read_to_string(&outputs).unwrap(), "item-id=777\n");
}

#[test]
fn a_workshop_item_named_by_its_id_is_updated() {
    let home = publishing("Success.", 0);
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, _) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(0_i32), "{stdout}");
    assert!(
        stdout.contains("app 480, item 5674, 1 file, checked"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\u{2713} app 480: Workshop item 5674\n    log  "),
        "{stdout}"
    );
}

#[test]
fn a_workshop_upload_steam_will_not_log_in_for_exits_3() {
    let home = publishing(
        "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)",
        5,
    );
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(3_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam refused the saved login (Expired Login Auth Code)"
    );
}

#[test]
fn a_failed_workshop_upload_names_why_and_where_the_log_is_and_exits_1() {
    let home = publishing("ERROR! Failed to update workshop item (Access Denied).", 8);
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "Failed to update workshop item (Access Denied)."
    );
    assert!(stderr.contains("log  "), "{stderr}");
}

/// `workshop` for the item at `folder`, as `workshop_item` lays it out, given by its fields for
/// app 480 and kept in `items` as `example`, with `extra` after them.
fn by_fields(folder: &Path, items: &Path, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "workshop",
        "480",
        "--items",
        items.to_str().unwrap(),
        "--name",
        "example",
        "--content",
        folder.join("hat").to_str().unwrap(),
        "--title",
        "A green hat",
        "--preview-image",
        folder.join("hat.png").to_str().unwrap(),
    ]
    .map(str::to_owned)
    .into();
    args.extend(extra.iter().map(|&arg| arg.to_owned()));
    args
}

#[test]
fn an_accounts_name_in_a_refusal_changes_neither_the_exit_code_nor_the_message() {
    for (name, said, code, words) in [
        (
            "limit",
            "Logging in user 'limit' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "steam",
            "Logging in user 'steam' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "cached",
            "Logging in user 'cached' [U:1:0] to Steam Public...FAILED (No cached credentials \
             and @NoPromptForPassword is set)",
            3_i32,
            "steamcmd holds no saved login for the build account",
        ),
    ] {
        let home = faked_with(&format!("#!/bin/sh\necho \"{said}\"\nexit 5\n"));
        saved_login(home.path());
        let (_temp, script) = workshop_item("");
        let (found, stdout, stderr) = steamship(
            &["workshop", script.to_str().unwrap(), "--account", name],
            Some(home.path()),
            &[],
        );
        assert_eq!(found, Some(code), "{name}: {stdout}{stderr}");
        assert!(stderr.contains(words), "{name}: {stderr}");
    }
}

#[test]
fn an_account_named_like_steamships_words_leaves_its_sentences_whole_in_a_failed_item() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"Logging in user 'steamcmd' [U:1:0] to Steam Public...OK\"\n\
         exit 137\n",
    );
    saved_login(home.path());
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &[
            "workshop",
            script.to_str().unwrap(),
            "--account",
            "steamcmd",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("steamcmd exited 137 without saying why"),
        "{stderr}"
    );
    assert!(!format!("{stdout}{stderr}").contains("user 'steamcmd'"));
}

#[test]
fn an_items_saved_console_names_no_account() {
    let home = faked_with(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         printf '\"workshopitem\"\\n{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}\\n' > \"$script\"\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'Success.'\n",
    );
    saved_login(home.path());
    let (temp, script) = workshop_item("");
    let folder = script.parent().unwrap();
    let items = temp.path().join("my game (x86)/steam/workshop_items.json");
    let args = by_fields(folder, &items, &["--visibility", "private"]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let console =
        fs::read_to_string(home.path().join("workshop/480/example/steamcmd.log")).unwrap();
    assert!(
        console.contains("Logging in user '[redacted]'") && !console.contains("build_bot"),
        "{console}"
    );
}

#[test]
fn an_item_given_by_its_fields_is_made_kept_by_its_name_and_then_updated_from_ci() {
    let home = publishing("Success.", 0);
    let (temp, script) = workshop_item("");
    let folder = script.parent().unwrap();
    let items = temp.path().join("my game (x86)/steam/workshop_items.json");
    let made = by_fields(
        folder,
        &items,
        &[
            "--description",
            "Green.",
            "--change-note",
            "1.0.0",
            "--visibility",
            "private",
            "--json",
        ],
    );
    let made: Vec<&str> = made.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&made, Some(home.path()), &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let log = home
        .path()
        .join("workshop")
        .join("480")
        .join("example")
        .join("steamcmd.log");
    let summary = |outcome: &str| {
        format!(
            "{}\n",
            serde_json::json!({
                "app": 480_u32,
                "items": [{
                    "item_id": 777_u64,
                    "log": log.to_str().unwrap(),
                    "name": "example",
                    "outcome": outcome,
                }],
            })
        )
    };
    assert_eq!(stdout, summary("made"));
    assert!(
        stderr.contains(&format!("  items     {}\n", items.display()))
            && stderr.contains("  item      \u{2713} app 480, a new item, 1 file, checked\n")
            && stderr.contains(&format!(
                "  \u{2713} app 480: Workshop item 777\n    a new item, kept in {} as example. \
                 Commit it so that later uploads update it\n",
                items.display()
            )),
        "{stderr}"
    );
    assert_eq!(
        fs::read_to_string(&items).unwrap(),
        "{\n  \"app\": 480,\n  \"items\": {\n    \"example\": 777\n  }\n}\n"
    );

    let updated = by_fields(folder, &items, &["--json"]);
    let updated: Vec<&str> = updated.iter().map(String::as_str).collect();
    let (again, printed, said) = steamship(&updated, Some(home.path()), &[("CI", "true")]);
    assert_eq!(again, Some(0_i32), "{printed}{said}");
    assert_eq!(printed, summary("updated"));
    assert!(
        said.contains("app 480, item 777, 1 file, checked"),
        "{said}"
    );
    let copy =
        fs::read_to_string(home.path().join("workshop/480/example/workshop_item.vdf")).unwrap();
    assert!(
        copy.contains("\"publishedfileid\"\t\t\"777\"") && !copy.contains("visibility"),
        "the item made is updated, keeping the visibility it has: {copy}"
    );

    // Another item uploaded after it keeps its console beside its own copy, and the first's stays.
    let other: Vec<String> = by_fields(folder, &items, &["--visibility", "private"])
        .into_iter()
        .map(|arg| {
            if arg == "example" {
                "boots".to_owned()
            } else {
                arg
            }
        })
        .collect();
    let (third, _, third_said) = steamship(
        &other.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(home.path()),
        &[],
    );
    assert_eq!(third, Some(0_i32), "{third_said}");
    for name in ["example", "boots"] {
        let console = fs::read_to_string(
            home.path()
                .join(format!("workshop/480/{name}/steamcmd.log")),
        )
        .unwrap();
        assert!(console.contains("Success."), "{name}: {console}");
    }
}

#[cfg(unix)]
#[test]
fn an_item_made_whose_id_cannot_be_kept_says_what_to_keep_by_hand_and_exits_5() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = publishing("Success.", 0);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    fs::write(&items, r#"{"app": 480, "items": {"hats": 5674}}"#).unwrap();
    fs::set_permissions(&items, fs::Permissions::from_mode(0o444)).unwrap();
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "\u{2717} app 480: Workshop item 777 was made: {}: ",
            items.display()
        )) && stderr
            .contains(". Add \"example\": 777 to its \"items\", or the next upload makes another"),
        "{stderr}"
    );
    assert_eq!(
        fs::read_to_string(&items).unwrap(),
        r#"{"app": 480, "items": {"hats": 5674}}"#
    );
}

const DENIED: &str = "ERROR! Failed to update workshop item (Access Denied).";
const REFUSED_LOGIN: &str =
    "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)";

#[test]
fn an_item_made_by_a_run_that_then_failed_is_kept_and_exits_1() {
    let home = publishing(DENIED, 1);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("Failed to update workshop item (Access Denied).")
            && stderr.contains("app 480: Workshop item 777 was made before the run failed")
            && stderr.contains("kept as example in "),
        "{stdout}{stderr}"
    );
    assert_eq!(
        fs::read_to_string(&items).unwrap(),
        "{\n  \"app\": 480,\n  \"items\": {\n    \"example\": 777\n  }\n}\n"
    );
}

#[test]
fn an_item_a_failed_run_made_from_a_script_exits_5_and_names_its_id() {
    let home = publishing(DENIED, 1);
    let (_temp, script) = workshop_item("");
    let before = fs::read_to_string(&script).unwrap();
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("Failed to update workshop item (Access Denied).")
            && stderr.contains("app 480: Workshop item 777 was made before the run failed")
            && stderr.contains("Add \"publishedfileid\" \"777\" to "),
        "{stdout}{stderr}"
    );
    assert_eq!(fs::read_to_string(&script).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn an_item_a_failed_run_made_that_cannot_be_kept_exits_5() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = publishing(DENIED, 1);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    fs::write(&items, r#"{"app": 480, "items": {"hats": 5674}}"#).unwrap();
    fs::set_permissions(&items, fs::Permissions::from_mode(0o444)).unwrap();
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(stderr.contains("Workshop item 777 was made"), "{stderr}");
}

#[test]
fn an_item_made_before_steam_refused_the_login_is_kept_and_exits_3() {
    let home = publishing(REFUSED_LOGIN, 5);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert!(
        fs::read_to_string(&items).unwrap().contains("777"),
        "{stderr}"
    );
}

#[test]
fn an_item_made_from_a_script_before_steam_refused_the_login_exits_5_and_names_its_id() {
    let home = publishing(REFUSED_LOGIN, 5);
    let (_temp, script) = workshop_item("");
    let before = fs::read_to_string(&script).unwrap();
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("not logged in")
            && stderr.contains("app 480: Workshop item 777 was made before the run failed")
            && stderr.contains("Add \"publishedfileid\" \"777\" to "),
        "{stdout}{stderr}"
    );
    assert_eq!(fs::read_to_string(&script).unwrap(), before);
}

#[test]
fn an_item_a_failed_run_made_in_actions_hands_its_id_to_the_workflow() {
    let home = publishing(DENIED, 1);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let outputs = home.path().join("github_output");
    fs::write(&outputs, "").unwrap();
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private", "--new"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(
        &args,
        Some(home.path()),
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_OUTPUT", outputs.to_str().unwrap()),
        ],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(fs::read_to_string(&outputs).unwrap(), "item-id=777\n");
}

/// The one line of JSON `stdout` is, parsed.
fn one_line(stdout: &str) -> serde_json::Value {
    assert_eq!(stdout.trim_end().lines().count(), 1, "{stdout}");
    serde_json::from_str(stdout.trim()).unwrap()
}

#[test]
fn an_item_a_failed_run_made_from_a_script_is_named_in_the_json() {
    let home = publishing(DENIED, 1);
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--json"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(line.pointer("/items/0/outcome"), Some(&"made".into()));
    assert_eq!(line.pointer("/items/0/item_id"), Some(&777_u64.into()));
    assert_eq!(line.pointer("/error/code"), Some(&"unfinished".into()));
    let message = line.pointer("/error/message").unwrap().as_str().unwrap();
    assert!(
        message.starts_with(&format!(
            "Steam holds Workshop item 777 of app 480, which {} does not name",
            script.display()
        )) && message.contains("Failed to update workshop item (Access Denied)."),
        "{message}"
    );
}

#[test]
fn an_item_made_by_a_run_that_then_failed_is_made_in_the_json() {
    let home = publishing(DENIED, 1);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private", "--json"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(line.pointer("/items/0/outcome"), Some(&"made".into()));
    assert_eq!(line.pointer("/items/0/item_id"), Some(&777_u64.into()));
    assert_eq!(line.pointer("/error/code"), Some(&"steamcmd-failed".into()));
    assert!(
        line.pointer("/error/message")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Failed to update workshop item (Access Denied)."),
        "{line}"
    );
}

#[test]
fn an_item_made_from_a_script_before_steamcmd_altered_itself_says_so_as_json() {
    let home = publishing_then("echo >> \"$0\"");
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--json"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(line.pointer("/items/0/outcome"), Some(&"made".into()));
    assert_eq!(line.pointer("/items/0/item_id"), Some(&777_u64.into()));
    assert_eq!(line.pointer("/error/code"), Some(&"unfinished".into()));
    let message = line.pointer("/error/message").unwrap().as_str().unwrap();
    assert!(
        message.starts_with("Steam holds Workshop item 777 of app 480, which")
            && message.contains(&script.display().to_string())
            && message.contains("no longer the pinned one"),
        "{message}"
    );
}

#[cfg(unix)]
#[test]
fn an_item_made_and_not_kept_says_so_as_json() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = publishing("Success.", 0);
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    fs::write(&items, r#"{"app": 480, "items": {"hats": 5674}}"#).unwrap();
    fs::set_permissions(&items, fs::Permissions::from_mode(0o444)).unwrap();
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private", "--json"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(line.pointer("/items/0/outcome"), Some(&"made".into()));
    assert_eq!(line.pointer("/items/0/item_id"), Some(&777_u64.into()));
    assert_eq!(line.pointer("/error/code"), Some(&"unfinished".into()));
    let message = line.pointer("/error/message").unwrap().as_str().unwrap();
    assert!(
        message.starts_with("Steam holds Workshop item 777 of app 480, which")
            && message.contains(&items.display().to_string()),
        "{message}"
    );
}

#[test]
fn a_refused_item_prints_no_json() {
    let home = publishing("Success.", 0);
    let (_temp, script) = workshop_item("");
    fs::remove_dir_all(script.parent().unwrap().join("hat")).unwrap();
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--json"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn a_failed_update_of_an_item_with_an_id_is_no_made_item_whatever_steamcmd_wrote() {
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         printf '\"workshopitem\"\\n{{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"999\"\\n}}\\n' > \"$script\"\n\
         printf '%s\\r\\n' '{DENIED}'\n\
         exit 1\n"
    ));
    saved_login(home.path());
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let outputs = home.path().join("github_output");
    fs::write(&outputs, "").unwrap();
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_OUTPUT", outputs.to_str().unwrap()),
        ],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        !stdout.contains("Workshop item 999") && !stderr.contains("was made before the run failed"),
        "{stdout}{stderr}"
    );
    assert_eq!(fs::read_to_string(&outputs).unwrap(), "");
}

#[test]
fn an_item_a_failed_run_made_from_a_script_before_steamcmd_altered_itself_exits_5() {
    let home = faked_with(&format!(
        "#!/bin/sh\n{MAKING}printf '%s\\r\\n' '{DENIED}'\necho >> \"$0\"\nexit 1\n"
    ));
    saved_login(home.path());
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("app 480: Workshop item 777 was made before the run failed"),
        "{stderr}"
    );
}

#[test]
fn an_item_given_by_its_fields_is_refused_before_steam_when_it_cannot_be_kept_or_made() {
    let home = tempfile::tempdir().unwrap();
    let (temp, script) = workshop_item("");
    let folder = script.parent().unwrap();
    let items = temp.path().join("workshop_items.json");
    let refused = |extra: &[&str], variables: &[(&str, &str)]| {
        let args = by_fields(folder, &items, extra);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, stdout, stderr) = steamship(&args, Some(home.path()), variables);
        assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
        failure(&stderr).to_owned()
    };

    assert_eq!(
        refused(&["--account", "build_bot"], &[]),
        "refused: a new item needs --visibility, which steamship never chooses for it"
    );
    assert_eq!(
        refused(
            &["--account", "build_bot", "--visibility", "public"],
            &[("CI", "true")]
        ),
        format!(
            "refused: {} has no item example, so every CI run would make a new Workshop item. \
             Make it outside CI and commit the file, or pass --new to make one",
            items.display()
        )
    );
    fs::write(&items, r#"{"app": 1000050, "items": {"example": 777}}"#).unwrap();
    assert_eq!(
        refused(&[], &[]),
        format!("{}: is for app 1000050, not app 480", items.display())
    );
    fs::write(&items, "[]").unwrap();
    assert_eq!(
        refused(&[], &[]),
        format!("{}: \"app\" is not an app ID", items.display())
    );

    let unnamed: Vec<String> = by_fields(folder, &items, &[])
        .into_iter()
        .map(|arg| if arg == "example" { String::new() } else { arg })
        .collect();
    let (unnamed_code, _, unnamed_stderr) = steamship(
        &unnamed.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(home.path()),
        &[],
    );
    assert_eq!(unnamed_code, Some(2_i32), "{unnamed_stderr}");
    assert_eq!(failure(&unnamed_stderr), "--name is empty");

    let climbing: Vec<String> = by_fields(folder, &items, &[])
        .into_iter()
        .map(|arg| {
            if arg == "example" {
                "../hats".to_owned()
            } else {
                arg
            }
        })
        .collect();
    let (climbing_code, _, climbing_stderr) = steamship(
        &climbing.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(home.path()),
        &[],
    );
    assert_eq!(climbing_code, Some(2_i32), "{climbing_stderr}");
    assert_eq!(
        failure(&climbing_stderr),
        "--name ../hats may hold only letters, digits, '-', '_' and '.', and not begin with '.'"
    );

    let unreadable = by_fields(folder, temp.path(), &[]);
    let (unreadable_code, _, unreadable_stderr) = steamship(
        &unreadable.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(home.path()),
        &[],
    );
    assert_eq!(unreadable_code, Some(2_i32), "{unreadable_stderr}");
    assert!(
        failure(&unreadable_stderr).starts_with(&format!("{}: ", temp.path().display())),
        "{unreadable_stderr}"
    );
}

#[test]
fn workshop_names_each_field_an_item_lacks_and_each_flag_an_item_script_cannot_go_with() {
    let home = tempfile::tempdir().unwrap();
    let said = |args: &[&str]| {
        let (code, stdout, stderr) = steamship(
            &[["workshop"].as_slice(), args].concat(),
            Some(home.path()),
            &[],
        );
        assert_eq!(
            (code, stdout.as_str()),
            (Some(2_i32), ""),
            "{args:?}: {stderr}"
        );
        stderr
    };
    let lacking = "error: the following required arguments were not provided:\n";
    let lacks = |stderr: &str, flags: &[&str]| {
        let listed: Vec<&str> = stderr
            .strip_prefix(lacking)
            .unwrap_or_else(|| panic!("{stderr}"))
            .lines()
            .take_while(|line| line.starts_with("  --"))
            .map(str::trim)
            .collect();
        let mut wanted = flags.to_vec();
        wanted.sort_unstable();
        let mut found = listed;
        found.sort_unstable();
        assert_eq!(found, wanted, "{stderr}");
    };
    let all = ["--items <FILE>", "--name <NAME>", "--content <FOLDER>"];

    lacks(&said(&["480", "--title", "T"]), &all);
    lacks(&said(&["item.vdf", "--title", "T"]), &all);
    lacks(
        &said(&[
            "480",
            "--items",
            "i.json",
            "--content",
            "hat",
            "--title",
            "T",
        ]),
        &["--name <NAME>"],
    );
    lacks(
        &said(&["480", "--items", "i.json"]),
        &["--name <NAME>", "--content <FOLDER>", "--title <TITLE>"],
    );
    for field in [
        ["--content", "hat"],
        ["--preview-image", "hat.png"],
        ["--description", "Green."],
        ["--change-note", "1.0.0"],
    ] {
        let alone = said(&["480", field[0], field[1]]);
        assert!(
            alone.starts_with(lacking) && alone.contains("\n  --items <FILE>\n"),
            "{field:?}: {alone}"
        );
    }

    let (_temp, script) = workshop_item("");
    let word = script.to_str().unwrap();
    for (flag, value) in [("--name", "example"), ("--visibility", "private")] {
        let (code, _, beside) = steamship(&["workshop", word, flag, value], Some(home.path()), &[]);
        assert_eq!(code, Some(2_i32), "{beside}");
        assert_eq!(
            failure(&beside),
            format!(
                "{flag} does not go with the item script {word}. Drop {flag}, or leave out the \
                 script to use it for the game's own mods"
            ),
            "{beside}"
        );
    }
}

#[test]
fn a_word_with_no_field_is_an_item_script_or_the_games_app() {
    let home = tempfile::tempdir().unwrap();
    let note = |word: &str| {
        format!(
            "{word} is not an item script. Give the path of one, or the app's ID first, then \
             `--items`, `--name`, `--content` and `--title`. In a game's folder, leave the word \
             out to upload the example mods"
        )
    };

    let exits_2 = |run: (Option<i32>, String, String)| {
        assert_eq!(run.0, Some(2_i32), "{}{}", run.1, run.2);
        (run.1, run.2)
    };

    let (_, misspelt) = exits_2(steamship(
        &["workshop", "item.vfd", "--account", "build_bot"],
        Some(home.path()),
        &[],
    ));
    assert!(
        failure(&misspelt).starts_with("refused: item.vfd: "),
        "{misspelt}"
    );
    assert!(misspelt.contains(&note("item.vfd")), "{misspelt}");

    let (_temp, script) = workshop_item("");
    let text = fs::read_to_string(&script).unwrap();
    let cut = script.with_file_name("cut.vdf");
    let (head, _) = text.split_once('{').unwrap();
    fs::write(&cut, format!("{head}{{")).unwrap();
    let (_, damaged) = exits_2(steamship(
        &["workshop", cut.to_str().unwrap(), "--account", "build_bot"],
        Some(home.path()),
        &[],
    ));
    assert!(
        failure(&damaged).starts_with(&format!("refused: {}: ", cut.display())),
        "{damaged}"
    );
    assert!(
        damaged.contains(&note(&cut.display().to_string())),
        "{damaged}"
    );

    let (_game, root) = pushed_game(None);
    let (_, strange) = exits_2(in_game(&root, &["workshop", "480"]));
    let refusal = failure(&strange);
    assert!(
        refusal.starts_with("app 480 is not this game's: ")
            && refusal.ends_with(
                " names app 1000050. Give 1000050, or the path of a script that names 480"
            ),
        "{strange}"
    );
    let (own_said, own_why) = exits_2(in_game(&root, &["workshop", "1000050"]));
    assert!(
        own_said.contains("as origin's main has it"),
        "{own_said}{own_why}"
    );
    assert!(!own_why.contains("is not this game's"), "{own_why}");
}

#[test]
fn fields_take_the_app_first_or_the_games_own() {
    let home = tempfile::tempdir().unwrap();
    let (temp, script) = workshop_item("");
    let folder = script.parent().unwrap();
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(folder, &items, &["--account", "build_bot"]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  app       480\n"), "{stdout}{stderr}");

    let (_game, root) = pushed_game(None);
    let (own_code, own_said, own_why) = in_game(&root, &own_args(&args));
    assert_eq!(own_code, Some(2_i32), "{own_said}{own_why}");
    assert!(
        own_said.contains("  app       1000050\n"),
        "{own_said}{own_why}"
    );

    let beside: Vec<&str> = args
        .iter()
        .enumerate()
        .map(|(at, arg)| {
            if at == 1 {
                script.to_str().unwrap()
            } else {
                arg
            }
        })
        .collect();
    let (beside_code, _, beside_why) = steamship(&beside, Some(home.path()), &[]);
    assert_eq!(beside_code, Some(2_i32), "{beside_why}");
    assert!(
        failure(&beside_why).contains("there is no \"AppBuild\" block"),
        "{beside_why}"
    );
    assert!(
        beside_why.contains("the first word names the app, by its ID or its app build script"),
        "{beside_why}"
    );
}

/// `args` without the app after `workshop`.
fn own_args<'args>(args: &[&'args str]) -> Vec<&'args str> {
    args.iter()
        .enumerate()
        .filter(|&(at, _)| at != 1)
        .map(|(_, arg)| *arg)
        .collect()
}

#[test]
fn an_item_script_with_a_real_fault_shows_that_fault_without_the_note_on_the_word() {
    let (_temp, script) = workshop_item(r#""visibility" "9""#);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(
        &[
            "workshop",
            script.to_str().unwrap(),
            "--account",
            "build_bot",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        failure(&stderr).contains("\"visibility\" is not 0 (public)"),
        "{stderr}"
    );
    assert!(!stderr.contains("is not an item script"), "{stderr}");
}

#[test]
fn a_workshop_script_that_would_upload_the_wrong_thing_is_refused_and_exits_2() {
    let (_temp, script) = workshop_item(r#""visibility" "9""#);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(
        &[
            "workshop",
            script.to_str().unwrap(),
            "--account",
            "build_bot",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert_eq!(
        failure(&stderr),
        format!(
            "refused: {}: \"visibility\" is not 0 (public), 1 (friends only), 2 (private) or 3 \
             (unlisted)",
            script.display()
        )
    );
}

#[test]
fn a_games_workshop_items_go_up_only_from_a_pushed_tree() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["workshop", "--name", "elsewhere"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "the game has no example mod named elsewhere"
    );

    fs::write(root.join("modding").join("example").join("new.txt"), "x").unwrap();
    let (unpushed, said, why) = in_game(&root, &["workshop"]);
    assert_eq!(unpushed, Some(2_i32), "{said}{why}");
    assert!(failure(&why).contains("(modding/example/new.txt)"), "{why}");
}

#[test]
fn a_games_example_mods_are_its_workshop_items_and_one_without_any_is_refused() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["workshop", "--visibility", "public"]);
    assert_ne!(code, Some(0_i32), "no steamcmd here: {stdout}{stderr}");
    assert!(
        stdout.contains("as origin's main has it"),
        "{stdout}{stderr}"
    );

    fs::remove_dir_all(root.join("modding")).unwrap();
    pushed(&root, "No example mods");
    let (none, said, why) = in_game(&root, &["workshop", "--json"]);
    assert_eq!(none, Some(2_i32), "{said}{why}");
    assert!(
        failure(&why).starts_with("the game has no example mod: no folder in "),
        "{why}"
    );
}

#[test]
fn a_games_workshop_run_gives_its_visibility_only_to_the_items_it_makes() {
    let (_temp, root) = pushed_game(None);
    let fresh = root.join("modding").join("fresh");
    fs::create_dir_all(&fresh).unwrap();
    fs::write(
        fresh.join("manifest.json"),
        r#"{"name": "Fresh Harbour", "version": "1.0.0", "description": "A mod not yet made."}"#,
    )
    .unwrap();
    fs::write(fresh.join("preview.png"), icon_png(256, 40)).unwrap();
    fs::write(
        root.join("steam").join("workshop_items.json"),
        r#"{"app": 1000050, "items": {"example": 5674}}"#,
    )
    .unwrap();
    pushed(&root, "A second example mod");
    // As `publishing` does, but it keeps each script as it was sent before giving a new item its
    // ID.
    let home = faked_with(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         cp \"$script\" \"$script.sent\"\n\
         grep -q publishedfileid \"$script\" || \
         printf '\"workshopitem\"\\n{\\n\"appid\" \"1000050\"\\n\"publishedfileid\" \"777\"\\n}\\n' > \"$script\"\n\
         printf '%s\\r\\n' 'Success.'\n",
    );
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let copy = |name: &str| {
        fs::read_to_string(
            home.path()
                .join(format!("workshop/1000050/{name}/workshop_item.vdf.sent")),
        )
        .unwrap()
    };
    let published = copy("example");
    assert!(
        published.contains("\"publishedfileid\"\t\t\"5674\"") && !published.contains("visibility"),
        "a published item keeps the visibility it has: {published}"
    );
    let made = copy("fresh");
    assert!(made.contains("\"visibility\"\t\t\"2\""), "{made}");
}

/// A steamcmd for Tidal Mill Inn that, asked for the app's info, prints it with `release` among
/// its common settings, and otherwise publishes the Workshop item it is sent, adding a line to
/// `sent` in the home.
fn workshop_released_as(release: &str) -> String {
    format!(
        "#!/bin/sh\n\
         case \" $* \" in *' +app_info_print '*)\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID : 1000050, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
         printf '\"1000050\"\\n{{\\n\\t\"common\"\\n\\t{{\\n\\t\\t\"name\"\\t\\t\"Tidal Mill Inn\"\\n'\n\
         printf '{release}\\t}}\\n}}\\n'\n\
         echo 'Unloading Steam API...OK'\n\
         exit 0;;\n\
         esac\n\
         echo sent >> \"$HOME/sent\"\n\
         printf '%s\\r\\n' 'Success.'\n"
    )
}

/// Runs `steamship workshop` with `args` in Tidal Mill Inn, whose `steamship.toml` holds its
/// Workshop items until launch, and whose steamcmd shows the app's info with `release`: what it
/// exits with, its output, and whether an item was sent.
fn workshop_held(release: &str, args: &[&str]) -> (Option<i32>, String, String, bool) {
    let (_temp, root) = pushed_game(Some("[workshop]\nheld_until_launch = true\n"));
    fs::write(
        root.join("steam").join("workshop_items.json"),
        r#"{"app": 1000050, "items": {"example": 5674}}"#,
    )
    .unwrap();
    pushed(&root, "The example mod is item 5674");
    let home = faked_with(&workshop_released_as(release));
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &[["workshop"].as_slice(), args].concat(),
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    (code, stdout, stderr, home.path().join("sent").exists())
}

#[test]
fn a_game_holding_its_workshop_items_until_launch_sends_none_while_steam_shows_it_unreleased() {
    let state = "\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n";
    let (code, stdout, stderr, sent) = workshop_held(state, &[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(!sent, "{stdout}{stderr}");
    assert!(
        stdout.contains("unreleased, as Steam shows it (unavailable)"),
        "{stdout}"
    );
    let refused = failure(&stderr);
    assert!(
        refused.starts_with("app 1000050 is unreleased, and ")
            && refused.ends_with(
                "steamship.toml holds the game's Workshop items until launch. Give --for-launch \
                 to the run that readies the launch"
            ),
        "{stderr}"
    );

    let (unknown, said, why, went) = workshop_held("", &[]);
    assert_eq!(unknown, Some(2_i32), "{said}{why}");
    assert!(!went, "{said}{why}");
    assert!(
        failure(&why).starts_with("Steam shows no release state for app 1000050, and "),
        "{why}"
    );
}

#[test]
fn a_game_holding_its_workshop_items_until_launch_sends_them_for_launch_or_once_released() {
    let unreleased = "\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n";
    let (code, stdout, stderr, sent) = workshop_held(unreleased, &["--for-launch"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(sent, "{stdout}{stderr}");
    assert!(
        !stdout.contains("asking whether the app is released"),
        "{stdout}"
    );

    let released = "\\t\\t\"ReleaseState\"\\t\\t\"released\"\\n";
    let (later, said, why, went) = workshop_held(released, &[]);
    assert_eq!(later, Some(0_i32), "{said}{why}");
    assert!(went, "{said}{why}");
    assert!(
        said.contains("released, as Steam shows it (released)"),
        "{said}"
    );
}

/// A steamcmd that adds a line to `starts` in the home each time it starts, and publishes the
/// Workshop item it is sent.
const COUNTING: &str = "#!/bin/sh\n\
    echo start >> \"$HOME/starts\"\n\
    printf '%s\\r\\n' 'Success.'\n";

/// How many times the steamcmd of `home` started.
fn starts(home: &Path) -> usize {
    fs::read_to_string(home.join("starts")).map_or(0, |text| text.lines().count())
}

/// Tidal Mill Inn with a second example mod, `fresh`, pushed with `toml` for its `steamship.toml`
/// and `items` for its `steam/workshop_items.json`.
fn two_mods(toml: Option<&str>, items: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let (temp, root) = pushed_game(toml);
    let fresh = root.join("modding").join("fresh");
    fs::create_dir_all(&fresh).unwrap();
    fs::write(
        fresh.join("manifest.json"),
        r#"{"name": "Fresh Harbour", "version": "1.0.0", "description": "A mod not yet made."}"#,
    )
    .unwrap();
    fs::write(fresh.join("preview.png"), icon_png(256, 40)).unwrap();
    if let Some(items) = items {
        fs::write(root.join("steam").join("workshop_items.json"), items).unwrap();
    }
    pushed(&root, "A second example mod");
    (temp, root)
}

#[test]
fn a_dry_run_checks_the_item_and_starts_no_steamcmd() {
    let home = faked_with(COUNTING);
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--dry-run"],
        Some(home.path()),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  item      \u{2713} app 480, item 5674, 1 file, checked\n")
            && stdout.contains("\u{2713} app 480: 1 item checked, nothing uploaded\n"),
        "{stdout}"
    );
    assert_eq!(starts(home.path()), 0, "{stdout}");
}

#[test]
fn a_dry_run_refuses_what_an_upload_would() {
    let home = tempfile::tempdir().unwrap();
    let (_temp, script) = workshop_item("");
    fs::remove_dir_all(script.parent().unwrap().join("hat")).unwrap();
    let run = |extra: &[&str]| {
        let (code, stdout, stderr) = steamship(
            &[&["workshop", script.to_str().unwrap()], extra].concat(),
            Some(home.path()),
            &[],
        );
        assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
        failure(&stderr).to_owned()
    };
    let dry = run(&["--dry-run"]);
    assert!(dry.starts_with("refused: "), "{dry}");
    assert_eq!(dry, run(&["--account", "build_bot"]));
}

#[test]
fn a_dry_run_in_ci_still_refuses_a_new_item_without_new() {
    let home = tempfile::tempdir().unwrap();
    let (_temp, script) = workshop_item("");
    let run = |extra: &[&str]| {
        steamship(
            &[&["workshop", script.to_str().unwrap(), "--dry-run"], extra].concat(),
            Some(home.path()),
            &[("CI", "true")],
        )
    };
    let (code, stdout, stderr) = run(&[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).contains("so every CI run would make a new Workshop item"),
        "{stderr}"
    );
    let (allowed, said, why) = run(&["--new"]);
    assert_eq!(allowed, Some(0_i32), "{said}{why}");
}

#[test]
fn a_dry_run_of_a_games_mods_needs_no_pushed_tree_or_account() {
    let (_temp, root) = two_mods(None, None);
    fs::write(root.join("modding").join("example").join("new.txt"), "x").unwrap();
    let (code, stdout, stderr) =
        in_game(&root, &["workshop", "--dry-run", "--visibility", "private"]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(stdout.matches(", checked\n").count(), 2, "{stdout}");
    assert!(
        stdout.contains("\u{2713} app 1000050: 2 items checked, nothing uploaded\n"),
        "{stdout}"
    );
}

#[test]
fn a_dry_run_of_a_game_held_until_launch_asks_steam_nothing() {
    let (_temp, root) = two_mods(
        Some("[workshop]\nheld_until_launch = true\n"),
        Some(r#"{"app": 1000050, "items": {"example": 5674, "fresh": 5675}}"#),
    );
    let home = faked_with(COUNTING);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--dry-run"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(!stdout.contains("as Steam shows it"), "{stdout}");
    assert_eq!(starts(home.path()), 0, "{stdout}");
}

#[test]
fn a_dry_run_prints_its_outcome_as_json() {
    let home = faked_with(COUNTING);
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--dry-run", "--json"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        stdout,
        "{\"app\":480,\"items\":[{\"item_id\":5674,\"log\":null,\"name\":null,\"outcome\":\"dry-run\"}]}\n"
    );
}

/// Makes the folder steamship keeps `mod_name`'s copy in a file, so that the copy cannot be made.
fn block_copy(home: &Path, mod_name: &str) {
    let folder = home.join("workshop/1000050");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(mod_name), "in the way").unwrap();
}

#[test]
fn a_second_mod_whose_copy_cannot_be_made_is_the_last_item_and_steamcmd_not_run() {
    let (_temp, root) = two_mods(None, None);
    let home = publishing("Success.", 0);
    block_copy(home.path(), "fresh");
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private", "--json"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(at(&line, "/items/0/outcome"), "made", "{line}");
    assert_eq!(text_at(&line, "/items/1/name"), "fresh", "{line}");
    assert_eq!(
        at(&line, "/items/1/outcome"),
        serde_json::Value::Null,
        "{line}"
    );
    assert_eq!(at(&line, "/items/2"), serde_json::Value::Null, "{line}");
    assert_eq!(at(&line, "/error/code"), "steamcmd-not-run", "{line}");
}

#[test]
fn a_first_mod_whose_copy_cannot_be_made_prints_no_json() {
    let (_temp, root) = two_mods(None, None);
    let home = publishing("Success.", 0);
    block_copy(home.path(), "example");
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private", "--json"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn a_workshop_run_whose_login_cannot_be_read_after_it_says_so_in_the_json_message() {
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         printf '%s\\r\\n' '{DENIED}'\n\
         rm -f \"$HOME/Steam/config/config.vdf\"; mkdir -p \"$HOME/Steam/config/config.vdf\"\n\
         exit 1\n"
    ));
    saved_login(home.path());
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap(), "--json"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(at(&line, "/error/code"), "steamcmd-failed", "{line}");
    let message = text_at(&line, "/error/message");
    assert!(
        message.starts_with("none kept, since steamcmd's login could not be read again"),
        "{message}"
    );
    assert!(!message.contains("Access Denied"), "{message}");
}

#[test]
fn two_mods_are_one_line_of_json() {
    let (_temp, root) = two_mods(None, None);
    let home = publishing("Success.", 0);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private", "--json"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(text_at(&line, "/items/0/name"), "example", "{line}");
    assert_eq!(text_at(&line, "/items/1/name"), "fresh", "{line}");
    assert_eq!(at(&line, "/items/2"), serde_json::Value::Null, "{line}");
    assert!(line.get("error").is_none(), "{line}");
}

#[test]
fn a_second_mod_steam_fails_is_the_last_item_and_the_lines_error() {
    let (_temp, root) = two_mods(None, None);
    let third = root.join("modding").join("third");
    fs::create_dir_all(&third).unwrap();
    fs::write(
        third.join("manifest.json"),
        r#"{"name": "Third Harbour", "version": "1.0.0", "description": "A third mod."}"#,
    )
    .unwrap();
    fs::write(third.join("preview.png"), icon_png(256, 40)).unwrap();
    pushed(&root, "A third example mod");
    let home = faked_with(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         if [ -e \"$HOME/second\" ]; then\n\
           printf '%s\\r\\n' 'ERROR! Failed to update workshop item (Access Denied).'\n\
           exit 1\n\
         fi\n\
         touch \"$HOME/second\"\n\
         printf '\"workshopitem\"\\n{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}\\n' > \"$script\"\n\
         printf '%s\\r\\n' 'Success.'\n",
    );
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private", "--json"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let line = one_line(&stdout);
    assert_eq!(at(&line, "/items/0/outcome"), "made", "{line}");
    assert_eq!(at(&line, "/items/1/name"), "fresh", "{line}");
    assert_eq!(
        at(&line, "/items/1/outcome"),
        serde_json::Value::Null,
        "{line}"
    );
    assert_eq!(at(&line, "/items/2"), serde_json::Value::Null, "{line}");
    assert_eq!(line.pointer("/error/code"), Some(&"steamcmd-failed".into()));
}

#[test]
fn a_second_mod_that_is_refused_stops_the_run_before_the_first_goes_up() {
    let (_temp, root) = two_mods(
        None,
        Some(r#"{"app": 1000050, "items": {"example": 5674}}"#),
    );
    let home = faked_with(COUNTING);
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1"), ("CI", "true")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr)
            .contains("has no item fresh, so every CI run would make a new Workshop item"),
        "{stderr}"
    );
    assert_eq!(starts(home.path()), 0, "{stdout}{stderr}");
}

#[test]
fn a_games_mods_ask_for_the_release_once() {
    let (_temp, root) = two_mods(
        Some("[workshop]\nheld_until_launch = true\n"),
        Some(r#"{"app": 1000050, "items": {"example": 5674, "fresh": 5675}}"#),
    );
    let released = "\\t\\t\"ReleaseState\"\\t\\t\"released\"\\n";
    let counted = workshop_released_as(released).replacen(
        "#!/bin/sh\n",
        "#!/bin/sh\necho start >> \"$HOME/starts\"\n",
        1,
    );
    let home = faked_with(&counted);
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(starts(home.path()), 3, "{stdout}{stderr}");
}

#[test]
fn two_new_mods_of_one_run_are_both_kept() {
    let (_temp, root) = two_mods(None, None);
    let home = publishing("Success.", 0);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let kept = fs::read_to_string(root.join("steam").join("workshop_items.json")).unwrap();
    assert!(
        kept.contains("\"example\": 777") && kept.contains("\"fresh\": 777"),
        "{kept}"
    );
}

/// The shell lines of a steamcmd that finds its item script in `script`, and gives the item
/// the ID 777 in it when it had none.
const MAKING: &str = "while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
    grep -q publishedfileid \"$script\" || \
    printf '\"workshopitem\"\\n{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}\\n' > \"$script\"\n";

/// A home whose steamcmd makes the item as [`MAKING`] does, says `Success.`, and then runs `then`.
fn publishing_then(then: &str) -> tempfile::TempDir {
    let home = faked_with(&format!(
        "#!/bin/sh\n{MAKING}printf '%s\\r\\n' 'Success.'\n{then}\n"
    ));
    saved_login(home.path());
    home
}

#[test]
fn an_item_made_from_a_script_before_steamcmd_altered_itself_exits_5() {
    let home = publishing_then("echo >> \"$0\"");
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(5_i32), "{stdout}{stderr}");
    assert!(stdout.contains("Workshop item 777"), "{stdout}");
    assert!(
        stdout.contains(&format!(
            "Add \"publishedfileid\" \"777\" to {} so that later uploads update it",
            script.display()
        )),
        "{stdout}"
    );
}

#[test]
fn a_steamcmd_altered_after_an_item_was_updated_exits_4() {
    let home = publishing_then("echo >> \"$0\"");
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    assert!(stdout.contains("Workshop item 5674"), "{stdout}");
}

#[test]
fn a_steamcmd_altered_after_an_item_was_made_and_kept_exits_4() {
    let home = publishing_then("echo >> \"$0\"");
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    assert!(stdout.contains("Workshop item 777"), "{stdout}");
    assert!(
        fs::read_to_string(&items)
            .unwrap()
            .contains("\"example\": 777"),
        "the item is kept"
    );
}

#[test]
fn a_second_item_that_fails_leaves_the_first_made_and_kept() {
    let (_temp, root) = two_mods(None, None);
    let home = faked_with(&format!(
        "#!/bin/sh\n{MAKING}\
         if [ -e \"$HOME/second\" ]; then\n\
           printf '%s\\r\\n' 'ERROR! Failed to update workshop item (Access Denied).'\n\
           exit 8\n\
         fi\n\
         touch \"$HOME/second\"\n\
         printf '%s\\r\\n' 'Success.'\n"
    ));
    saved_login(home.path());
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["workshop", "--visibility", "private"],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(stdout.contains("Workshop item 777"), "{stdout}");
    let kept = fs::read_to_string(root.join("steam").join("workshop_items.json")).unwrap();
    assert!(kept.contains(": 777"), "{kept}");
}

#[test]
fn a_console_that_cannot_be_saved_still_keeps_the_item_made_and_exits_0() {
    let home = faked_with(&format!(
        "#!/bin/sh\n{MAKING}\
         mkdir \"$(dirname \"$script\")/steamcmd.log\"\n\
         printf '%s\\r\\n' 'Success.'\n"
    ));
    saved_login(home.path());
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(shown.contains("log  none, since"), "{shown}");
    assert!(!shown.contains("could not be removed"), "{shown}");
    assert!(
        fs::read_to_string(&items)
            .unwrap()
            .contains("\"example\": 777"),
        "the item is kept"
    );
}

#[test]
fn an_item_console_that_cannot_be_removed_is_said_on_the_log_line() {
    let home = publishing_then("");
    let blocked = home.path().join("workshop/480/example/steamcmd.log");
    fs::create_dir_all(&blocked).unwrap();
    fs::write(blocked.join("kept"), "in the way").unwrap();
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(shown.contains("log  none, since"), "{shown}");
    assert!(
        shown.contains(". The earlier console could not be removed: "),
        "{shown}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_login_unreadable_after_an_item_was_made_still_keeps_it() {
    let home = publishing_then("chmod 000 \"$HOME/Steam/config/config.vdf\"");
    let (temp, script) = workshop_item("");
    let items = temp.path().join("workshop_items.json");
    let args = by_fields(
        script.parent().unwrap(),
        &items,
        &["--visibility", "private"],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        fs::read_to_string(&items)
            .unwrap()
            .contains("\"example\": 777"),
        "the item is kept"
    );
    assert!(
        !home
            .path()
            .join("workshop/480/example/steamcmd.log")
            .exists(),
        "no console is saved"
    );
}

#[test]
fn a_workshop_reason_that_holds_a_secret_is_printed_redacted() {
    let home = publishing("ERROR! Failed to update workshop item (Access Denied).", 8);
    let secret = "Failed to update workshop item";
    let local = home
        .path()
        .join("Steam/userdata/12345/config/localconfig.vdf");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(
        local,
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{secret}\" }} }}\n"),
    )
    .unwrap();
    let (_temp, script) = workshop_item(r#""publishedfileid" "5674""#);
    for json in [false, true] {
        let mut args = vec!["workshop", script.to_str().unwrap()];
        args.extend(json.then_some("--json"));
        let (code, stdout, stderr) = steamship(&args, Some(home.path()), &[]);
        assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
        assert!(stderr.contains("[redacted] (Access Denied)."), "{stderr}");
        assert!(!stdout.contains(secret) && !stderr.contains(secret));
    }
}

#[test]
fn a_refused_workshop_login_never_shows_a_secret_of_its_reason_when_the_login_is_unreadable_after()
{
    let secret = "Expired Login Auth Code";
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' to Steam Public...FAILED ({secret})\"\n\
         rm -f \"$HOME/Steam/config/config.vdf\"; mkdir -p \"$HOME/Steam/config/config.vdf\"\n\
         exit 5\n"
    ));
    saved_login(home.path());
    let local = home
        .path()
        .join("Steam/userdata/12345/config/localconfig.vdf");
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(
        local,
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{secret}\" }} }}\n"),
    )
    .unwrap();
    let (_temp, script) = workshop_item("");
    let (code, stdout, stderr) = steamship(
        &["workshop", script.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert!(!stdout.contains(secret) && !stderr.contains(secret));
}
