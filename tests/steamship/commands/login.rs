//! `steamship login`.

use std::fs;
use std::iter;
use std::path::Path;
use std::time::Duration;

use steamship::install;
use steamship::keychain;
use steamship::webapi::STAND_IN;

#[cfg(unix)]
use crate::common;
#[cfg(unix)]
use crate::logout;
use crate::{
    APPS, BETAS, BUILDS, KEY, Session, failure, faked, faked_with, forgetting_home, login_with_key,
    steamship, steamship_typing, web_api,
};

#[test]
fn login_runs_steamcmd_for_the_account_in_the_home_and_remembers_it() {
    let home = faked();
    let (code, stdout, stderr) =
        steamship(&["login", "--account", "build_bot"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(stdout.starts_with("steamship login\n"), "{stdout}");
    assert!(stdout.contains("  account   as named\n"), "{stdout}");
    assert!(
        stdout.contains("args: +@ShutdownOnFailedCommand 1 +login build_bot +quit\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("home: {}\n", home.path().display())),
        "{stdout}"
    );
    // macOS's temporary folder is reached through a link, which steamcmd's folder is not.
    #[cfg(unix)]
    let root = fs::canonicalize(home.path().join(install::FOLDER)).unwrap();
    #[cfg(windows)]
    let root = home.path().join(install::FOLDER);
    assert!(
        stdout.contains(&format!("folder: {}\n", root.display())),
        "{stdout}"
    );
    assert!(
        stdout.ends_with("\u{2713} logged in\n    uploads use this login until it expires\n"),
        "{stdout}"
    );
    let (again, said, _) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(again, Some(0_i32));
    assert!(said.contains("  account   remembered\n"), "{said}");
    assert!(said.contains(" +login build_bot "), "{said}");
}

#[test]
fn login_with_a_saved_login_says_it_is_already_logged_in() {
    let home = faked();
    let (code, stdout, stderr) = steamship(
        &["login", "--account", "build_bot"],
        Some(home.path()),
        &[("STEAMSHIP_FAKE_SAVED", "1")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(!stdout.contains("cached credentials"), "{stdout}");
    assert!(
        stdout.ends_with(
            "\u{2713} already logged in\n    uploads use the saved login until it expires. \
             steamship logout forgets it\n"
        ),
        "{stdout}"
    );
}

#[test]
fn login_that_steamcmd_ends_badly_exits_1_and_remembers_nothing() {
    let home = faked();
    let (code, _, stderr) = steamship(
        &["login", "--account", "build_bot"],
        Some(home.path()),
        &[("STEAMSHIP_FAKE_EXIT", "5")],
    );
    assert_eq!(code, Some(1_i32));
    assert_eq!(failure(&stderr), "not logged in: steamcmd exited 5");
    assert!(
        stderr.ends_with("run steamship login to try again\n"),
        "{stderr}"
    );
    assert!(!home.path().join("account").exists());
}

/// A home whose steamcmd asks for a password, keeps what was typed in `typed` in the home, and
/// logs in only when that was `hunter2`. Otherwise it refuses it as steamcmd does.
///
/// With `STEAMSHIP_FAKE_GUARD` set to `code` it then asks for a Steam Guard code, kept in `code`
/// and taken only when it is `AB12C`. Set to `app`, it waits for the login to be approved in the
/// Steam Mobile app, and says something steamship has no meaning for meanwhile. With
/// `STEAMSHIP_FAKE_DENY` set the approval never comes.
fn asking() -> tempfile::TempDir {
    faked_with(
        "#!/bin/sh\n\
         failed() { echo \"Logging in user 'build_bot' [U:1:0] to Steam Public...$1\"; exit 5; }\n\
         echo 'Cached credentials not found.'\n\
         printf 'password: '\n\
         read -r typed\n\
         printf '%s' \"$typed\" > \"$HOME/typed\"\n\
         echo\n\
         [ \"$typed\" = hunter2 ] || failed 'ERROR (Invalid Password)'\n\
         case \"$STEAMSHIP_FAKE_GUARD\" in\n\
           code)\n\
             printf 'Steam Guard code: '\n\
             read -r code\n\
             printf '%s' \"$code\" > \"$HOME/code\"\n\
             echo\n\
             [ \"$code\" = AB12C ] || failed 'FAILED (Invalid Login Auth Code)' ;;\n\
           app)\n\
             echo 'Please confirm the login in the Steam Mobile app on your phone.'\n\
             echo 'A line steamship has no meaning for'\n\
             sleep 1\n\
             [ -z \"$STEAMSHIP_FAKE_DENY\" ] || failed 'FAILED (Timeout)'\n\
             echo 'Waiting for confirmation...OK' ;;\n\
         esac\n\
         echo \"Logging in user 'build_bot' [U:1:0] to Steam Public...OK\"\n",
    )
}

/// `steamship login` in `home`, with `typed` piped to it as a script would, and `variables` set.
fn login_typing(
    home: &Path,
    typed: &str,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    steamship_typing(&["login", "--account", "build_bot"], home, variables, typed)
}

#[test]
fn login_passes_the_password_to_steamcmd_and_never_shows_it() {
    let home = asking();
    let (code, stdout, stderr) = login_typing(home.path(), "hunter2\n", &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        fs::read_to_string(home.path().join("typed")).unwrap(),
        "hunter2"
    );
    assert!(stdout.contains("  password  \n"), "{stdout}");
    assert!(!stdout.contains("hunter2"), "the password was shown");
    assert!(!stdout.contains("Cached credentials"), "{stdout}");
    assert!(
        stdout.ends_with("\u{2713} logged in\n    uploads use this login until it expires\n"),
        "{stdout}"
    );
}

#[test]
fn login_with_a_wrong_password_says_steam_refused_it_and_exits_1() {
    let home = asking();
    let (code, stdout, stderr) = login_typing(home.path(), "hunter3\n", &[]);
    assert_eq!(code, Some(1_i32));
    assert_eq!(
        failure(&stderr),
        "not logged in: Steam refused the password"
    );
    for shown in [&stdout, &stderr] {
        assert!(!shown.contains("hunter3"), "the password was shown");
    }
    assert!(!home.path().join("account").exists());
}

#[test]
fn login_with_no_password_to_give_ends_steamcmd_and_exits_1() {
    let home = asking();
    let (code, _, stderr) = login_typing(home.path(), "", &[]);
    assert_eq!(code, Some(1_i32));
    assert_eq!(
        failure(&stderr),
        "not logged in: input ended before steamcmd had an answer"
    );
    assert!(!home.path().join("typed").exists());
}

#[test]
fn login_passes_a_steam_guard_code_to_steamcmd() {
    let home = asking();
    let guard = [("STEAMSHIP_FAKE_GUARD", "code")];
    let (code, stdout, stderr) = login_typing(home.path(), "hunter2\nAB12C\n", &guard);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        fs::read_to_string(home.path().join("code")).unwrap(),
        "AB12C"
    );
    assert!(stdout.contains("  password  \n  code      \n"), "{stdout}");
    let (wrong, _, said) = login_typing(home.path(), "hunter2\nAB12D\n", &guard);
    assert_eq!(wrong, Some(1_i32));
    assert_eq!(failure(&said), "not logged in: Steam refused the code");
}

#[test]
fn login_waits_for_approval_in_the_app_and_passes_on_what_it_cannot_place() {
    let home = asking();
    let (code, stdout, stderr) =
        login_typing(home.path(), "hunter2\n", &[("STEAMSHIP_FAKE_GUARD", "app")]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    let waiting = "  approve   waiting for you in the Steam Mobile app...\n";
    assert!(
        stdout.contains(&format!(
            "{waiting}    A line steamship has no meaning for\n{waiting}  approve   \u{2713} approved\n"
        )),
        "{stdout}"
    );
    assert!(stdout.ends_with("\u{2713} logged in\n    uploads use this login until it expires\n"));
}

#[test]
fn login_never_approved_says_so_and_exits_1() {
    let home = asking();
    let (code, stdout, stderr) = login_typing(
        home.path(),
        "hunter2\n",
        &[
            ("STEAMSHIP_FAKE_GUARD", "app"),
            ("STEAMSHIP_FAKE_DENY", "1"),
        ],
    );
    assert_eq!(code, Some(1_i32));
    assert!(
        stdout.ends_with("  approve   \u{2717} not approved\n"),
        "{stdout}"
    );
    assert_eq!(failure(&stderr), "not logged in: steamcmd says: Timeout");
    assert!(!home.path().join("account").exists());
}

/// A line that runs `steamship login` with no account named, then shows how the terminal was
/// left: on Unix its settings, and on Windows whether a line typed at the shell's own prompt
/// is echoed, as it is only in the mode steamship found it in.
fn login_then_look() -> String {
    let steamship = env!("CARGO_BIN_EXE_steamship");
    // A line reaches `cmd` with its quotes escaped as other programs read them, which `cmd` does
    // not, so nothing in it is quoted.
    #[cfg(windows)]
    assert!(!steamship.contains(' '), "{steamship}");
    // `call echo` with a caret reads the code after `login` ran, where a plain `%errorlevel%`
    // would be filled in as the line is parsed.
    #[cfg(windows)]
    let line = format!(
        "set STEAMSHIP_ACCOUNT=& {steamship} login & call echo exited %^errorlevel% & set /p after=next: "
    );
    #[cfg(unix)]
    let line = format!("unset STEAMSHIP_ACCOUNT; '{steamship}' login; echo \"exited $?\"; stty -a");
    line
}

#[test]
fn login_on_a_terminal_asks_for_the_account_shows_it_as_typed_and_refuses_a_bad_one() {
    typing_a_bad_account(&[("STEAMSHIP_NO_UPDATE_CHECK", "1"), ("NO_COLOR", "")]);
}

#[test]
fn backspace_at_a_prompt_takes_the_character_off_the_screen_with_colour_turned_off() {
    typing_a_bad_account(&[("STEAMSHIP_NO_UPDATE_CHECK", "1"), ("NO_COLOR", "1")]);
}

/// Types an account name that is not one at `steamship login`, with `variables` set, taking a
/// character back on the way.
fn typing_a_bad_account(variables: &[(&str, &str)]) {
    let home = tempfile::tempdir().unwrap();
    let mut session = Session::start_with(&login_then_look(), home.path(), variables);
    session.wait_for("account");
    assert!(
        session.seen().contains("uploads your build to Steam"),
        "the banner on a terminal"
    );
    // A character of each length UTF-8 has, which are read as whole characters.
    session.type_in("+qx\u{7f}uit\u{e9}\u{20ac}\u{1f600}");
    session.wait_for("+quit\u{e9}\u{20ac}\u{1f600}");
    session.type_in("\r");
    session.wait_for("\"+quit\u{e9}\u{20ac}\u{1f600}\" is not a Steam account name");
    looks_as_it_was(&mut session, "exited 2");
    assert!(
        !home.path().join("account").exists(),
        "a refused account is not kept"
    );
}

/// Checks that `session`'s terminal is as steamship found it, once steamship has `exited` so.
#[cfg(windows)]
fn looks_as_it_was(session: &mut Session, exited: &str) {
    session.wait_for(exited);
    session.wait_for("next:");
    session.type_in("typed after\r");
    session.wait_for("next: typed after");
    let _: Option<i32> = session.end().wait().unwrap();
}

#[cfg(unix)]
fn looks_as_it_was(session: &mut Session, exited: &str) {
    session.wait_for(exited);
    session.wait_for("icanon");
    let _: Option<i32> = session.end().wait().unwrap();
    let seen = session.seen();
    assert!(
        !seen.contains("-icanon") && !seen.contains("-isig"),
        "{seen}"
    );
}

#[test]
fn login_on_a_terminal_can_be_given_up_with_ctrl_c() {
    let home = tempfile::tempdir().unwrap();
    let mut session = Session::start(&login_then_look(), home.path());
    session.wait_for("account");
    session.type_in("build\u{3}");
    session.wait_for("the account name: given up");
    looks_as_it_was(&mut session, "exited 1");
}

#[test]
fn login_on_a_terminal_hides_the_password_and_waits_for_approval_with_a_spinner() {
    let home = asking();
    let line = format!(
        "unset STEAMSHIP_ACCOUNT; STEAMSHIP_FAKE_GUARD=app '{}' login; echo \"exited $?\"",
        env!("CARGO_BIN_EXE_steamship")
    );
    let mut session = Session::start(&line, home.path());
    session.wait_for("account");
    session.type_in("build_bot\r");
    session.wait_for("password");
    session.type_in("hunter2\r");
    // Drawn over and over on a terminal, with how long it has been, where elsewhere it would be
    // written once, ending in "...".
    session.wait_for(" waiting for you in the Steam Mobile app ");
    // Then the Web API key is offered, and Enter alone skips it.
    session.wait_for("Enter skips it");
    session.type_in("\r");
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(
        seen.contains("  api key   skipped (steamship login --web-api-key-only keeps one)\n"),
        "{seen}"
    );
    assert!(!seen.contains("Mobile app..."), "{seen}");
    assert!(seen.contains("  account   build_bot\n"), "{seen}");
    assert!(
        !seen.contains("  account   as named") && !seen.contains("  account   remembered"),
        "{seen}"
    );
    assert!(
        seen.contains(&format!("  password  {}\n", "\u{2022}".repeat(7))),
        "{seen}"
    );
    assert!(!seen.contains("hunter2"), "the password was shown");
    assert!(seen.contains("  approve   \u{2713} approved\n"), "{seen}");
    assert!(seen.contains("\u{2713} logged in"), "{seen}");
    assert_eq!(session.end().wait().unwrap(), Some(0_i32));
    assert_eq!(
        fs::read_to_string(home.path().join("account")).unwrap(),
        "build_bot\n"
    );
}

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
        assert_eq!((code, stdout.as_str()), (Some(2_i32), "steamship login\n"));
        assert!(
            failure(&stderr).starts_with("\"+quit\" is not a Steam account name"),
            "{stderr}"
        );
    }
}

#[test]
fn login_with_no_account_says_how_to_name_one_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32));
    assert_eq!(
        failure(&stderr),
        "no build account is named. Name it with --account or STEAMSHIP_ACCOUNT"
    );
}

