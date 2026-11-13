//! `steamship drm-wrap`.

use std::fs;

use steamship::platform::Platform;
use steamship::steamcmd;

use crate::{faked_with, packed_login, saved_login, steamship};

/// A steamcmd that wraps as steamcmd 1788292693 did an executable importing `GetModuleHandleA`:
/// it writes the wrapped one where it was told and says so, keeping what it was started with in
/// `args`.
const WRAPS: &str = "#!/bin/sh\n\
    echo \"$*\" > \"$HOME/args\"\n\
    while [ $# -gt 0 ] && [ \"$1\" != +drm_wrap ]; do shift; done\n\
    [ $# -ge 4 ] || exit 1\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    printf \"$STEAMSHIP_FAKE_WRAPPED\" > \"$4\"\n\
    echo \"DRM wrap completed; output is in $4\"\n";

/// A steamcmd whose DRM wrap Valve's tool refuses, as it refused Falcon Ridge Overseer's Godot
/// executable.
const REFUSES_WRAP: &str = "#!/bin/sh\n\
    echo \"$*\" > \"$HOME/args\"\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'Error result: 8 (Invalid Parameter - Valve Portable DRM Tool (Build: Sep 22 2026)'\n\
    echo '(C) Copyright 2012-2025, Valve Corporation, All rights reserved.'\n\
    echo 'PE module implementation does not currently support adding imports'\n\
    echo 'Unable to add import of kernel32.dll:GetModuleHandleA'\n\
    echo ')'\n\
    echo 'DRM wrap failed with EResult 8 (Invalid Parameter)'\n\
    exit 11\n";

#[test]
fn an_executable_that_is_not_there_is_named_with_what_to_give() {
    let files = tempfile::tempdir().unwrap();
    let missing = files.path().join("nothere.exe");
    let (code, _, said) = steamship(
        &["drm-wrap", "1000", &missing.to_string_lossy()],
        None,
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{said}");
    assert!(
        said.contains(&format!(
            "{}: does not exist. Name the Windows executable to wrap, such as \
             build/windows/game.exe",
            missing.display()
        )),
        "{said}"
    );
}

#[test]
fn a_folder_given_as_the_executable_is_refused_with_why_and_not_as_missing() {
    let files = tempfile::tempdir().unwrap();
    let (code, _, said) = steamship(
        &["drm-wrap", "1000", &files.path().to_string_lossy()],
        None,
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{said}");
    assert!(
        said.contains(&format!("{}: ", files.path().display())),
        "{said}"
    );
    assert!(!said.contains("does not exist"), "{said}");
}

#[cfg(unix)]
#[test]
fn an_executable_is_wrapped_in_place_or_into_the_output_keeping_its_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let home = faked_with(WRAPS);
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("my game.exe");
    fs::write(&game, "MZplain").unwrap();
    fs::set_permissions(&game, fs::Permissions::from_mode(0o750)).unwrap();
    let quiet = [
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ("STEAMSHIP_FAKE_WRAPPED", "MZwrapped"),
    ];

    let (code, stdout, stderr) = steamship(
        &["drm-wrap", "1000", &game.to_string_lossy()],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("my game.exe is wrapped in Steam DRM"),
        "{stdout}"
    );
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZwrapped");
    assert_eq!(
        fs::metadata(&game).unwrap().permissions().mode() & 0o777,
        0o750,
        "the original's permissions"
    );
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    let beside = files.path().join(".my game.exe.steamship-drm");
    assert!(
        args.contains(&format!(
            "+login build_bot +drm_wrap 1000 {} {} drmtoolp 0 +quit",
            game.display(),
            beside.display()
        )),
        "{args}"
    );
    assert_eq!(
        fs::read_dir(files.path()).unwrap().count(),
        1,
        "nothing left beside it"
    );

    fs::write(&game, "MZplain").unwrap();
    let copy = files.path().join("wrapped.exe");
    let (copied, said, _) = steamship(
        &[
            "drm-wrap",
            "1000",
            &game.to_string_lossy(),
            "--output",
            &copy.to_string_lossy(),
            "--compatibility",
        ],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(copied, Some(0_i32), "{said}");
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZplain", "left alone");
    assert_eq!(fs::read_to_string(&copy).unwrap(), "MZwrapped");
    let compatible = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(compatible.ends_with("drmtoolp 6 +quit\n"), "{compatible}");
}

#[test]
fn a_wrap_in_ci_is_done_as_the_account_handed_over() {
    // No login saved and no account remembered: only what `steamship ci` packed.
    let home = faked_with(WRAPS);
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("game.exe");
    fs::write(&game, "MZplain").unwrap();
    let (code, stdout, stderr) = steamship(
        &["drm-wrap", "1000", &game.to_string_lossy()],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_FAKE_WRAPPED", "MZwrapped"),
        ],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains("+login build_bot "), "{args}");
    assert!(
        steamcmd::saved_login(home.path(), Platform::THIS).exists(),
        "the login handed over was put where steamcmd looks"
    );
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZwrapped");
}

#[test]
fn a_wrap_in_ci_with_an_account_named_is_done_as_the_account_named() {
    let home = faked_with(WRAPS);
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("game.exe");
    fs::write(&game, "MZplain").unwrap();
    let (code, stdout, stderr) = steamship(
        &[
            "drm-wrap",
            "1000",
            &game.to_string_lossy(),
            "--account",
            "other_bot",
        ],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_FAKE_WRAPPED", "MZwrapped"),
        ],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains("+login other_bot "), "{args}");
}

