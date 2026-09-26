//! The `steamship` command itself: what it prints and the code it exits with.
//!
//! These run after the library's own tests on purpose: a change that broke the installer would
//! send `install` here to Valve's CDN, and the unit tests that fail at once on such a change are
//! the ones that should say so.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::fs::{self, File};
use std::path::Path;
use std::process::Command;

use steamship::install;
use steamship::manifest::Manifest;
use steamship::platform::Platform;

/// `steamship install` with `STEAMSHIP_HOME` set to `home`, or with no environment at all.
fn run(home: Option<&Path>) -> (Option<i32>, String, String) {
    steamship(&["install"], home, &[])
}

/// `steamship` with `args`, `STEAMSHIP_HOME` set to `home` (or with no environment at all), and
/// `variables` set besides.
fn steamship(
    args: &[&str],
    home: Option<&Path>,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_steamship"));
    let _: &mut Command = command.args(args).env_remove("STEAMSHIP_ACCOUNT");
    let _: &mut Command = match home {
        Some(home) => command.env("STEAMSHIP_HOME", home),
        None => command.env_clear(),
    };
    let output = command.envs(variables.iter().copied()).output().unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A home that records the pinned steamcmd as installed, with `files` in its inventory.
fn recorded(files: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let pinned = Manifest::pinned(Platform::THIS).unwrap();
    fs::create_dir_all(home.path().join(install::FOLDER)).unwrap();
    let inventory = format!(
        "steamship inventory 1\nsystem {}\nversion {}\n{files}",
        pinned.system, pinned.version
    );
    fs::write(home.path().join("steamcmd.inventory"), inventory).unwrap();
    home
}

#[test]
fn install_says_when_the_install_is_as_pinned() {
    let home = recorded("");
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(stdout.ends_with(" is as pinned\n"), "{stdout}");
}

#[test]
fn install_names_what_changed_and_exits_4() {
    let home = recorded(&format!("file {} steamcmd.exe\n", "0".repeat(64)));
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!((code, stdout.as_str()), (Some(4_i32), ""));
    assert!(
        stderr.trim_end().ends_with("steamcmd.exe is missing"),
        "{stderr}"
    );
}

#[test]
fn install_turns_a_second_run_away_and_exits_1() {
    let home = recorded("");
    let held = File::create(home.path().join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let (code, _, stderr) = run(Some(home.path()));
    assert_eq!(code, Some(1_i32));
    assert!(stderr.contains("another steamship is using"), "{stderr}");
    held.unlock().unwrap();
}

#[test]
fn install_says_where_it_could_not_find_a_home_and_exits_1() {
    let (code, stdout, stderr) = run(None);
    assert_eq!((code, stdout.as_str()), (Some(1_i32), ""));
    assert!(
        stderr.starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}

/// A home whose recorded steamcmd is a script that prints what it was started with and exits
/// with `STEAMSHIP_FAKE_EXIT`, or 0. With `STEAMSHIP_FAKE_CHANGE` set it first changes itself, as
/// an update would.
#[cfg(unix)]
fn faked() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    use steamship::digest;

    let script = "#!/bin/sh\n\
                  echo \"args: $*\"\n\
                  echo \"home: $HOME\"\n\
                  echo \"folder: $PWD\"\n\
                  [ -n \"$STEAMSHIP_FAKE_CHANGE\" ] && echo >> \"$0\"\n\
                  exit \"${STEAMSHIP_FAKE_EXIT:-0}\"\n";
    let hash = digest::hex(&digest::sha256(&mut script.as_bytes()).unwrap());
    let home = recorded(&format!("file {hash} steamcmd.sh\n"));
    let program = home.path().join(install::FOLDER).join("steamcmd.sh");
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    home
}

#[cfg(unix)]
#[test]
fn login_runs_steamcmd_for_the_account_in_the_home_and_remembers_it() {
    let home = faked();
    let (code, stdout, stderr) =
        steamship(&["login", "--account", "build_bot"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(
        stdout.contains("args: +@ShutdownOnFailedCommand 1 +login build_bot +quit\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("home: {}\n", home.path().display())),
        "{stdout}"
    );
    let root = fs::canonicalize(home.path().join(install::FOLDER)).unwrap();
    assert!(
        stdout.contains(&format!("folder: {}\n", root.display())),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(
            "build_bot is logged in; uploads use the login steamcmd keeps, until it expires\n"
        ),
        "{stdout}"
    );
    let (code, stdout, _) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(code, Some(0_i32));
    assert!(stdout.contains(" +login build_bot "), "{stdout}");
}

#[cfg(unix)]
#[test]
fn login_that_steamcmd_refuses_exits_1_and_remembers_nothing() {
    let home = faked();
    let (code, _, stderr) = steamship(
        &["login", "--account", "build_bot"],
        Some(home.path()),
        &[("STEAMSHIP_FAKE_EXIT", "5")],
    );
    assert_eq!(
        (code, stderr.as_str()),
        (
            Some(1_i32),
            "steamcmd did not log build_bot in (it exited 5)\n"
        )
    );
    assert!(!home.path().join("account").exists());
}

#[cfg(unix)]
#[test]
fn login_after_which_steamcmd_has_changed_exits_4() {
    let home = faked();
    let (code, _, stderr) = steamship(
        &["login", "--account", "build_bot"],
        Some(home.path()),
        &[("STEAMSHIP_FAKE_CHANGE", "1")],
    );
    assert_eq!(code, Some(4_i32));
    assert!(
        stderr.trim_end().ends_with("steamcmd.sh has changed"),
        "{stderr}"
    );
}

#[test]
fn login_refuses_a_name_steamcmd_could_read_as_a_command_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    for (code, stdout, stderr) in [
        steamship(&["login", "--account=+quit"], Some(home.path()), &[]),
        steamship(
            &["login"],
            Some(home.path()),
            &[("STEAMSHIP_ACCOUNT", "+quit")],
        ),
    ] {
        assert_eq!((code, stdout.as_str()), (Some(2_i32), ""));
        assert!(
            stderr.starts_with("\"+quit\" is not a Steam account name"),
            "{stderr}"
        );
    }
}

#[test]
fn login_with_no_account_says_how_to_name_one_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(
        (code, stdout.as_str(), stderr.as_str()),
        (
            Some(2_i32),
            "",
            "name the build account with --account or STEAMSHIP_ACCOUNT\n"
        )
    );
}

#[test]
fn login_with_a_remembered_account_that_is_not_one_exits_1() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("account"), "+quit\n").unwrap();
    let (code, _, stderr) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32));
    assert!(stderr.starts_with("the account remembered in "), "{stderr}");
}

#[test]
fn login_with_nowhere_to_look_for_an_account_exits_1() {
    let (code, _, stderr) = steamship(&["login"], None, &[]);
    assert_eq!(code, Some(1_i32));
    assert!(
        stderr.starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}
