//! `steamship status`.

#[cfg(windows)]
use std::env;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixListener;
#[cfg(windows)]
use std::os::windows::fs::symlink_file;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::thread;

use steamship::install;
use steamship::platform::Platform;
#[cfg(windows)]
use steamship::run;
use steamship::steamcmd;

#[cfg(windows)]
use crate::outcome;
#[cfg(target_os = "linux")]
use crate::{APPS, KEY, login_with_key, web_api};
use crate::{
    Session, answering, checking, failure, faked, faked_with, packed_login, saved_login, steamship,
};

#[test]
fn status_with_nothing_saved_says_to_log_in_changes_nothing_and_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    #[cfg(windows)]
    let console = if env::var_os("STEAMSHIP_TEST_CONPTY").is_some() {
        "  console   Microsoft's ConPTY\n"
    } else {
        "  console   Windows' own (conpty.dll is not beside steamship.exe)\n"
    };
    #[cfg(not(windows))]
    let console = "";
    assert_eq!(
        stdout,
        format!(
            "steamship status\n  \
             version   {}\n  \
             home      {}\n\
             {console}  \
             api key   none kept\n  \
             account   none remembered\n  \
             steamcmd  not installed (steamship login installs it)\n  \
             login     none saved\n",
            env!("CARGO_PKG_VERSION"),
            home.path().display()
        )
    );
    assert_eq!(failure(&stderr), "not logged in");
    assert!(stderr.ends_with("    run steamship login\n"), "{stderr}");
    assert_eq!(
        fs::read_dir(home.path()).unwrap().count(),
        0,
        "nothing written"
    );
}

/// What `status` prints in a new home when run as `program`, with no `STEAMSHIP_TEST_CONPTY`.
/// It exits 3, since nothing is saved there.
#[cfg(windows)]
fn status_of(program: &Path) -> String {
    let home = tempfile::tempdir().unwrap();
    let mut command = run::command(program);
    let _: &mut Command = command
        .arg("status")
        .env("STEAMSHIP_HOME", home.path())
        .env_remove("STEAMSHIP_TEST_CONPTY")
        .env_remove("STEAMSHIP_ACCOUNT")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("GITHUB_OUTPUT");
    let (code, stdout, stderr) = outcome(&run::output(&mut command).unwrap());
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    stdout
}

/// A new folder holding steamship.exe, and the package's files when `with_console` is set.
#[cfg(windows)]
fn installed(with_console: bool) -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let _: u64 = fs::copy(
        env!("CARGO_BIN_EXE_steamship"),
        folder.path().join("steamship.exe"),
    )
    .unwrap();
    if with_console {
        let package = env::var_os("STEAMSHIP_TEST_CONPTY")
            .ok_or(
                "STEAMSHIP_TEST_CONPTY names no folder: run `bash .github/conpty.sh x64 <folder>` and set it to that folder",
            )
            .unwrap();
        for file in ["conpty.dll", "OpenConsole.exe"] {
            let _: u64 =
                fs::copy(Path::new(&package).join(file), folder.path().join(file)).unwrap();
        }
    }
    folder
}

#[cfg(windows)]
#[test]
fn status_names_the_console_steamcmd_runs_on() {
    let with_console = installed(true);
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("steamship.exe");
    symlink_file(with_console.path().join("steamship.exe"), &link).unwrap();
    assert!(
        status_of(&link).contains("  console   Microsoft's ConPTY\n"),
        "through a link"
    );

    let alone = installed(false);
    assert!(
        status_of(&alone.path().join("steamship.exe"))
            .contains("  console   Windows' own (conpty.dll is not beside steamship.exe)\n"),
        "steamship.exe alone"
    );
}

