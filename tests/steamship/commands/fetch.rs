//! `steamship fetch`.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use steamship::platform::Platform;
use steamship::{fetch, install, steamcmd};

use crate::common;
use crate::{
    APPS, at, faked_with, fetching, fetching_typed, logout, packed_login, saved_login, steamship,
    steamship_in, summary_of, text_at, web_api, web_api_variables,
};

#[test]
fn an_accounts_name_in_a_failed_fetch_changes_neither_its_exit_code_nor_steamships_words() {
    for (name, said, exit, code, words) in [
        (
            "limit",
            "Logging in user 'limit' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)",
            5_i32,
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "steam",
            "Logging in user 'steam' [U:1:0] to Steam Public...FAILED (Rate Limit Exceeded)",
            5_i32,
            6_i32,
            "Steam is limiting login attempts. Wait, then try again",
        ),
        (
            "steamcmd",
            "Logging in user 'steamcmd' [U:1:0] to Steam Public...OK",
            137_i32,
            1_i32,
            "steamcmd exited 137 without saying why",
        ),
    ] {
        let home = faked_with(&format!("#!/bin/sh\necho \"{said}\"\nexit {exit}\n"));
        saved_login(home.path());
        let (found, stdout, stderr) = fetching(
            home.path(),
            &["1000", "--branch", "testing", "--account", name],
            &[],
        );
        assert_eq!(found, Some(code), "{name}: {stdout}{stderr}");
        assert!(stderr.contains(words), "{name}: {stderr}");
    }
}

#[test]
fn a_build_that_is_not_a_number_says_what_a_build_id_is() {
    let folder = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(
        folder.path(),
        &["fetch", "--build", "x"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        stderr.contains("'x' is not a build ID. Give the number `builds` lists, such as 1234567"),
        "{stderr}"
    );
}

/// A steamcmd that fetches app 1000 as steamcmd 1792617903 did: it records the build,
/// `STEAMSHIP_FAKE_BUILD` or 4242, and the branch `-beta` names in the folder it was given, and
/// says the app is installed. It keeps what it was started with in `args`.
const FETCHES: &str = "#!/bin/sh\n\
    echo \"$*\" > \"$HOME/args\"\n\
    folder=; beta=\n\
    while [ $# -gt 0 ]; do\n\
      case \"$1\" in\n\
        +force_install_dir) folder=\"$2\"; shift ;;\n\
        -beta) beta=\"$2\"; shift ;;\n\
      esac\n\
      shift\n\
    done\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    mkdir -p \"$folder/steamapps\"\n\
    printf '\"AppState\"\\n{\\n\"buildid\" \"%s\"\\n\"UserConfig\"\\n{\\n\"BetaKey\" \"%s\"\\n}\\n}\\n' \
      \"${STEAMSHIP_FAKE_BUILD:-4242}\" \"$beta\" > \"$folder/steamapps/appmanifest_1000.acf\"\n\
    echo \"Success! App '1000' fully installed.\"\n";

#[test]
fn the_build_live_on_a_branch_is_fetched_into_a_folder_of_its_own_and_named_with_it() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let system = fetch::system_name(Platform::THIS);
    let folder = home
        .path()
        .join("fetched")
        .join("1000")
        .join(format!("testing-{system}"));

    let (code, stdout, stderr) =
        fetching(home.path(), &["1000", "--branch", "testing", "--json"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert_eq!(
        args.trim_end(),
        format!(
            "+@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +@sSteamCmdForcePlatformType \
             {system} +force_install_dir {} +login build_bot +app_update 1000 -beta testing \
             validate +quit",
            folder.display()
        )
    );
    assert!(
        stderr.contains(&format!(
            "app 1000: build 4242 of the testing branch is in {}",
            folder.display()
        )),
        "{stderr}"
    );
    let printed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!({
            "app": 1000_u32,
            "branch": "testing",
            "build_id": 4242_u64,
            "system": system,
            "folder": folder.display().to_string(),
        })
    );

    // The default branch, for another system, into a folder of the caller's.
    let into = tempfile::tempdir().unwrap();
    let chosen = into.path().join("play");
    let (default, said, _) = fetching(
        home.path(),
        &[
            "1000",
            "--system",
            "windows",
            "--output",
            &chosen.to_string_lossy(),
        ],
        &[],
    );
    assert_eq!(default, Some(0_i32), "{said}");
    let for_windows = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        for_windows.contains("+@sSteamCmdForcePlatformType windows +force_install_dir")
            && for_windows.ends_with("+app_update 1000 -beta public validate +quit\n"),
        "{for_windows}"
    );
    assert!(
        said.contains("app 1000: build 4242 of the default branch is in"),
        "{said}"
    );

    // Fetched again, the folder is checked and updated in place.
    let (again, again_said, again_why) = fetching(
        home.path(),
        &[
            "1000",
            "--build",
            "4242",
            "--output",
            &chosen.to_string_lossy(),
        ],
        &[],
    );
    assert_eq!(again, Some(0_i32), "{again_said}{again_why}");
    assert!(
        again_said.contains(&format!(
            "app 1000: build 4242 of the default branch is in {}",
            chosen.display()
        )),
        "{again_said}{again_why}"
    );
    let updated = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        updated.contains(&format!("+force_install_dir {} ", chosen.display()))
            && updated.ends_with("+app_update 1000 -beta public validate +quit\n"),
        "{updated}"
    );
}