#[test]
fn login_with_a_remembered_account_that_is_not_one_exits_1() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("account"), "+quit\n").unwrap();
    let (code, _, stderr) = steamship(&["login"], Some(home.path()), &[]);
    assert_eq!(code, Some(1_i32));
    assert!(
        failure(&stderr).starts_with("the account remembered in "),
        "{stderr}"
    );
}

#[test]
fn login_with_nowhere_to_look_for_an_account_exits_1() {
    let (code, _, stderr) = steamship(&["login"], None, &[]);
    assert_eq!(code, Some(1_i32));
    assert!(
        failure(&stderr).starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}

#[test]
fn login_with_web_api_key_only_refuses_what_is_not_a_key_before_asking_steam() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_typing(
        &["login", "--web-api-key-only"],
        home.path(),
        &[],
        "not_a_key_1234\n",
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("Manage Groups"),
        "says where the key is: {stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "that is not a publisher Web API key, which is 32 hexadecimal digits"
    );
    assert!(!format!("{stdout}{stderr}").contains("not_a_key"));
    let (_, status, _) = steamship(&["status"], Some(home.path()), &[]);
    assert!(status.contains("  api key   none kept\n"), "{status}");
}

#[test]
fn the_key_as_a_value_is_refused_as_an_argument() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["login", "--web-api-key-only", "0123"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(stderr.contains("unexpected argument '0123'"), "{stderr}");
}

