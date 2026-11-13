//! `steamship check`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::common;
use crate::common::pe::settings_ticking;
use crate::common::project::Project;
use steamship::run;
use steamship::scripts::Problem;
use steamship::webapi::STAND_IN;

#[cfg(unix)]
use crate::write_program;
use crate::{
    BETAS, BUILDS, DEMO_TYPED, INN_BOARDS, KEY, NEEDS_VC, failure, in_game, in_game_with,
    presence_file, pushed, pushed_game, refusing_web_api, shipping_dll, steamship, steamship_in,
    web_api, web_api_variables, with_demo,
};

/// An app script for Spacewar's content that sets `branch` live, in a folder of its own.
fn setting_live(branch: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("spacewar");
    let script = folder.path().join("app_build.vdf");
    fs::write(
        &script,
        format!(
            "\"AppBuild\"\n{{\n\t\"AppID\" \"480\"\n\t\"ContentRoot\" \"{}\"\n\t\"SetLive\" \
             \"{branch}\"\n\t\"Depots\" {{ \"481\" \"{}\" }}\n}}\n",
            fixture.join("content").display(),
            fixture.join("steam").join("depot_build.vdf").display()
        ),
    )
    .unwrap();
    (folder, script)
}

#[test]
fn check_asks_steam_whether_the_live_branch_exists_when_a_key_is_at_hand() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BETAS)]);
    let set = web_api_variables(&host);

    let (_there, found) = setting_live("Testing");
    let found = found.to_string_lossy().into_owned();
    let (passed, said, _) = steamship(&["check", &found], Some(home.path()), &set);
    assert_eq!(passed, Some(0_i32), "{said}");
    assert!(said.contains("Testing, found on Steam"), "{said}");

    let (_missing, missing) = setting_live("beta2");
    let missing = missing.to_string_lossy().into_owned();
    let (refused, _, why) = steamship(&["check", &missing], Some(home.path()), &set);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "app_build.vdf: \"SetLive\" names \"beta2\", but app 480 has no branch by that name. \
             Create the branch in Steamworks under SteamPipe, Builds first"
        ),
        "{why}"
    );
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamApps/GetAppBetas/v1/?") && asked.contains("appid=480"),
        "{asked}"
    );
}

#[test]
fn check_refuses_a_web_api_key_that_is_not_one_without_repeating_it() {
    let home = tempfile::tempdir().unwrap();
    let (_there, script) = setting_live("beta2");
    let script = script.to_string_lossy().into_owned();
    let (code, stdout, stderr) = steamship(
        &["check", &script],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", "not_a_key_1234"),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_WEB_API_KEY does not hold a publisher Web API key, which is 32 hexadecimal \
         digits"
    );
    assert!(!format!("{stdout}{stderr}").contains("not_a_key"));
}

#[test]
fn check_goes_on_when_steam_cannot_say_or_no_key_is_at_hand() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let (_folder, script) = setting_live("beta2");
    let script = script.to_string_lossy().into_owned();
    let (unanswered, said, _) = steamship(
        &["check", &script],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(unanswered, Some(0_i32), "{said}");
    assert!(said.contains("\u{2717} not checked"), "{said}");

    // An app the key holds with no build live on its default branch, such as a new demo's: Steam
    // cannot say yet, which is no fault of the script's.
    let held = r#"{"applist": {"apps": {"app": [
        {"appid": 480, "app_type": "game", "app_name": "Spacewar"}
    ]}}}"#;
    let (holding, _asked) = web_api(vec![("500 Internal Server Error", ""), ("200 OK", held)]);
    let (unbuilt, new, _) = steamship(
        &["check", &script],
        Some(home.path()),
        &web_api_variables(&holding),
    );
    assert_eq!(unbuilt, Some(0_i32), "{new}");
    assert!(
        new.contains("  branch    beta2, not on Steam yet\n"),
        "{new}"
    );
    assert!(!new.contains('\u{2717}'), "{new}");
    assert!(
        new.contains(
            "app 480 has no build live on its default branch, and Steam gives it no other branch \
             until it has. An upload with --default sets the build live on default alone"
        ),
        "{new}"
    );

    // No key set or kept: nothing is asked of Steam, and the branch is said to be unchecked, with
    // what that risks and how to have it checked.
    let (unasked, _never) = web_api::<&str>(vec![]);
    let (keyless, warned, _) = steamship(
        &["check", &script],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1"), (STAND_IN, &unasked)],
    );
    assert_eq!(keyless, Some(0_i32), "{warned}");
    assert!(
        warned.contains("  branch    beta2, not checked: no Web API key at hand\n"),
        "{warned}"
    );
    assert!(
        warned.contains(
            "leaves the build uploaded but not live. steamship login --web-api-key-only keeps"
        ),
        "{warned}"
    );
}