#[test]
fn a_branch_named_in_another_case_is_fetched_under_steams_name() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let system = fetch::system_name(Platform::THIS);

    let (code, stdout, stderr) = fetching(home.path(), &["1000", "--branch", "Testing"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains(" -beta testing "), "{args}");
    let shown = format!("{stdout}{stderr}");
    assert!(
        shown.contains("build 4242 of the testing branch")
            && shown.contains(&format!("testing-{system}")),
        "{shown}"
    );
}

#[test]
fn a_branch_steam_lists_in_two_cases_is_fetched_under_the_name_typed() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let (host, _requests) = web_api(vec![(
        "200 OK",
        r#"{"response": {"result": 1, "betas": {
            "Testing": {"BuildID": 4100},
            "testing": {"BuildID": 4242}
        }}}"#,
    )]);
    let (code, stdout, stderr) = steamship(
        &["fetch", "1000", "--branch", "testing"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains(" -beta testing "), "{args}");
    assert!(
        format!("{stdout}{stderr}").contains("build 4242 of the testing branch"),
        "{stdout}{stderr}"
    );
}

#[test]
fn a_fetch_that_got_another_build_than_steam_shows_live_fails() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let (code, stdout, stderr) = fetching(
        home.path(),
        &["1000", "--branch", "testing", "--json"],
        &[("STEAMSHIP_FAKE_BUILD", "4243")],
    );
    assert_eq!(code, Some(1_i32), "{stderr}");
    assert!(
        stderr.contains("steamcmd put build 4243 of the testing branch in")
            && stderr.contains("where Steam's Web API showed build 4242 live on testing"),
        "{stderr}"
    );
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/app"), 1000_u32, "{printed}");
    assert_eq!(at(&printed, "/error/code"), "another-build", "{printed}");
    let message = text_at(&printed, "/error/message");
    assert!(
        message.contains("steamcmd put build 4243 of the testing branch in")
            && message.contains("where Steam's Web API showed build 4242 live on testing"),
        "{printed}"
    );
}

#[test]
fn a_fetch_steam_does_not_answer_prints_its_error_as_json() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("500 Internal Server Error", ""), ("200 OK", APPS)]);
    let (code, stdout, stderr) = steamship(
        &["fetch", "480", "--json"],
        Some(home.path()),
        &web_api_variables(&host),
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/app"), 480_u32, "{printed}");
    assert_eq!(at(&printed, "/error/code"), "http-error", "{printed}");
    assert!(
        text_at(&printed, "/error/message").starts_with("Steam answered HTTP 500"),
        "{printed}"
    );
}

#[test]
fn a_fetch_refused_a_login_prints_not_logged_in_as_json() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)\"\n\
         exit 5\n",
    );
    saved_login(home.path());
    let (code, stdout, stderr) = fetching(home.path(), &["1000", "--json"], &[]);
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/app"), 1000_u32, "{printed}");
    assert_eq!(at(&printed, "/error/code"), "not-logged-in", "{printed}");
    assert!(
        text_at(&printed, "/error/message").contains("Expired Login Auth Code"),
        "{printed}"
    );
}