#[test]
fn login_with_web_api_key_only_keeps_a_key_steam_takes_and_logout_forgets_it() {
    let home = forgetting_home();
    let (host, requests) = web_api(vec![("200 OK", APPS)]);
    let (code, stdout, stderr) = login_with_key(home.path(), &host, KEY, &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("Steam takes the key, for 2 apps\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  1000010   Falcon Ridge Overseer\n  1000030   Nocturne\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  api key   \u{2713} kept in {}\n",
            keychain::STORE
        )),
        "{stdout}"
    );
    let keeping: Vec<String> =
        iter::once(format!("  api key   \u{2713} kept in {}", keychain::STORE))
            .chain(keychain::KEPT_NOTE.iter().map(|line| format!("    {line}")))
            .collect();
    assert!(
        stdout.contains(&format!("{}\n", keeping.join("\n"))),
        "what to know about the store, right after: {stdout}"
    );
    assert!(stdout.contains("\u{2713} Web API key kept\n"), "{stdout}");
    assert!(!format!("{stdout}{stderr}").contains(KEY), "never shown");
    let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        request.starts_with("GET /ISteamApps/GetPartnerAppListForWebAPIKey/v2/?steamship="),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains(&format!("x-webapi-key: {KEY}")),
        "{request}"
    );
    let (_, kept, _) = steamship(&["status"], Some(home.path()), &[]);
    assert!(
        kept.contains(&format!("  api key   kept in {}\n", keychain::STORE)),
        "{kept}"
    );
    let set_key = "fedcba9876543210fedcba9876543210";
    let (_, set, _) = steamship(
        &["status"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", set_key)],
    );
    assert!(
        set.contains("  api key   from STEAMSHIP_WEB_API_KEY\n"),
        "the variable comes first: {set}"
    );
    assert!(!set.contains(set_key), "{set}");
    let (builds_host, builds_requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
    let (built, used, said) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", ""),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &builds_host),
        ],
    );
    assert_eq!((built, said.as_str()), (Some(0_i32), ""), "{used}");
    assert!(
        used.contains(&format!("  api key   kept in {}\n", keychain::STORE)),
        "{used}"
    );
    assert!(
        !used.contains("keep it"),
        "a kept key is not offered again: {used}"
    );
    let asked = builds_requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(
        asked
            .to_ascii_lowercase()
            .contains(&format!("x-webapi-key: {KEY}")),
        "the kept key: {asked}"
    );
    let (_, forgot, _) = steamship(&["logout"], Some(home.path()), &[]);
    assert!(
        forgot.contains("  api key   \u{2713} forgotten\n"),
        "{forgot}"
    );
    let (_, after, _) = steamship(&["status"], Some(home.path()), &[]);
    assert!(after.contains("  api key   none kept\n"), "{after}");
}