#[test]
fn check_names_what_a_redistributable_installs_and_says_whether_the_app_ticks_it_was_not_checked() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], None);
    let script = script.to_string_lossy().into_owned();
    let (code, said, _) = steamship(
        &["check", &script],
        Some(home.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(0_i32), "{said}");
    for line in [
        "  redist    1 DLL, not checked\n",
        NEEDS_VC,
        "upload asks Steam which redistributables the app ticks, and check reads them from a \
         settings snapshot beside the script",
    ] {
        assert!(said.contains(line), "{line:?} in {said}");
    }
}

#[test]
fn check_reads_which_redistributables_the_app_ticks_from_the_snapshot_beside_the_script() {
    let home = tempfile::tempdir().unwrap();
    let checked = |snapshot: &str| {
        let (_project, script) =
            shipping_dll(&["kernel32.dll", "vcruntime140.dll"], Some(snapshot));
        steamship(
            &["check", &script.to_string_lossy()],
            Some(home.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        )
    };

    let (passed, said, _) = checked(&settings_ticking(&["228989"]));
    assert_eq!(passed, Some(0_i32), "{said}");
    assert!(
        said.contains("  redist    \u{2713} VC 2022 Redist, ticked in Steamworks\n"),
        "{said}"
    );

    let (refused, _, why) = checked(&settings_ticking(&[]));
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!(
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks. Tick one under Installation, \
             Redistributables, ship the DLL in a depot, or link its library in statically"
        )),
        "{why}"
    );

    let (unreadable, _, broken) = checked("\"1000\" {");
    assert_eq!(unreadable, Some(2_i32), "{broken}");
    assert!(broken.contains("settings.vdf: "), "{broken}");
}

/// `steamship check` on the build folder of a game whose `game.dll` needs Visual C++ and whose
/// snapshot is `snapshot`, run in `folder`, or else in the game's own. The folder's path comes
/// with it.
fn folder_checked(
    snapshot: &str,
    folder: Option<&Path>,
    extra: &[&str],
) -> ((Option<i32>, String, String), PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], Some(snapshot));
    let root = script.parent().unwrap().parent().unwrap().to_path_buf();
    let content = root.join("content");
    let mut args = vec!["check", content.to_str().unwrap()];
    args.extend(extra);
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let ran = steamship_in(folder.unwrap_or(&root), &args, Some(home.path()), &quiet);
    (ran, content)
}

#[test]
fn check_takes_a_build_s_folder_alone_naming_what_a_redistributable_installs() {
    let elsewhere = tempfile::tempdir().unwrap();
    let ((code, said, why), content) =
        folder_checked(&settings_ticking(&[]), Some(elsewhere.path()), &[]);
    assert_eq!(code, Some(0_i32), "{why}");
    assert_eq!(
        said,
        format!(
            "steamship check\n  \
             folder    {}\n  \
             build     \u{2713} 2 files, 1 for Windows, checked\n  \
             redist    1 DLL, not checked\n    \
             {}\n    \
             upload asks Steam which redistributables the app ticks, and check reads them from \
             a settings snapshot beside the script\n  \
             \u{2713} nothing refused\n",
            content.display(),
            Problem::new(
                &content.join("game.dll"),
                "imports vcruntime140.dll, which VC 2015 Redist, VC 2017 Redist, VC 2019 Redist \
                 or VC 2022 Redist installs"
            ),
        ),
        "outside a game, which redistributables are ticked is not known"
    );
    let ((scripts, _, refusal), folder) = folder_checked(
        &settings_ticking(&[]),
        Some(elsewhere.path()),
        &["--scripts-only"],
    );
    assert_eq!(scripts, Some(2_i32));
    assert!(
        refusal.contains(&format!(
            "{} is a build's folder, which holds no scripts. Give --scripts-only the app build \
             script",
            folder.display()
        )),
        "{refusal}"
    );
}

#[test]
fn check_judges_a_build_s_folder_in_a_game_by_the_redistributables_its_snapshot_ticks() {
    let ((refused, _, why), _) = folder_checked(&settings_ticking(&[]), None, &[]);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!(
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks. Tick one under Installation, \
             Redistributables, ship the DLL in a depot, or link its library in statically"
        )),
        "{why}"
    );
    let ((passed, said, _), _) = folder_checked(&settings_ticking(&["228989"]), None, &[]);
    assert_eq!(passed, Some(0_i32), "{said}");
    assert!(
        said.contains("  redist    \u{2713} VC 2022 Redist, ticked in Steamworks\n"),
        "{said}"
    );
}