#[test]
fn a_fetch_steam_refuses_prints_its_reasons_as_json() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo \"ERROR! Failed to install app '1000' (No subscription)\"\n\
         exit 8\n",
    );
    saved_login(home.path());
    let (code, stdout, stderr) = fetching(home.path(), &["1000", "--json"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/error/code"), "steamcmd-failed", "{printed}");
    assert!(
        text_at(&printed, "/error/message")
            .contains("Failed to install app '1000' (No subscription)"),
        "{printed}"
    );
}

#[test]
fn a_fetch_refused_after_steam_answered_prints_nothing() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let (code, stdout, stderr) =
        fetching(home.path(), &["1000", "--branch", "nightly", "--json"], &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("app 1000 has no branch nightly"),
        "{stderr}"
    );
}

#[test]
fn a_steamcmd_altered_before_the_download_is_steamcmd_install_as_json() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let program = home.path().join(install::FOLDER).join("steamcmd.sh");
    fs::write(&program, "#!/bin/sh\necho changed\n").unwrap();
    let (code, stdout, stderr) = fetching(home.path(), &["1000", "--json"], &[]);
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/app"), 1000_u32, "{printed}");
    assert_eq!(at(&printed, "/error/code"), "steamcmd-install", "{printed}");
    assert!(
        text_at(&printed, "/error/message").contains("steamcmd.sh has changed"),
        "{printed}"
    );
}

#[test]
fn a_fetch_is_refused_before_steamcmd_for_a_branch_it_cannot_fetch_or_a_folder_not_its_own() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let other = files.path().join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("notes.txt"), "mine").unwrap();
    let on_testing = files.path().join("testing");
    fs::create_dir_all(on_testing.join("steamapps")).unwrap();
    fs::write(
        on_testing.join("steamapps").join("appmanifest_1000.acf"),
        "\"AppState\"\n{\n\"buildid\" \"4242\"\n\"MountedConfig\"\n{\n\"BetaKey\" \"testing\"\n}\n}\n",
    )
    .unwrap();
    let refusal = "app 1000's friends branch takes a password. Set STEAMSHIP_BRANCH_PASSWORD to \
                   it, or run fetch without --json and type it when asked";
    for (args, said) in [
        (vec!["1000", "--branch", "friends"], refusal.to_owned()),
        (
            vec!["1000", "--branch", "friends", "--json"],
            refusal.to_owned(),
        ),
        (
            vec!["1000", "--build", "4300"],
            "build 4300 is live on no branch of app 1000".to_owned(),
        ),
        (
            vec!["1000", "--branch", "nightly"],
            "app 1000 has no branch nightly: its branches are friends, default, testing".to_owned(),
        ),
        (
            vec!["1000", "--output", other.to_str().unwrap()],
            format!("{}: holds files and no build of app 1000", other.display()),
        ),
        (
            vec!["1000", "--output", on_testing.to_str().unwrap()],
            format!(
                "{}: holds app 1000's testing branch. Fetch default into a folder of its own",
                on_testing.display()
            ),
        ),
    ] {
        let (code, stdout, stderr) = fetching(home.path(), &args, &[]);
        assert_eq!(code, Some(2_i32), "{args:?}: {stderr}");
        assert!(stderr.contains(&said), "{said:?} in {stderr}");
        assert!(!home.path().join("args").exists(), "steamcmd was not run");
        if args.contains(&"--json") {
            assert!(stdout.is_empty(), "no prompt on standard output: {stdout}");
        }
    }
    assert_eq!(fs::read_to_string(other.join("notes.txt")).unwrap(), "mine");

    // An empty variable gives no password.
    let (empty, _, empty_said) = fetching(
        home.path(),
        &["1000", "--branch", "friends"],
        &[("STEAMSHIP_BRANCH_PASSWORD", "")],
    );
    assert_eq!(empty, Some(2_i32), "{empty_said}");
    assert!(empty_said.contains(refusal), "{empty_said}");

    // Nor does Enter alone at the prompt.
    let (entered, _, entered_said) = fetching_typed(
        home.path(),
        &["1000", "--branch", "friends"],
        &[],
        Some("\n"),
    );
    assert_eq!(entered, Some(2_i32), "{entered_said}");
    assert!(entered_said.contains(refusal), "{entered_said}");
    assert!(!home.path().join("args").exists(), "steamcmd was not run");

    // A record that is there but cannot be read is a failure, not a folder of other files.
    let unreadable = files.path().join("unreadable");
    fs::create_dir_all(unreadable.join("steamapps").join("appmanifest_1000.acf")).unwrap();
    let (code, _, stderr) = fetching(
        home.path(),
        &["1000", "--output", &unreadable.to_string_lossy()],
        &[],
    );
    assert_eq!(code, Some(1_i32), "{stderr}");
    assert!(!stderr.contains("holds files and no build"), "{stderr}");
}

