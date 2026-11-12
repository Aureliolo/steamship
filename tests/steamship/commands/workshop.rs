//! `steamship workshop`.

use std::fs;
use std::path::Path;

use steamship::platform::Platform;
use steamship::steamcmd;

use crate::{
    failure, faked_with, icon_png, in_game, packed_login, pushed, pushed_game, saved_login,
    steamship, steamship_in, workshop_item,
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
    assert_eq!(
        fs::read_to_string(&outputs).unwrap(),
        "published-file-id=777\n"
    );
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
    let (_temp, script) = workshop_item("");
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

/// `workshop` for the item at `folder`, as `workshop_item` lays it out, given by its fields and
/// kept in `items` as `example`, with `extra` after them.
fn by_fields(folder: &Path, items: &Path, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "workshop",
        "--app",
        "480",
        "--items",
        items.to_str().unwrap(),
        "--name",
        "example",
        "--content",
        folder.join("hat").to_str().unwrap(),
        "--title",
        "A green hat",
        "--preview",
        folder.join("hat.png").to_str().unwrap(),
    ]
    .map(str::to_owned)
    .into();
    args.extend(extra.iter().map(|&arg| arg.to_owned()));
    args
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
                "item": 777_u64,
                "log": log.to_str().unwrap(),
                "name": "example",
                "outcome": outcome,
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
fn an_item_made_whose_id_cannot_be_kept_says_what_to_keep_by_hand_and_exits_1() {
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
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
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
fn workshop_names_each_flag_a_script_cannot_go_with_and_each_one_an_items_fields_lack() {
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
    let fields = [
        ["--app", "480"],
        ["--items", "items.json"],
        ["--content", "hat"],
        ["--preview", "hat.png"],
        ["--title", "A green hat"],
        ["--description", "Green."],
        ["--change-note", "1.0.0"],
    ];
    let also = [["--name", "example"], ["--visibility", "private"]];

    let every = said(&[["item.vdf"].as_slice(), &fields.concat(), &also.concat()].concat());
    assert!(
        every.starts_with(
            "error: the argument '[SCRIPT]' cannot be used with:\n  --app <APP>\n  --items \
             <FILE>\n  --content <FOLDER>\n  --preview <IMAGE>\n  --title <TITLE>\n  \
             --description <TEXT>\n  --change-note <TEXT>\n  --name <NAME>\n  --visibility \
             <VISIBILITY>\n\n"
        ),
        "{every}"
    );
    let one = said(&["item.vdf", "--app", "480"]);
    assert!(
        one.starts_with("error: the argument '[SCRIPT]' cannot be used with '--app <APP>'\n\n"),
        "{one}"
    );

    let lacking = "error: the following required arguments were not provided:\n";
    let few = said(&["--app", "480", "--name", "example"]);
    assert!(
        few.starts_with(&format!(
            "{lacking}  --items <FILE>\n  --content <FOLDER>\n  --title <TITLE>\n\n"
        )),
        "{few}"
    );
    let unnamed = said(&[
        "--app",
        "480",
        "--items",
        "items.json",
        "--content",
        "hat",
        "--title",
        "A green hat",
    ]);
    assert!(
        unnamed.starts_with(&format!("{lacking}  --name <NAME>\n\n")),
        "{unnamed}"
    );
    for field in &fields[1..] {
        let alone = said(field);
        assert!(
            alone.starts_with(lacking) && alone.contains("\n  --app <APP>\n\n"),
            "{field:?}: {alone}"
        );
    }
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