#[test]
fn check_of_a_build_s_folder_in_a_game_reads_its_rich_presence_offline() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = shipping_dll(
        &["kernel32.dll", "vcruntime140.dll"],
        Some(&settings_ticking(&["228989"])),
    );
    let steam = script.parent().unwrap();
    let root = steam.parent().unwrap();
    let content = root.join("content");
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let check = ["check", content.to_str().unwrap()];
    drop(presence_file(
        steam,
        "english",
        "\t\t\"#menu\"\t\"At the menu\"\n",
    ));
    // The file's tail alone, as macOS gives the child its temporary folder through /private.
    let english = Path::new("steam").join("rich_presence_english.vdf");

    let (passed, said, stderr) = steamship_in(root, &check, Some(home.path()), &quiet);
    assert_eq!(passed, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains("  presence  \u{2713} 1 rich presence file, checked\n"),
        "{said}"
    );

    drop(presence_file(
        steam,
        "english",
        "\t\t\"#menu\"\t\"Charter {0}\"\n",
    ));
    let (refused, _, why) = steamship_in(root, &check, Some(home.path()), &quiet);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!(
            "{}: #menu: \"{{0}}\" is a slot Steam cannot fill: Steam fills %field% and \
             {{#Token_%field%}} alone",
            english.display()
        )),
        "{why}"
    );
}