#[test]
fn a_wrap_said_done_that_wrote_nothing_is_a_failure_and_replaces_nothing() {
    let home = faked_with(WRAPS);
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("game.exe");
    fs::write(&game, "MZplain").unwrap();
    let (code, stdout, stderr) = steamship(
        &["drm-wrap", "1000", &game.to_string_lossy()],
        Some(home.path()),
        &[
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_FAKE_WRAPPED", ""),
        ],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr
            .contains("steamcmd said the wrap completed, but the wrapped file is missing or empty"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZplain", "left alone");
    assert_eq!(
        fs::read_dir(files.path()).unwrap().count(),
        1,
        "the empty file is not left beside it"
    );
}

#[test]
fn an_accounts_name_in_a_wraps_refusal_changes_neither_its_exit_code_nor_steamships_words() {
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("game.exe");
    for (name, script, code, words) in [
        (
            "limit",
            "#!/bin/sh\n\
             echo \"Logging in user 'limit' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"\n\
             exit 5\n",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "steam",
            "#!/bin/sh\n\
             echo \"Logging in user 'steam' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)\"\n\
             exit 5\n",
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "valve",
            REFUSES_WRAP,
            1_i32,
            "Valve's DRM tool refused the executable: PE module implementation does not \
             currently support adding imports",
        ),
    ] {
        let home = faked_with(script);
        saved_login(home.path());
        fs::write(&game, "MZgodot").unwrap();
        let (found, stdout, stderr) = steamship(
            &[
                "drm-wrap",
                "1000",
                &game.to_string_lossy(),
                "--account",
                name,
            ],
            Some(home.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(found, Some(code), "{name}: {stdout}{stderr}");
        assert!(stderr.contains(words), "{name}: {stderr}");
    }
}

#[test]
fn a_refused_wrap_says_why_and_a_file_that_is_no_windows_program_is_refused_first() {
    let home = faked_with(REFUSES_WRAP);
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let game = files.path().join("game.exe");
    fs::write(&game, "MZgodot").unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];

    let (code, _, why) = steamship(
        &["drm-wrap", "1000", &game.to_string_lossy()],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(code, Some(1_i32), "{why}");
    assert!(
        why.contains(
            "Valve's DRM tool refused the executable: PE module implementation does not \
             currently support adding imports. Unable to add import of \
             kernel32.dll:GetModuleHandleA"
        ) && why.contains("Godot's and Rust's executables do not"),
        "{why}"
    );
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZgodot", "left alone");
    assert_eq!(fs::read_dir(files.path()).unwrap().count(), 1);

    let linux = files.path().join("game.x86_64");
    fs::write(&linux, "\x7fELF").unwrap();
    fs::remove_file(home.path().join("args")).unwrap();
    let (refused, _, stderr) = steamship(
        &["drm-wrap", "1000", &linux.to_string_lossy()],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(refused, Some(2_i32), "{stderr}");
    assert!(stderr.contains("is not a Windows executable"), "{stderr}");
    assert!(!home.path().join("args").exists(), "steamcmd was not run");
}