#[test]
fn status_in_ci_logs_in_with_the_login_handed_over_and_puts_it_where_steamcmd_looks() {
    let home = answering(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let packed = packed_login();
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", &packed), ("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  account   from STEAMSHIP_LOGIN\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  login     from STEAMSHIP_LOGIN, packed 2026-09-28\n"),
        "{stdout}"
    );
    assert!(stdout.ends_with("  \u{2713} ready to upload\n"), "{stdout}");
    assert!(
        !stdout.contains(&packed) && !stdout.contains("build_bot"),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(home.path().join("args")).unwrap(),
        "+@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +login build_bot +quit\n"
    );
    assert_eq!(
        fs::read_to_string(steamcmd::saved_login(home.path(), Platform::THIS)).unwrap(),
        "\"token\" \"a_saved_login_token_0123456789\""
    );
}

/// A steamcmd stopped from outside as it starts, as one was on a macOS runner: exit 137, with a
/// line added to its own connection log and nothing said of why.
const KILLED: &str = "#!/bin/sh\n\
    logs=\"$HOME/Steam/logs\"\n\
    [ \"$(uname)\" = Darwin ] && logs=\"$HOME/Library/Application Support/Steam/logs\"\n\
    case \"$(uname)\" in MINGW* | MSYS*) logs=\"$HOME/steamcmd/logs\" ;; esac\n\
    mkdir -p \"$logs\"\n\
    echo 'this run: connecting' >> \"$logs/connection_log.txt\"\n\
    echo 'Loading Steam API...OK'\n\
    exit 137\n";

#[test]
fn a_login_check_that_fails_without_a_reason_shows_steams_logs_in_actions() {
    let home = faked_with(KILLED);
    let logs = steamcmd::state_folder(home.path(), Platform::THIS).join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("connection_log.txt"), "an older run\n").unwrap();
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_ACCOUNT", ""),
            ("GITHUB_ACTIONS", "true"),
        ],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("steamcmd exited 137 without saying why"),
        "{stderr}"
    );
    for group in [
        "::group::steamcmd's console\n",
        "::group::steamcmd's connection_log.txt, from this run\n",
    ] {
        assert!(stdout.contains(group), "{group:?} in {stdout}");
    }
    assert!(stdout.contains("Loading Steam API...OK"), "{stdout}");
    assert!(stdout.contains("this run: connecting"), "{stdout}");
    assert!(!stdout.contains("an older run"), "{stdout}");

    // Outside Actions nothing is dumped, as for an upload.
    let (_, quiet, _) = steamship(
        &["status"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_ACCOUNT", ""),
        ],
    );
    assert!(!quiet.contains("::group::"), "{quiet}");
}

#[test]
fn a_login_check_that_fails_names_no_account_in_actions_group() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...",
        137,
    );
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[("GITHUB_ACTIONS", "true")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("::group::steamcmd's console\n")
            && stdout.contains("Logging in user [redacted] [U:1:0]"),
        "{stdout}"
    );
    assert!(!format!("{stdout}{stderr}").contains("build_bot"));
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
        let home = answering(said, 5);
        saved_login(home.path());
        let (found, stdout, stderr) =
            steamship(&["status", "--account", name], Some(home.path()), &[]);
        assert_eq!(found, Some(code), "{name}: {stdout}{stderr}");
        assert_eq!(
            failure(&stderr),
            format!("not logged in: {words}"),
            "{name}"
        );
    }
}

#[test]
fn a_login_handed_over_that_is_not_one_is_refused_without_repeating_it() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[(
            "STEAMSHIP_LOGIN",
            "steamship-login-2:2026-09-28:secret_name:zz",
        )],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_LOGIN does not hold a login packed by steamship ci. Pack the login again \
         with steamship ci"
    );
    assert!(!format!("{stdout}{stderr}").contains("secret_name"));
}

#[test]
fn a_login_from_a_newer_steamship_is_refused_with_how_to_upgrade() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", "steamship-login-4:x")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(
        failure(&stderr).contains("packed by a newer steamship, in format 4"),
        "{stderr}"
    );
}