#[test]
fn an_app_id_must_be_the_one_the_games_script_names() {
    let (_temp, root) = pushed_game(None);
    for args in [["check", "480"].as_slice(), ["upload", "480"].as_slice()] {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        let refusal = failure(&stderr);
        let script = refusal
            .strip_prefix("app 480 is not this game's: ")
            .and_then(|rest| {
                rest.strip_suffix(
                    " names app 1000050. Give 1000050, or the path of a script that names 480",
                )
            })
            .unwrap_or_else(|| panic!("{args:?}: {refusal}"));
        assert!(
            Path::new(script).ends_with(Path::new("steam").join("app_build.vdf")),
            "{args:?}: {refusal}"
        );
    }
    let (code, stdout, stderr) = in_game(&root, &["check", "1000050", "--scripts-only"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
}

#[test]
fn a_game_whose_script_names_app_0_is_refused_by_check_and_upload() {
    let (_temp, root) = pushed_game(None);
    fs::write(
        root.join("steam").join("app_build.vdf"),
        r#""AppBuild" { "AppID" "0" "ContentRoot" "../export/" "SetLive" "testing" "Depots" { "1000051" "depot.vdf" } }"#,
    )
    .unwrap();
    pushed(&root, "No app");
    for args in [["check"].as_slice(), ["upload"].as_slice()] {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        assert!(
            stderr.contains("\"AppID\" is 0, which Steamworks never gives"),
            "{args:?}: {stderr}"
        );
    }

    let (built, said, why) = in_game(&root, &["check", "export"]);
    assert_eq!(
        built,
        Some(0_i32),
        "the build is checked before the app exists: {why}"
    );
    assert!(
        said.lines().any(|line| line
            .strip_prefix("  app       not named yet in ")
            .is_some_and(
                |script| Path::new(script).ends_with(Path::new("steam").join("app_build.vdf"))
            )),
        "{said}"
    );
    assert!(said.contains("  \u{2713} nothing refused\n"), "{said}");
}

#[test]
fn a_game_check_compares_what_the_game_holds_with_steam_and_lists_its_builds() {
    let (_temp, root) = pushed_game(None);
    fs::remove_file(root.join("steam").join("settings.vdf")).unwrap();
    fs::write(
        root.join("mods")
            .join("base")
            .join("data")
            .join("boards.json"),
        INN_BOARDS,
    )
    .unwrap();
    let host = refusing_web_api(12);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(
        code,
        Some(1_i32),
        "Steam refused every question: {stdout}{stderr}"
    );
    for result in [
        "  app       \u{2713} 1000050, 1 depot, scripts checked\n",
        "  branch    \u{2717} not checked\n",
        "  steam     \u{2717} key refused\n",
    ] {
        assert!(stdout.contains(result), "{result} in {stdout}{stderr}");
    }
    for section in [
        "steamship check\n",
        "steamship achievements\n",
        "steamship leaderboards\n",
        "steamship rich-presence, dry run\n",
        "steamship builds\n",
    ] {
        assert!(stdout.contains(section), "{section} in {stdout}{stderr}");
    }
}

#[test]
fn a_game_check_names_each_comparison_the_game_gives_nothing_for() {
    let (_temp, root) = pushed_game(None);
    fs::remove_dir_all(root.join("mods")).unwrap();
    fs::remove_file(root.join("steam").join("rich_presence_english.vdf")).unwrap();
    let host = refusing_web_api(4);
    let (_code, stdout, stderr) = in_game_with(
        &root,
        &["check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert!(
        stdout.contains("  content   not checked (upload checks it once the game is built)\n"),
        "{stdout}"
    );
    // The folder's tail alone, as macOS gives the child its temporary folder through /private.
    let data = Path::new("inn").join("mods").join("base").join("data");
    let script = Path::new("inn").join("steam").join("app_build.vdf");
    for (start, end) in [
        (
            "steamship achievements\n  unchecked no achievement, stat or set records in ",
            format!(
                "{}. List the game's under `records` in steamship.toml",
                data.display()
            ),
        ),
        (
            "steamship leaderboards\n  unchecked no leaderboard records in ",
            data.display().to_string(),
        ),
        (
            "steamship rich-presence\n  unchecked no rich_presence_<language>.vdf beside ",
            script.display().to_string(),
        ),
    ] {
        let line = stdout
            .split_once(start)
            .and_then(|(_, after)| after.lines().next())
            .unwrap_or_default();
        assert!(line.ends_with(&end), "{start}…{end} in {stdout}{stderr}");
    }
}

#[test]
fn a_game_check_refuses_a_rich_presence_line_steam_cannot_fill() {
    let (_temp, root) = pushed_game(None);
    drop(presence_file(
        &root.join("steam"),
        "english",
        "\t\t\"#menu\"\t\"Leading {guild}\"\n",
    ));
    let host = refusing_web_api(12);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_ne!(code, Some(0_i32), "{stdout}{stderr}");
    let file = Path::new("steam").join("rich_presence_english.vdf");
    assert!(
        stderr.contains(&format!(
            "{}: #menu: \"{{guild}}\" is a slot Steam cannot fill: Steam fills %field% and \
             {{#Token_%field%}} alone",
            file.display()
        )),
        "{stdout}{stderr}"
    );
}

#[test]
fn a_demo_goes_up_as_its_own_app_typed_demo_under_the_full_game_and_never_into_it() {
    let (_temp, root) = pushed_game(None);
    let (none, said, why) = in_game(&root, &["check", "--demo"]);
    assert_eq!(none, Some(2_i32), "{said}{why}");
    assert!(
        failure(&why).starts_with(
            "the game has no demo. Write its app build script to steam/demo/app_build.vdf"
        ),
        "{why}"
    );

    let demo = root.join("steam").join("demo");
    fs::create_dir_all(&demo).unwrap();
    fs::write(
        demo.join("app_build.vdf"),
        r#""AppBuild" { "AppID" "0" "ContentRoot" "../../export/" "SetLive" "testing" "Depots" { "0" "../depot.vdf" } }"#,
    )
    .unwrap();
    pushed(&root, "A demo whose app is still to be made");
    let (unmade, placeholder, refusal) = in_game(&root, &["check", "--demo"]);
    assert_eq!(unmade, Some(2_i32), "{placeholder}{refusal}");
    assert!(
        refusal.contains("\"AppID\" is 0, which Steamworks never gives"),
        "{refusal}"
    );
    let (full, shipping, halted) = in_game(&root, &["upload"]);
    assert!(
        shipping.contains("  build     none named in steamship.toml"),
        "the full game ships: {shipping}{halted}"
    );
    assert_eq!(full, Some(2_i32), "stopped before steamcmd: {halted}");

    fs::write(
        demo.join("app_build.vdf"),
        r#""AppBuild" { "AppID" "1000050" "ContentRoot" "../../export/" "SetLive" "testing" "Depots" { "1000051" "../depot.vdf" } }"#,
    )
    .unwrap();
    pushed(&root, "A demo in the full game's app");
    for args in [["check", "--demo"].as_slice(), ["upload"].as_slice()] {
        let (crossed, shown, reason) = in_game(&root, args);
        assert_eq!(crossed, Some(2_i32), "{args:?}: {shown}{reason}");
        for line in [
            "names app 1000050, the full game's own in ",
            ". A demo is its own app, so put the demo's ID in it",
            "depot 1000051 is in both ",
            ": a demo ships depots of its own",
        ] {
            assert!(reason.contains(line), "{args:?}: {line:?} in {reason}");
        }
    }

    with_demo(&root, "../../export/", "\t\t\"type\"\t\t\"Game\"\n");
    pushed(&root, "A demo typed as a game");
    let (untyped, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(untyped, Some(2_i32), "{shown}{reason}");
    for line in [
        "app 1000070 is typed Game, not Demo. Set its type to Demo under General Application \
         Settings in Steamworks",
        "app 1000070's settings name no base game. Enter the full game, app 1000050, under \
         General Application Settings in Steamworks",
    ] {
        assert!(reason.contains(line), "{line:?} in {reason}");
    }

    with_demo(&root, "../../export/", DEMO_TYPED);
    pushed(&root, "The demo under the full game");
    let (shipped, building, stopped) = in_game(&root, &["upload", "--demo"]);
    assert!(
        building.contains(
            "  build     none named under [demo] in steamship.toml, so the content goes up as it is"
        ),
        "{building}{stopped}"
    );
    assert_eq!(shipped, Some(2_i32), "stopped before steamcmd: {stopped}");

    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
    let (listed, list, problems) = in_game_with(
        &root,
        &["builds", "--demo"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(listed, Some(0_i32), "{list}{problems}");
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(asked.contains("appid=1000070"), "{asked}");
}

#[test]
fn a_demos_build_folder_is_judged_against_the_demos_own_settings() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/demo/", DEMO_TYPED);
    fs::write(
        root.join("steam").join("demo").join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{DEMO_TYPED}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\
             \t\t\"steamcontrollertemplateindex\"\t\t\"13\"\n\
             \t\t\"steaminputmanifestpath\"\t\t\"steam_input/action_manifest.vdf\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
    let build = root.join("export").join("demo").join("windows");
    fs::create_dir_all(&build).unwrap();
    fs::write(build.join("inn.pck"), "pack").unwrap();
    let folder = Path::new("export").join("demo").join("windows");
    let folder = folder.to_str().unwrap();
    let unshipped = "has no \"steam_input/action_manifest.vdf\", which app 1000070's settings name \
                     as its Steam Input action manifest";
    let snapshot = |said: &str, demo: bool| {
        let mut path = Path::new("steam").to_path_buf();
        if demo {
            path.push("demo");
        }
        said.lines().any(|line| {
            line.strip_prefix("  file      ")
                .is_some_and(|file| Path::new(file).ends_with(path.join("settings.vdf")))
        })
    };

    let (passed, said, why) = in_game(&root, &["check", folder]);
    assert_eq!(
        passed,
        Some(0_i32),
        "the full game's depots ship it too: {said}{why}"
    );
    assert!(snapshot(&said, false), "{said}");

    let (refused, shown, reason) = in_game(&root, &["check", "--demo", folder]);
    assert_eq!(refused, Some(2_i32), "{shown}{reason}");
    assert!(reason.contains(unshipped), "{reason}");
    assert!(snapshot(&shown, true), "{shown}");

    fs::write(
        root.join("steam").join("depot.vdf"),
        r#""DepotBuild" { "DepotID" "1000051" "FileMapping" { "LocalPath" "windows/*" "DepotPath" "." "Recursive" "1" } }"#,
    )
    .unwrap();
    let (alone, judged, unmet) = in_game(&root, &["check", folder]);
    assert_eq!(
        alone,
        Some(2_i32),
        "only the demo's depots ship it: {judged}{unmet}"
    );
    assert!(unmet.contains(unshipped), "{unmet}");

    let (scripted, titled, named) =
        in_game(&root, &["check", "--demo", "steam/demo/app_build.vdf"]);
    assert_eq!(scripted, Some(2_i32), "{titled}{named}");
    assert_eq!(
        failure(&named),
        "steam/demo/app_build.vdf is an app build script, which names its own app. Give --demo a \
         build's folder, or nothing in the game's folder"
    );

    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("build")).unwrap();
    let (gameless, headed, told) = in_game(outside.path(), &["check", "--demo", "build"]);
    assert_eq!(gameless, Some(2_i32), "{headed}{told}");
    assert!(
        failure(&told).ends_with(
            " is in no game's folder, so it has no demo. Run the check in the game's folder, or \
             leave out --demo"
        ),
        "{told}"
    );
}

/// Writes `path` as a program a launch option could start: a script, executable where the system
/// asks for it.
fn put_program(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    #[cfg(unix)]
    write_program(path, "#!/bin/sh\n");
    #[cfg(not(unix))]
    fs::write(path, "#!/bin/sh\n").unwrap();
}

/// The game's settings snapshot with these launch options, each its program, the systems it
/// runs on, and the beta branch it is offered on alone, if any.
fn launching(root: &Path, options: &[(&str, &str, Option<&str>)]) {
    let launch: Vec<String> = options
        .iter()
        .enumerate()
        .map(|(key, (program, systems, branch))| {
            let branch = branch.map_or_else(String::new, |branch| {
                format!("\t\t\t\t\t\"betakey\"\t\t\"{branch}\"\n")
            });
            format!(
                "\t\t\t\"{key}\"\n\t\t\t{{\n\t\t\t\t\"executable\"\t\t\"{program}\"\n\
                 \t\t\t\t\"type\"\t\t\"default\"\n\
                 \t\t\t\t\"config\"\n\t\t\t\t{{\n\t\t\t\t\t\"oslist\"\t\t\"{systems}\"\n{branch}\
                 \t\t\t\t}}\n\t\t\t}}\n"
            )
        })
        .collect();
    fs::write(
        root.join("steam").join("settings.vdf"),
        format!(
            "\"1000050\"\n{{\n\t\"config\"\n\t{{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
             \t\t\"launch\"\n\t\t{{\n{}\t\t}}\n\t}}\n}}\n",
            launch.concat()
        ),
    )
    .unwrap();
}

#[test]
fn a_build_without_the_program_its_launch_option_starts_is_refused() {
    let (_temp, root) = pushed_game(None);
    launching(&root, &[("inn.exe", "windows", None)]);
    let export = root.join("export");
    put_program(&export.join("old-inn.exe"));
    // A library starts as a program does, and a folder of data is no macOS app: neither is named.
    put_program(&export.join("steam_api64.dll"));
    fs::create_dir_all(export.join("data")).unwrap();
    fs::write(export.join("data").join("level.pck"), "pack").unwrap();
    let script = Path::new("steam").join("app_build.vdf");
    let script = script.to_str().unwrap();

    let (refused, said, why) = in_game(&root, &["check", script]);
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(
            "starts \"inn.exe\" in app 1000050's launch option 0 on windows, which the build \
             does not install. It installs only \"old-inn.exe\" in the install folder. Name the \
             build's program in the launch option, under Installation, General Installation in \
             Steamworks, and save the settings again. Or ship the program under that name"
        ),
        "{why}"
    );
    let (folder_refused, shown, reason) = in_game(&root, &["check", "export"]);
    assert_eq!(folder_refused, Some(2_i32), "{shown}{reason}");
    assert!(
        reason.contains(
            "is a build of app 1000050 holding \"old-inn.exe\", and none of the programs its \
             launch options start: \"inn.exe\" on windows."
        ),
        "{reason}"
    );

    // Windows finds a program by its name in any case.
    fs::rename(export.join("old-inn.exe"), export.join("INN.EXE")).unwrap();
    let (passed, checked, told) = in_game(&root, &["check", script]);
    assert_eq!(passed, Some(0_i32), "{checked}{told}");
    let (folder_passed, folder_checked, folder_told) = in_game(&root, &["check", "export"]);
    assert_eq!(folder_passed, Some(0_i32), "{folder_checked}{folder_told}");
}

#[test]
fn a_launch_option_naming_no_system_needs_its_program_on_each_system_a_depot_installs_on() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::write(
        steam.join("app_build.vdf"),
        r#""AppBuild" { "AppID" "1000050" "ContentRoot" "../export/" "SetLive" "testing" "Depots" { "1000051" "depot.vdf" "1000052" "linux.vdf" } }"#,
    )
    .unwrap();
    fs::write(
        steam.join("depot.vdf"),
        r#""DepotBuild" { "DepotID" "1000051" "FileMapping" { "LocalPath" "windows/*" "DepotPath" "." "Recursive" "1" } }"#,
    )
    .unwrap();
    fs::write(
        steam.join("linux.vdf"),
        r#""DepotBuild" { "DepotID" "1000052" "FileMapping" { "LocalPath" "linux/*" "DepotPath" "." "Recursive" "1" } }"#,
    )
    .unwrap();
    fs::write(
        steam.join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
         \t\t\"launch\"\n\t\t{\n\t\t\t\"0\"\n\t\t\t{\n\t\t\t\t\"executable\"\t\t\"inn.exe\"\n\
         \t\t\t}\n\t\t}\n\t}\n\t\"depots\"\n\t{\n\
         \t\t\"1000051\"\n\t\t{\n\t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\
         \t\t\t}\n\t\t}\n\
         \t\t\"1000052\"\n\t\t{\n\t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"linux\"\n\
         \t\t\t}\n\t\t}\n\t}\n}\n",
    )
    .unwrap();
    let export = root.join("export");
    put_program(&export.join("windows").join("inn.exe"));
    put_program(&export.join("linux").join("inn.x86_64"));

    let (refused, said, why) = in_game(&root, &["check", "steam/app_build.vdf"]);
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(
            "starts \"inn.exe\" in app 1000050's launch option 0 on linux, which the build does \
             not install. It installs only \"inn.x86_64\" in the install folder."
        ),
        "{why}"
    );
    assert!(
        !why.contains(" on windows"),
        "the Windows depot installs it: {why}"
    );
}

#[test]
fn on_linux_a_launch_options_program_matches_by_case_and_a_macos_app_is_its_folder() {
    let (_temp, root) = pushed_game(None);
    launching(
        &root,
        &[
            ("inn.x86_64", "linux", None),
            ("Inn.app", "macos", None),
            ("experiment.exe", "windows", Some("experimental")),
        ],
    );
    let export = root.join("export");
    put_program(&export.join("Inn.x86_64"));
    fs::create_dir_all(export.join("Inn.app").join("Contents")).unwrap();
    fs::write(
        export.join("Inn.app").join("Contents").join("Info.plist"),
        "<plist/>",
    )
    .unwrap();
    put_program(
        &export
            .join("Inn.app")
            .join("Contents")
            .join("MacOS")
            .join("Inn"),
    );
    fs::create_dir_all(export.join("data")).unwrap();
    fs::write(export.join("data").join("level.pck"), "pack").unwrap();
    let script = Path::new("steam").join("app_build.vdf");
    let script = script.to_str().unwrap();

    let (refused, said, why) = in_game(&root, &["check", script]);
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(
            "starts \"inn.x86_64\" in app 1000050's launch option 0 on linux, which the build \
             does not install. It installs only \"Inn.app\" and \"Inn.x86_64\" in the install \
             folder."
        ),
        "{why}"
    );
    assert!(
        !why.contains("\"Inn.app\" in app") && !why.contains("experiment.exe"),
        "the macOS app is there, and an option for a branch the build does not go live on is \
         not judged: {why}"
    );

    fs::rename(export.join("Inn.x86_64"), export.join("inn.x86_64")).unwrap();
    let (passed, checked, told) = in_game(&root, &["check", script]);
    assert_eq!(passed, Some(0_i32), "{checked}{told}");
}

#[test]
fn a_godot_export_ships_only_in_the_app_its_demo_feature_says_it_is_for() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/", DEMO_TYPED);
    fs::write(
        root.join("export_presets.cfg"),
        "[preset.0]\n\nname=\"Windows Desktop\"\nplatform=\"Windows Desktop\"\n\
         custom_features=\"\"\nexport_path=\"export/windows/inn.exe\"\n\n[preset.0.options]\n\n\
         [preset.1]\n\nname=\"Windows Desktop Demo\"\nplatform=\"Windows Desktop\"\n\
         custom_features=\"demo\"\nexport_path=\"export/demo/windows/inn.exe\"\n\n\
         [preset.1.options]\n",
    )
    .unwrap();
    pushed(&root, "Both depots shipping every export");
    for (args, line) in [
        (
            ["check", "--demo"].as_slice(),
            "into depot 1000071, the demo's, and the export preset \"Windows Desktop\" in \
             export_presets.cfg makes it without the \"demo\" feature. Map only the folders the \
             demo's presets export to",
        ),
        (
            ["check"].as_slice(),
            "into depot 1000051, the full game's, and the export preset \"Windows Desktop Demo\" \
             in export_presets.cfg makes it with the \"demo\" feature. Map only the folders the full \
             game's presets export to",
        ),
    ] {
        let (crossed, shown, reason) = in_game(&root, args);
        assert_eq!(crossed, Some(2_i32), "{args:?}: {shown}{reason}");
        assert!(reason.contains(line), "{args:?}: {line:?} in {reason}");
    }

    with_demo(&root, "../../export/demo/", DEMO_TYPED);
    pushed(&root, "The demo's depot shipping the demo's exports alone");
    let (apart, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert!(
        shown.contains("  build     none named under [demo]"),
        "{shown}{reason}"
    );
    assert_eq!(apart, Some(2_i32), "stopped before steamcmd: {reason}");
}

#[test]
fn a_games_script_may_set_only_the_games_branch_live() {
    let (_temp, root) = pushed_game(None);
    let beta = r#""AppBuild" { "AppID" "1000050" "ContentRoot" "../export/" "SetLive" "beta" "Depots" { "1000051" "depot.vdf" } }"#;
    fs::write(root.join("steam").join("app_build.vdf"), beta).unwrap();
    pushed(&root, "Live on beta");
    for args in [["check"].as_slice(), ["upload"].as_slice()] {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        assert!(
            failure(&stderr).contains(
                " sets beta live, and a game's build goes live on testing alone, and on the \
                 default branch too while Steam shows the game unreleased"
            ),
            "{args:?}: {stderr}"
        );
        assert!(!stdout.contains("nothing refused"), "{args:?}: {stdout}");
    }

    fs::write(root.join("steamship.toml"), "branch = \"beta\"\n").unwrap();
    pushed(&root, "The game's branch is beta");
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "no build account here: {stdout}{stderr}");
    assert!(!stderr.contains("sets beta live"), "{stderr}");
    assert!(stdout.contains("  build     "), "past the script: {stdout}");
}