#[test]
fn login_with_web_api_key_only_keeps_nothing_steam_refuses() {
    let home = forgetting_home();
    let (host, _requests) = web_api(vec![("403 Forbidden", "")]);
    let (code, stdout, stderr) = login_with_key(home.path(), &host, KEY, &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("Steam refused the Web API key. "),
        "{stderr}"
    );
    let (_, status, _) = steamship(&["status"], Some(home.path()), &[]);
    assert!(status.contains("  api key   none kept\n"), "{status}");
}

#[cfg(unix)]
#[test]
fn login_on_a_terminal_keeps_the_key_typed_at_its_offer_and_offers_no_more() {
    let home = common::store::Home::of(asking(), logout);
    let (host, _requests) = web_api(vec![("200 OK", APPS)]);
    let login = format!(
        "'{}' login --account build_bot",
        env!("CARGO_BIN_EXE_steamship")
    );
    let mut session = Session::start_with(
        &format!("unset STEAMSHIP_ACCOUNT; {login}; {login}; echo \"exited $?\""),
        home.path(),
        &[
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_WEB_API_KEY", ""),
            (STAND_IN, &host),
        ],
    );
    session.wait_for_another("  password  ");
    session.type_in("hunter2\r");
    session.wait_for("Enter skips it");
    session.type_in(&format!("{KEY}\r"));
    session.wait_for(&format!("  api key   \u{2713} kept in {}", keychain::STORE));
    session.wait_for_another("  password  ");
    session.type_in("hunter2\r");
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(seen.contains("Steam takes the key, for 2 apps\n"), "{seen}");
    assert!(
        seen.contains(&format!("  api key   kept in {}\n", keychain::STORE)),
        "the second login says where it is kept: {seen}"
    );
    assert_eq!(
        seen.matches("Enter skips it").count(),
        1,
        "offered once: {seen}"
    );
    assert!(!seen.contains(KEY), "the key was shown: {seen}");
    assert_eq!(session.end().wait().unwrap(), Some(0_i32));
}