#[test]
fn an_empty_account_variable_names_no_account() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, _) = steamship(&["status"], Some(home.path()), &[("STEAMSHIP_ACCOUNT", "")]);
    assert_eq!(code, Some(3_i32), "{stdout}");
    assert!(stdout.contains("  account   none remembered\n"), "{stdout}");
}

#[test]
fn status_checks_the_saved_login_with_steam_and_says_it_is_ready() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(stdout.contains("  account   remembered\n"), "{stdout}");
    assert!(stdout.contains("  login     saved\n"), "{stdout}");
    assert!(
        stdout.contains("  steam     \u{2713} Steam takes the saved login\n"),
        "{stdout}"
    );
    assert!(stdout.ends_with("  \u{2713} ready to upload\n"), "{stdout}");
    assert!(
        !stdout.contains("build_bot"),
        "the account is never named: {stdout}"
    );
    assert_eq!(
        fs::read_to_string(home.path().join("args")).unwrap(),
        "+@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +login build_bot +quit\n"
    );
}

#[test]
fn status_with_a_login_steam_refuses_says_why_without_the_account_and_exits_3() {
    let home = checking(
        "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)",
        5,
    );
    let (code, stdout, stderr) = steamship(
        &["status", "--account", "build_bot"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  account   as named\n"), "{stdout}");
    assert!(
        stdout.contains("  steam     \u{2717} refused after "),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam refused the saved login (Expired Login Auth Code)"
    );
    assert!(
        stderr.ends_with("log the build account in again with steamship login\n"),
        "{stderr}"
    );
    assert!(!format!("{stdout}{stderr}").contains("build_bot"));
}

#[test]
fn status_with_a_login_steam_limits_says_to_wait_and_gives_no_hint_to_log_in() {
    let home = checking(
        "Logging in user 'build_bot' to Steam Public...FAILED (Rate Limit Exceeded)",
        5,
    );
    let (code, stdout, stderr) = steamship(
        &["status", "--account", "build_bot"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam is limiting login attempts. Wait, then try again"
    );
    assert!(
        stderr
            .trim_end()
            .ends_with("Steam is limiting login attempts. Wait, then try again"),
        "{stderr}"
    );
}

#[test]
fn a_login_handed_over_that_steam_limits_says_to_wait_and_not_when_it_was_packed() {
    let home = answering(
        "Logging in user 'build_bot' to Steam Public...FAILED (Account Login Denied Throttle)",
        5,
    );
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_ACCOUNT", ""),
        ],
    );
    assert_eq!(code, Some(6_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam is limiting login attempts. Wait, then try again"
    );
    assert!(!stderr.contains("packed on"), "{stderr}");
    assert!(!stderr.contains("steamship login"), "{stderr}");
}

#[test]
fn a_login_handed_over_that_steam_refuses_says_when_it_was_packed_and_how_to_pack_another() {
    let home = answering(
        "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)",
        5,
    );
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_ACCOUNT", ""),
        ],
    );
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam refused the saved login (Expired Login Auth Code). The login in \
         STEAMSHIP_LOGIN was packed on 2026-09-28"
    );
    assert!(
        stderr.ends_with(
            "log in again with steamship login, then run steamship ci to pack the new login\n"
        ),
        "{stderr}"
    );
}

#[test]
fn status_that_cannot_reach_steam_names_the_reason_and_exits_1() {
    let home = checking("ERROR! Timed out waiting for Steam", 6);
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  steam     \u{2717} could not check\n"),
        "{stdout}"
    );
    assert_eq!(failure(&stderr), "Timed out waiting for Steam");
}

#[test]
fn status_with_a_saved_login_and_no_account_says_to_log_in_and_exits_3() {
    let home = faked();
    saved_login(home.path());
    fs::remove_file(home.path().join("account")).unwrap();
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  account   none remembered\n"), "{stdout}");
    assert!(
        stdout.contains("  steamcmd  \u{2713} ") && stdout.ends_with("  login     saved\n"),
        "{stdout}"
    );
    assert_eq!(failure(&stderr), "no build account");
}

