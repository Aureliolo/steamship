//! `steamship init`.

use std::fs;
use std::path::Path;

use crate::{failure, steamship, steamship_in};

#[test]
fn init_writes_scripts_check_takes_and_never_overwrites_them() {
    let folder = tempfile::tempdir().unwrap();
    let home = folder.path().join("home");
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &[
            "init",
            "480",
            "--depot",
            "481=build/windows",
            "--depot",
            "482=build/linux",
        ],
        Some(&home),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    for name in [
        "app_build.vdf",
        "depot_build_481.vdf",
        "depot_build_482.vdf",
    ] {
        let path = Path::new("steam").join(name);
        assert!(
            stdout.contains(&format!("  wrote     {}\n", path.display())),
            "{stdout}"
        );
        assert!(folder.path().join(&path).is_file(), "{name}");
    }
    // steam/app_build.vdf is where a game keeps its script, so this folder is now a game.
    assert!(
        stdout.contains("  branch    testing, the game's, set live\n"),
        "{stdout}"
    );
    let written = fs::read_to_string(folder.path().join("steam").join("app_build.vdf")).unwrap();
    assert!(written.contains("\t\"SetLive\" \"testing\"\n"), "{written}");
    let (demo, demo_said, _) = steamship_in(
        folder.path(),
        &["init", "490", "--output", "steam/demo"],
        Some(&home),
        &[],
    );
    assert_eq!(demo, Some(0_i32), "{demo_said}");
    assert!(
        demo_said.contains("  branch    testing, the game's, set live\n"),
        "{demo_said}"
    );
    for platform in ["windows", "linux"] {
        let build = folder.path().join("build").join(platform);
        fs::create_dir_all(&build).unwrap();
        fs::write(build.join("game.pck"), "pack").unwrap();
    }
    let script = folder.path().join("steam").join("app_build.vdf");
    let (checked, said, _) = steamship(&["check", script.to_str().unwrap()], Some(&home), &[]);
    assert_eq!(checked, Some(0_i32), "{said}");

    let before = fs::read_to_string(&script).unwrap();
    let (again, _, refused) = steamship_in(folder.path(), &["init", "480"], Some(&home), &[]);
    assert_eq!(again, Some(2_i32));
    assert_eq!(
        failure(&refused),
        format!(
            "{} already exists and is kept. Give another --output, or remove it",
            Path::new("steam").join("app_build.vdf").display()
        )
    );
    assert_eq!(fs::read_to_string(&script).unwrap(), before);
}

#[test]
fn an_app_that_is_not_a_number_says_what_an_app_id_is() {
    let folder = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(folder.path(), &["init", "abc"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        stderr.contains("'abc' is not an app ID. Give the number from Steamworks, such as 480"),
        "{stderr}"
    );
}

#[test]
fn an_app_of_0_is_refused_as_no_app_id() {
    let folder = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(folder.path(), &["init", "0"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("'0' is not an app ID"), "{stderr}");
}

#[test]
fn init_leaves_the_branch_to_choose_for_scripts_no_game_keeps() {
    let folder = tempfile::tempdir().unwrap();
    let home = folder.path().join("home");
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &["init", "480", "--output", "tools/steam"],
        Some(&home),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(!stdout.contains("  branch "), "{stdout}");
    let written = fs::read_to_string(
        folder
            .path()
            .join("tools")
            .join("steam")
            .join("app_build.vdf"),
    )
    .unwrap();
    assert!(written.contains("\t// \"SetLive\" \"beta\"\n"), "{written}");
}

#[test]
fn init_refuses_a_game_branch_holding_a_double_quote_before_writing_a_script() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("steamship.toml"), "branch = 'te\"st'\n").unwrap();
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &["init", "480"],
        Some(&folder.path().join("home")),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(
        failure(&stderr).ends_with(
            "steamship.toml: `branch` is te\"st. A build script cannot hold a double quote. \
             Name a branch without one"
        ),
        "{stderr}"
    );
    assert!(!folder.path().join("steam").exists());
}

#[test]
fn init_refuses_a_steamship_toml_it_cannot_read_before_writing_a_script() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("steamship.toml"), "branch = 1\n").unwrap();
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &["init", "480"],
        Some(&folder.path().join("home")),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(
        failure(&stderr).ends_with("steamship.toml: `branch` is not text. Write it in quotes"),
        "{stderr}"
    );
    assert!(!folder.path().join("steam").exists());
}

#[test]
fn init_writes_scripts_for_a_game_whose_steamship_toml_names_no_branch() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("steamship.toml"), "released = false\n").unwrap();
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &["init", "480"],
        Some(&folder.path().join("home")),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  branch    testing, the game's, set live\n"),
        "{stdout}"
    );
    assert!(folder.path().join("steam").join("app_build.vdf").is_file());
}