/// `steamship check` on `script`, in a home of its own and with no key set: a key kept for the
/// home of whoever runs the tests would otherwise have `check` ask Steam about the test's app.
fn run(script: &Path) -> (Option<i32>, String, String) {
    run_with(script, &[])
}

/// [`run`], with `extra` arguments after the script.
fn run_with(script: &Path, extra: &[&str]) -> (Option<i32>, String, String) {
    let home = tempfile::tempdir().unwrap();
    let output = run::output(
        run::command(env!("CARGO_BIN_EXE_steamship"))
            .arg("check")
            .arg(script)
            .args(extra)
            .env("STEAMSHIP_HOME", home.path())
            .env_remove("STEAMSHIP_WEB_API_KEY")
            .envs(common::store::environment()),
    )
    .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_command_says_what_it_mapped_and_exits_0() {
    let (_project, app) = Project::shipping("");
    let (code, stdout, stderr) = run(&app);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert_eq!(
        stdout,
        format!(
            "steamship check\n  \
             script    {}\n  \
             app       \u{2713} 1000, 2 depots, 4 files, checked\n  \
             branch    testing, not checked: no Web API key at hand\n    \
             a branch the app does not have leaves the build uploaded but not live. steamship \
             login --web-api-key-only keeps a key to check it with\n  \
             depot     1001, 2 files\n  \
             depot     1002, 2 files\n  \
             \u{2713} nothing refused\n",
            app.display()
        )
    );
}

#[test]
fn the_command_checks_the_scripts_alone_before_the_build_and_says_the_content_was_not() {
    let (project, app) = Project::shipping("");
    fs::remove_dir_all(project.root.join("export")).unwrap();
    let (code, stdout, stderr) = run_with(&app, &["--scripts-only"]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert_eq!(
        stdout,
        format!(
            "steamship check\n  \
             script    {}\n  \
             app       \u{2713} 1000, 2 depots, scripts checked\n  \
             branch    testing, not checked: no Web API key at hand\n    \
             a branch the app does not have leaves the build uploaded but not live. steamship \
             login --web-api-key-only keeps a key to check it with\n  \
             content   not checked (--scripts-only)\n  \
             \u{2713} nothing refused\n",
            app.display()
        )
    );
    let (full, _, _) = run(&app);
    assert_eq!(
        full,
        Some(2_i32),
        "without the flag, the missing build is refused"
    );
}

#[test]
fn the_command_says_a_script_that_is_not_there_does_not_exist_and_exits_2() {
    let project = Project::new();
    for missing in ["app_build.vdf", "steam/app_build.vdf"] {
        let script = project.root.join(missing);
        let (code, _, stderr) = run(&script);
        assert_eq!(code, Some(2_i32), "{missing}");
        assert_eq!(
            stderr,
            format!(
                "  \u{2717} refused: {}: does not exist. Name the app's build script, such as \
                 steam/app_build.vdf\n",
                script.display()
            ),
            "{missing}"
        );
    }
}

#[test]
fn the_command_lists_every_refusal_and_exits_2() {
    let (_project, app) = Project::shipping(r#""Preview" "1""#);
    let (code, stdout, stderr) = run(&app);
    assert_eq!(code, Some(2_i32));
    assert!(!stdout.contains('\u{2713}'), "{stdout}");
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with("  \u{2717} refused: ")),
        "{stderr}"
    );
    assert!(stderr.contains("\"Preview\" is set in the script. Pass --dry-run"));
}