/// A steamcmd that logs in, waits at its console for a command as steamcmd 1792617903 does, and
/// echoes it there as that console does. Typed the fetch of app 1000's friends branch with the
/// password `STEAMSHIP_FAKE_PASSWORD` or `open-sesame`, it records build 4100 of that branch in
/// its folder and says the app is installed. With any other, it refuses the password in
/// steamcmd's words. It then waits at the
/// console again, and keeps what it was started with in `args`, what was typed in `typed`, and
/// what was typed last in `quit`.
const TYPED_FETCHES: &str = "#!/bin/sh\n\
    echo \"$*\" > \"$HOME/args\"\n\
    folder=\n\
    while [ $# -gt 0 ]; do\n\
      case \"$1\" in\n\
        +force_install_dir) folder=\"$2\"; shift ;;\n\
      esac\n\
      shift\n\
    done\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    printf 'Steam>'\n\
    read -r typed\n\
    printf '%s' \"$typed\" > \"$HOME/typed\"\n\
    echo \"$typed\"\n\
    taken=\"app_update 1000 -beta friends -betapassword ${STEAMSHIP_FAKE_PASSWORD:-open-sesame} \
    validate\"\n\
    if [ \"$typed\" = \"$taken\" ]; then\n\
      mkdir -p \"$folder/steamapps\"\n\
      printf '\"AppState\"\\n{\\n\"buildid\" \"4100\"\\n\"UserConfig\"\\n{\\n\"BetaKey\" \"friends\"\\n}\\n}\\n' \
        > \"$folder/steamapps/appmanifest_1000.acf\"\n\
      echo \"Success! App '1000' fully installed.\"\n\
    else\n\
      echo 'ERROR! Password check for AppId 1000 returned error Failure.'\n\
    fi\n\
    printf 'Steam>'\n\
    read -r quit\n\
    printf '%s' \"$quit\" > \"$HOME/quit\"\n";

#[test]
fn the_password_of_a_branch_is_typed_into_steamcmds_console_and_kept_when_asked() {
    let home = common::store::Home::of(faked_with(TYPED_FETCHES), logout);
    saved_login(home.path());
    let password = [("STEAMSHIP_BRANCH_PASSWORD", "open-sesame")];
    let named = ["1000", "--branch", "friends"];
    let friends = ["1000", "--branch", "friends", "--keep-password"];
    let (code, stdout, stderr) = fetching(home.path(), &friends, &password);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        args.trim_end().ends_with("+login build_bot"),
        "steamcmd waits at its console: {args}"
    );
    assert_eq!(
        fs::read_to_string(home.path().join("typed")).unwrap(),
        "app_update 1000 -beta friends -betapassword open-sesame validate"
    );
    assert_eq!(
        fs::read_to_string(home.path().join("quit")).unwrap(),
        "quit"
    );
    let said = format!("{stdout}{stderr}");
    assert!(
        said.contains("from STEAMSHIP_BRANCH_PASSWORD")
            && said.contains("app 1000: build 4100 of the friends branch is in")
            && said.contains("kept in")
            && !said.contains("open-sesame"),
        "{said}"
    );

    // The kept password opens the branch next time.
    let (again, again_said) = fetching_said(home.path(), &named, &[]);
    assert_eq!(again, Some(0_i32), "{again_said}");
    assert!(again_said.contains("kept in"), "{again_said}");

    // A wrong one given is refused, and neither kept nor the kept one forgotten.
    let wrong = [("STEAMSHIP_BRANCH_PASSWORD", "open-barley")];
    let (refused, refused_said) = fetching_said(home.path(), &friends, &wrong);
    assert_eq!(refused, Some(1_i32), "{refused_said}");
    assert!(
        refused_said.contains("Password check for AppId 1000 returned error Failure.")
            && refused_said.contains("Steam refused the branch's password")
            && !refused_said.contains("forgotten")
            && !refused_said.contains("open-barley"),
        "{refused_said}"
    );

    // Once the branch takes another, the kept one Steam refuses is forgotten.
    let changed = [("STEAMSHIP_FAKE_PASSWORD", "open-millet")];
    let (stale, stale_said) = fetching_said(home.path(), &named, &changed);
    assert_eq!(stale, Some(1_i32), "{stale_said}");
    assert!(stale_said.contains("is forgotten"), "{stale_said}");
    let (none, none_said) = fetching_said(home.path(), &named, &changed);
    assert_eq!(none, Some(2_i32), "nothing kept to use: {none_said}");
}