#[test]
fn status_with_steamcmd_altered_says_so_and_exits_4() {
    let home = faked();
    let program = home.path().join(install::FOLDER).join("steamcmd.sh");
    fs::write(&program, "#!/bin/sh\necho changed\n").unwrap();
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    assert!(
        stdout.ends_with("  steamcmd  \u{2717} differs from the pinned version\n"),
        "{stdout}"
    );
    assert!(
        failure(&stderr).contains("steamcmd.sh has changed"),
        "{stderr}"
    );
}

#[test]
fn status_on_a_terminal_starts_with_the_banner() {
    let home = tempfile::tempdir().unwrap();
    let steamship = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship} status");
    #[cfg(unix)]
    let line = format!("'{steamship}' status");
    let mut session = Session::start(&line, home.path());
    session.wait_for("run steamship login");
    let _: Option<i32> = session.end().wait().unwrap();
    let seen = session.seen();
    let banner = seen.find("uploads your build to Steam");
    let title = seen.find("steamship status");
    assert!(
        banner.is_some() && banner < title,
        "the banner, then the title: {seen}"
    );
}

/// A session bus that hangs up on everyone who connects, at a socket in the folder given back.
#[cfg(target_os = "linux")]
fn broken_bus() -> (tempfile::TempDir, String) {
    let folder = tempfile::tempdir().unwrap();
    let socket = folder.path().join("bus");
    let listener = UnixListener::bind(&socket).unwrap();
    let _hanging_up = thread::spawn(move || {
        for connection in listener.incoming() {
            drop(connection);
        }
    });
    (folder, format!("unix:path={}", socket.display()))
}

#[cfg(target_os = "linux")]
#[test]
fn with_no_credential_store_no_key_is_kept_and_status_and_logout_say_so() {
    let home = tempfile::tempdir().unwrap();
    let nowhere = [(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/nonexistent/steamship/bus",
    )];
    let (host, _requests) = web_api(vec![("200 OK", APPS)]);
    let (code, stdout, stderr) = login_with_key(home.path(), &host, KEY, &nowhere);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("Steam takes the key, for 2 apps\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  api key   \u{2717} not kept\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "no credential store: no D-Bus session bus"
    );
    let (_, status, _) = steamship(&["status"], Some(home.path()), &nowhere);
    assert!(
        status.contains("  api key   no credential store here\n"),
        "{status}"
    );
    let (logged_out, logout, said) = steamship(&["logout"], Some(home.path()), &nowhere);
    assert_eq!((logged_out, said.as_str()), (Some(0_i32), ""), "{logout}");
    assert!(logout.contains("  api key   none kept\n"), "{logout}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_credential_store_that_fails_is_named_and_nothing_goes_on_without_it() {
    let home = tempfile::tempdir().unwrap();
    let (_folder, address) = broken_bus();
    let broken = [
        ("DBUS_SESSION_BUS_ADDRESS", address.as_str()),
        ("STEAMSHIP_WEB_API_KEY", ""),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let (_, status, _) = steamship(&["status"], Some(home.path()), &broken);
    assert!(
        status.contains("  api key   \u{2717} the Secret Service: the session bus: "),
        "{status}"
    );
    let (logged_out, logout, said) = steamship(&["logout"], Some(home.path()), &broken);
    assert_eq!(logged_out, Some(1_i32), "{logout}{said}");
    assert!(
        logout.contains("  api key   \u{2717} not forgotten\n"),
        "{logout}"
    );
    assert!(!logout.contains("logged out"), "{logout}");
    let (code, builds, stderr) = steamship(&["builds", "1000010"], Some(home.path()), &broken);
    assert_eq!(code, Some(1_i32), "{builds}{stderr}");
    assert!(
        builds.contains("  api key   \u{2717} not read\n"),
        "{builds}"
    );
    assert!(
        failure(&stderr).starts_with("the Secret Service: "),
        "{stderr}"
    );
}