#[test]
fn a_branch_password_steam_refuses_is_steamcmd_failed_as_json() {
    let home = common::store::Home::of(faked_with(TYPED_FETCHES), logout);
    saved_login(home.path());
    let given = ["1000", "--branch", "friends", "--json"];
    let wrong = [("STEAMSHIP_BRANCH_PASSWORD", "open-barley")];
    let (code, stdout, stderr) = fetching(home.path(), &given, &wrong);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/error/code"), "steamcmd-failed", "{printed}");
    let message = text_at(&printed, "/error/message");
    assert!(
        message.contains("Steam refused the branch's password")
            && message.ends_with("Fetch again with the password the branch takes now"),
        "{printed}"
    );
    assert!(
        !format!("{stdout}{stderr}").contains("open-barley"),
        "{stdout}{stderr}"
    );
}

#[test]
fn a_kept_branch_password_steam_refuses_is_forgotten_and_says_so_as_json() {
    let home = common::store::Home::of(faked_with(TYPED_FETCHES), logout);
    saved_login(home.path());
    let given = ["1000", "--branch", "friends", "--json"];
    let kept = ["1000", "--branch", "friends", "--keep-password"];
    let right = [("STEAMSHIP_BRANCH_PASSWORD", "open-sesame")];
    let (stored, stored_said) = fetching_said(home.path(), &kept, &right);
    assert_eq!(stored, Some(0_i32), "{stored_said}");
    let changed = [("STEAMSHIP_FAKE_PASSWORD", "open-millet")];
    let (code, stdout, stderr) = fetching(home.path(), &given, &changed);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let printed = summary_of(&stdout);
    assert_eq!(at(&printed, "/error/code"), "steamcmd-failed", "{printed}");
    assert!(
        text_at(&printed, "/error/message").contains("is forgotten"),
        "{printed}"
    );
    assert!(!format!("{stdout}{stderr}").contains("open-sesame"));
}

/// [`fetching`], with everything it printed on either stream as one text.
fn fetching_said(home: &Path, args: &[&str], variables: &[(&str, &str)]) -> (Option<i32>, String) {
    let (code, stdout, stderr) = fetching(home, args, variables);
    (code, format!("{stdout}{stderr}"))
}

#[test]
fn a_typed_password_is_kept_on_enter_or_when_asked_and_not_on_n() {
    let named = ["1000", "--branch", "friends"];
    let keeping = ["1000", "--branch", "friends", "--keep-password"];
    for (args, typed, kept) in [
        (named.as_slice(), "open-sesame\n\n", true),
        (named.as_slice(), "open-sesame\nn\n", false),
        (keeping.as_slice(), "open-sesame\n", true),
    ] {
        let home = common::store::Home::of(faked_with(TYPED_FETCHES), logout);
        saved_login(home.path());
        let (code, stdout, stderr) = fetching_typed(home.path(), args, &[], Some(typed));
        let said = format!("{stdout}{stderr}");
        assert_eq!(code, Some(0_i32), "{typed:?}: {said}");
        assert_eq!(
            fs::read_to_string(home.path().join("typed")).unwrap(),
            "app_update 1000 -beta friends -betapassword open-sesame validate"
        );
        assert!(!said.contains("open-sesame"), "{said}");
        assert_eq!(said.contains("not kept"), !kept, "{typed:?}: {said}");
        assert_eq!(
            said.contains("Enter keeps the password"),
            args.len() == named.len(),
            "asked only without --keep-password: {said}"
        );
        let (again, again_stdout, again_stderr) = fetching(home.path(), &named, &[]);
        assert_eq!(
            again,
            Some(if kept { 0_i32 } else { 2_i32 }),
            "{typed:?}: {again_stdout}{again_stderr}"
        );
    }
}

#[test]
fn a_typed_fetch_whose_steamcmd_will_never_reach_steam_is_stopped_at_once() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo 'unable to load trusted SSL root certificates'\n\
         printf 'Steam>'\n\
         sleep 30\n",
    );
    saved_login(home.path());
    let started = Instant::now();
    let (code, stdout, stderr) = fetching(
        home.path(),
        &["1000", "--branch", "friends"],
        &[("STEAMSHIP_BRANCH_PASSWORD", "open-sesame")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stderr.contains("steamcmd found no certificates to check Steam's servers against"),
        "{stderr}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "not waited for: {:?}",
        started.elapsed()
    );
}

#[test]
fn the_password_of_a_branch_goes_on_steamcmds_command_line_only_when_asked() {
    let home = faked_with(FETCHES);
    saved_login(home.path());
    let (code, stdout, stderr) = fetching(
        home.path(),
        &["1000", "--branch", "friends", "--password-on-command-line"],
        &[
            ("STEAMSHIP_BRANCH_PASSWORD", "open sesame"),
            ("STEAMSHIP_FAKE_BUILD", "4100"),
        ],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        args.ends_with("+app_update 1000 -beta friends -betapassword open sesame validate +quit\n"),
        "{args}"
    );
    let said = format!("{stdout}{stderr}");
    assert!(
        said.contains("the password goes on steamcmd's command line")
            && !said.contains("open sesame"),
        "{said}"
    );

    // Without it, a password the console would split is refused before steamcmd runs.
    fs::remove_file(home.path().join("args")).unwrap();
    let (split, split_said) = fetching_said(
        home.path(),
        &["1000", "--branch", "friends"],
        &[("STEAMSHIP_BRANCH_PASSWORD", "open sesame")],
    );
    assert_eq!(split, Some(2_i32), "{split_said}");
    assert!(
        split_said.contains("only be given with --password-on-command-line"),
        "{split_said}"
    );
    assert!(!home.path().join("args").exists(), "steamcmd was not run");
}

#[test]
fn a_fetch_in_ci_is_done_as_the_account_handed_over() {
    // No login saved and no account remembered: only what `steamship ci` packed.
    let home = faked_with(FETCHES);
    let (code, stdout, stderr) = fetching(
        home.path(),
        &["1000"],
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains("+login build_bot "), "{args}");
    assert!(
        steamcmd::saved_login(home.path(), Platform::THIS).exists(),
        "the login handed over was put where steamcmd looks"
    );
}

#[test]
fn a_fetch_steam_refuses_says_why() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo \"ERROR! Failed to install app '1000' (No subscription)\"\n\
         exit 8\n",
    );
    saved_login(home.path());
    let (code, _, stderr) = fetching(home.path(), &["1000"], &[]);
    assert_eq!(code, Some(1_i32), "{stderr}");
    assert!(
        stderr.contains("Failed to install app '1000' (No subscription)")
            && stderr.contains("the build account holds no licence for app 1000"),
        "{stderr}"
    );
}

#[test]
fn a_fetch_steamcmd_gave_up_on_in_its_content_log_alone_says_why_from_there() {
    // Where steamcmd keeps its logs, from the home it is given as `HOME`.
    let log = steamcmd::state_folder(Path::new(""), Platform::THIS)
        .join("logs")
        .join("content_log.txt");
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID 1000 update canceled : Missing app info for 1000 (Missing configuration)' \
         >> \"$HOME/{}\"\n\
         exit 8\n",
        log.display()
    ));
    saved_login(home.path());
    let earlier = home.path().join(&log);
    fs::create_dir_all(earlier.parent().unwrap()).unwrap();
    fs::write(&earlier, "AppID 1000 update canceled : an earlier run's\n").unwrap();
    let (code, _, stderr) = fetching(home.path(), &["1000"], &[]);
    assert_eq!(code, Some(1_i32), "{stderr}");
    assert!(
        stderr.contains(
            "steamcmd cancelled the download: Missing app info for 1000 (Missing configuration)"
        ) && !stderr.contains("an earlier run's"),
        "{stderr}"
    );
}
