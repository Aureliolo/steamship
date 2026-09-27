//! The `steamship` command itself: what it prints and the code it exits with.
//!
//! These run after the library's own tests on purpose: a change that broke the installer would
//! send `install` here to Valve's CDN, and the unit tests that fail at once on such a change are
//! the ones that should say so.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use steamship::account::Account;
use steamship::install;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::terminal::{Event, Reader};
#[cfg(unix)]
use steamship::unix::Terminal;
#[cfg(windows)]
use steamship::windows::Terminal;
#[cfg(unix)]
use steamship::{ci, steamcmd};

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
    steamship_in(Path::new("."), args, home, variables)
}

/// [`steamship`], run in `folder`.
fn steamship_in(
    folder: &Path,
    args: &[&str],
    home: Option<&Path>,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_steamship"));
    let _: &mut Command = command.current_dir(folder);
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

/// The first failure steamship reported, without the cross before it; or, when it reported
/// none, all it wrote, for the assertion to show.
fn failure(stderr: &str) -> &str {
    stderr
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("\u{2717} "))
        .unwrap_or(stderr)
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
    assert!(stdout.starts_with("steamship install\n"), "{stdout}");
    assert!(stdout.contains(", pinned and verified\n"), "{stdout}");
    let folder = home.path().join(install::FOLDER);
    assert!(
        stdout.ends_with(&format!("  folder    {}\n", folder.display())),
        "{stdout}"
    );
}

#[test]
fn install_names_what_changed_and_exits_4() {
    let home = recorded(&format!("file {} steamcmd.exe\n", "0".repeat(64)));
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!(code, Some(4_i32));
    assert!(stdout.ends_with("\u{2717} not as pinned\n"), "{stdout}");
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
    let (code, _, stderr) = run(None);
    assert_eq!(code, Some(1_i32));
    assert!(
        failure(&stderr).starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}

#[test]
fn logout_forgets_steamcmds_saved_login_on_every_system_and_the_account() {
    let home = tempfile::tempdir().unwrap();
    let saved = [
        "steamcmd/config/config.vdf",
        "Steam/config/config.vdf",
        "Library/Application Support/Steam/config/config.vdf",
        "steamcmd/userdata/12345/config/localconfig.vdf",
    ];
    for file in saved {
        let path = home.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    }
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let kept = home.path().join("steamcmd/config/other.vdf");
    fs::write(&kept, "kept").unwrap();
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        stdout,
        "steamship logout\n  \
         login     \u{2713} forgotten\n  \
         account   \u{2713} forgotten\n  \
         \u{2713} logged out\n    \
         the next upload needs `steamship login` first\n"
    );
    for file in saved {
        assert!(!home.path().join(file).exists(), "{file}");
    }
    assert!(!home.path().join("account").exists());
    assert!(kept.exists(), "only the login is removed");
    let (again, said, _) = steamship(&["logout"], Some(home.path()), &[]);
    assert_eq!(again, Some(0_i32));
    assert!(
        said.contains("  login     none saved\n  account   none remembered\n"),
        "{said}"
    );
}

#[test]
fn help_lists_every_command_with_one_short_line() {
    let (code, stdout, _) = steamship(&["--help"], Some(Path::new(".")), &[]);
    assert_eq!(code, Some(0_i32));
    assert!(
        !stdout.contains("uploads your build to Steam"),
        "no banner in a log"
    );
    assert!(
        stdout.contains("Usage: steamship <COMMAND>\n"),
        "named as it is typed, on Windows too: {stdout}"
    );
    for line in [
        "  login    Log in to Steam, once, for uploads\n",
        "  status   Show the login and steamcmd, checking the login with Steam\n",
        "  check    Check the build scripts, without logging in\n",
        "  upload   Check, build and upload, then print the build ID\n",
        "  ci       Set up uploads from CI, the login kept as a secret\n",
        "  logout   Forget the saved login\n",
        "  install  Install or verify the pinned steamcmd\n",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
}

#[test]
fn status_with_nothing_saved_says_to_log_in_changes_nothing_and_exits_3() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(3_i32), "{stdout}{stderr}");
    assert_eq!(
        stdout,
        format!(
            "steamship status\n  \
             version   {}\n  \
             home      {}\n  \
             account   none remembered\n  \
             steamcmd  not installed; `steamship login` installs it\n  \
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

/// A home with a login saved where steamcmd keeps it on this system, for `build_bot`, remembered.
#[cfg(unix)]
fn saved_login(home: &Path) {
    let config = steamcmd::saved_login(home, Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    fs::write(home.join("account"), "build_bot\n").unwrap();
}

/// A home with the login `saved_login` makes, whose steamcmd keeps what it was started with in
/// `args`, says `said` and exits `code`.
#[cfg(unix)]
fn checking(said: &str, code: u8) -> tempfile::TempDir {
    let home = answering(said, code);
    saved_login(home.path());
    home
}

/// [`checking`] with no login saved.
#[cfg(unix)]
fn answering(said: &str, code: u8) -> tempfile::TempDir {
    faked_with(&format!(
        "#!/bin/sh\necho \"$*\" > \"$HOME/args\"\nprintf '%s\\r\\n' '{said}'\nexit {code}\n"
    ))
}

/// The login `saved_login` makes, packed as `steamship ci` packs it.
#[cfg(unix)]
fn packed_login() -> String {
    let home = tempfile::tempdir().unwrap();
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    let account = Account::parse("build_bot").unwrap();
    let login = ci::Login::saved(home.path(), Platform::THIS, account).unwrap();
    login.packed().to_string()
}

#[cfg(unix)]
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
        stdout.contains("  login     from STEAMSHIP_LOGIN\n"),
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

#[cfg(unix)]
#[test]
fn an_upload_in_ci_logs_in_as_the_account_handed_over() {
    let home = faked();
    let (_project, script) = project(true);
    let script = script.to_str().unwrap();
    let (_, stdout, _) = steamship(
        &["upload", script, "--version", "1.4.0", "--preview"],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", &packed_login())],
    );
    let console = fs::read_to_string(home.path().join("apps/1000/output/steamcmd.log")).unwrap();
    assert!(console.contains(" +login build_bot "), "{stdout}{console}");
    assert!(steamcmd::saved_login(home.path(), Platform::THIS).exists());
}

#[test]
fn a_login_handed_over_that_is_not_one_is_refused_without_repeating_it() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["status"],
        Some(home.path()),
        &[("STEAMSHIP_LOGIN", "steamship-login-1:secret_name:zz")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_LOGIN does not hold a login packed by `steamship ci`"
    );
    assert!(!format!("{stdout}{stderr}").contains("secret_name"));
}

#[cfg(unix)]
#[test]
fn a_login_handed_over_that_is_not_text_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_steamship"))
        .arg("status")
        .env("STEAMSHIP_HOME", home.path())
        .env(
            "STEAMSHIP_LOGIN",
            OsStr::from_bytes(b"steamship-login-1:\xff"),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2_i32));
    assert_eq!(
        failure(&String::from_utf8_lossy(&output.stderr)),
        "STEAMSHIP_LOGIN does not hold a login packed by `steamship ci`"
    );
}

#[test]
fn ci_with_the_script_named_goes_on_to_the_account() {
    let (_project, root) = github_project();
    let home = tempfile::tempdir().unwrap();
    let script = root.join("steam").join("app_build.vdf");
    let (code, stdout, stderr) = steamship(
        &[
            "ci",
            "--script",
            script.to_str().unwrap(),
            "--output",
            "unused",
        ],
        Some(home.path()),
        &[("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains(&format!("  script    {}\n", script.display())),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "name the build account with --account or STEAMSHIP_ACCOUNT"
    );
}

#[test]
fn ci_in_a_repository_without_an_app_build_script_says_so_and_exits_2() {
    let empty = tempfile::tempdir().unwrap();
    git(empty.path(), &["init", "--quiet"]);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(empty.path(), &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        failure(&stderr).starts_with("no app build script in "),
        "{stderr}"
    );
    assert!(stderr.contains("; name one with --script"), "{stderr}");
}

#[test]
fn an_empty_account_variable_names_no_account() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, _) = steamship(&["status"], Some(home.path()), &[("STEAMSHIP_ACCOUNT", "")]);
    assert_eq!(code, Some(3_i32), "{stdout}");
    assert!(stdout.contains("  account   none remembered\n"), "{stdout}");
}

#[test]
fn ci_refuses_a_secret_github_would_not_take_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(&["ci", "--secret", "GITHUB_LOGIN"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        failure(&stderr).starts_with("GITHUB_LOGIN cannot name a GitHub secret"),
        "{stderr}"
    );
}

#[test]
fn ci_outside_a_repository_asks_for_the_script_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(outside.path(), &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert_eq!(
        failure(&stderr),
        "this is not a Git repository; run it in the game's, or name the script with --script"
    );
}

#[test]
fn ci_finds_the_script_but_needs_a_github_origin_and_exits_2() {
    let (project, _) = project(true);
    let root = project.path().join("fgm gate (x86) 1a2b");
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_in(&root, &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let found = Path::new("steam").join("app_build.vdf");
    assert!(
        stdout.contains(&format!("  script    {}, found\n", found.display())),
        "{stdout}"
    );
    assert!(
        stdout.contains("\u{2713} 1000, 1 depot, 1 file, checked"),
        "{stdout}"
    );
    assert!(
        failure(&stderr).starts_with("the script's repository has no GitHub origin"),
        "{stderr}"
    );
}

#[test]
fn ci_with_several_scripts_and_nobody_to_ask_names_them_and_exits_2() {
    let (project, _) = project(true);
    let root = project.path().join("fgm gate (x86) 1a2b");
    let _: u64 = fs::copy(
        root.join("steam/app_build.vdf"),
        root.join("steam/app_build_demo.vdf"),
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(&root, &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    let steam = Path::new("steam");
    assert_eq!(
        failure(&stderr),
        format!(
            "several app build scripts, {}, {}; name one with --script",
            steam.join("app_build.vdf").display(),
            steam.join("app_build_demo.vdf").display()
        )
    );
}

#[cfg(unix)]
#[test]
fn ci_writes_the_checked_login_to_a_file_for_another_ci() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (project, _) = project(true);
    let root = project.path().join("fgm gate (x86) 1a2b");
    let output = project.path().join("login.txt");
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci", "--output", output.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  steam     \u{2713} Steam takes the saved login\n"),
        "{stdout}"
    );
    assert!(stdout.contains("\u{2713} ready for CI\n"), "{stdout}");
    let written = fs::read_to_string(&output).unwrap();
    assert_eq!(written, packed_login());
    assert!(!stdout.contains(&written), "the login is never shown");
    let unpacked = ci::Login::unpack(&written).unwrap();
    assert_eq!(unpacked.account().name(), "build_bot");
}

#[cfg(unix)]
#[test]
fn ci_with_no_saved_login_says_to_log_in_first_and_exits_3() {
    let home = answering("", 0);
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (project, _) = project(true);
    let root = project.path().join("fgm gate (x86) 1a2b");
    let output = project.path().join("login.txt");
    let (code, _, stderr) = steamship_in(
        &root,
        &["ci", "--output", output.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(3_i32), "{stderr}");
    assert!(
        stderr.ends_with("log in here first with steamship login\n"),
        "{stderr}"
    );
    assert!(!output.exists());
}

/// The game's repository from [`project`], cloned from GitHub as far as its `origin` says.
fn github_project() -> (tempfile::TempDir, PathBuf) {
    let (project, _) = project(true);
    let root = project.path().join("fgm gate (x86) 1a2b");
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Aureliolo/some-game.git",
        ],
    );
    (project, root)
}

/// A stand-in for the GitHub command line, first on a `PATH` that is otherwise the test's own.
/// `gh secret set` keeps what it was given and how beside itself, and refuses when
/// `STEAMSHIP_FAKE_GH_REFUSE` is set; `gh api` answers a commit, or fails when
/// `STEAMSHIP_FAKE_GH_OFFLINE` is set.
#[cfg(unix)]
fn fake_gh() -> (tempfile::TempDir, String) {
    use std::env;
    use std::os::unix::fs::PermissionsExt as _;

    let bin = tempfile::tempdir().unwrap();
    let gh = bin.path().join("gh");
    fs::write(
        &gh,
        "#!/bin/sh\n\
         here=\"$(dirname \"$0\")\"\n\
         case \"$1\" in\n\
         secret)\n\
         echo \"$*\" > \"$here/args\"\n\
         cat > \"$here/secret\"\n\
         [ -z \"$STEAMSHIP_FAKE_GH_REFUSE\" ] || { echo 'HTTP 403: Resource not accessible' >&2; exit 1; } ;;\n\
         api)\n\
         [ -z \"$STEAMSHIP_FAKE_GH_OFFLINE\" ] || exit 1\n\
         echo 0123456789abcdef0123456789abcdef01234567 ;;\n\
         esac\n",
    )
    .unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.path().display(), env::var("PATH").unwrap());
    (bin, path)
}

#[cfg(unix)]
#[test]
fn ci_sets_the_secret_through_gh_and_shows_the_step_pinned_by_commit() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) =
        steamship_in(&root, &["ci"], Some(home.path()), &[("PATH", &path)]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  repo      Aureliolo/some-game, from origin\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  secret    \u{2713} STEAMSHIP_LOGIN set on Aureliolo/some-game\n"),
        "{stdout}"
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.ends_with(&format!(
            "\n      - name: Upload to Steam\n        \
             uses: Aureliolo/steamship@0123456789abcdef0123456789abcdef01234567 # v{version}\n        \
             with:\n          \
             script: steam/app_build.vdf\n          \
             version: ${{{{ github.ref_name }}}}\n          \
             login: ${{{{ secrets.STEAMSHIP_LOGIN }}}}\n"
        )),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(gh.path().join("args")).unwrap(),
        "secret set STEAMSHIP_LOGIN --repo Aureliolo/some-game\n"
    );
    let secret = fs::read_to_string(gh.path().join("secret")).unwrap();
    assert_eq!(secret, packed_login());
    assert!(!stdout.contains(&secret), "the login is never shown");
}

#[cfg(unix)]
#[test]
fn ci_takes_the_repository_and_secret_named_and_pins_the_tag_when_github_cannot_be_asked() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &[
            "ci",
            "--repo",
            "Aureliolo/other-game",
            "--secret",
            "STEAM_UPLOAD",
        ],
        Some(home.path()),
        &[("PATH", &path), ("STEAMSHIP_FAKE_GH_OFFLINE", "1")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  repo      Aureliolo/other-game, as named\n"),
        "{stdout}"
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(&format!(
            "uses: Aureliolo/steamship@v{version} # v{version}\n"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("login: ${{ secrets.STEAM_UPLOAD }}\n"),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(gh.path().join("args")).unwrap(),
        "secret set STEAM_UPLOAD --repo Aureliolo/other-game\n"
    );
}

#[cfg(unix)]
#[test]
fn ci_says_what_gh_answered_when_it_cannot_set_the_secret_and_exits_1() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (_gh, path) = fake_gh();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", &path), ("STEAMSHIP_FAKE_GH_REFUSE", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} not set\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "gh could not set the secret: HTTP 403: Resource not accessible"
    );
    assert!(
        stderr.ends_with("if gh is not logged in, run gh auth login\n"),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn ci_without_gh_says_where_to_get_it_and_exits_1() {
    use std::os::unix::fs::symlink;

    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let bin = tempfile::tempdir().unwrap();
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let git = String::from_utf8(git.stdout).unwrap();
    symlink(git.trim(), bin.path().join("git")).unwrap();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", bin.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} gh is not installed\n"),
        "{stdout}"
    );
    assert_eq!(failure(&stderr), "the GitHub command line is needed");
    assert!(
        stderr.ends_with(
            "install it from https://cli.github.com, or write the login to a file with --output\n"
        ),
        "{stderr}"
    );
}

#[test]
fn ci_refuses_a_repository_that_is_not_an_owner_and_name_and_exits_2() {
    let (_project, root) = github_project();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(
        &root,
        &["ci", "--repo", "Aureliolo/some-game/extra"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert_eq!(
        failure(&stderr),
        "Aureliolo/some-game/extra is not a repository's owner/name"
    );
}

#[test]
fn ci_on_a_terminal_asks_which_script_when_there_are_several() {
    let (_project, root) = github_project();
    let _: u64 = fs::copy(
        root.join("steam/app_build.vdf"),
        root.join("steam/app_build_demo.vdf"),
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let steamship = env!("CARGO_BIN_EXE_steamship");
    // As in login_then_look: `cmd` reads no quoting, so the path is written without.
    #[cfg(windows)]
    let line = format!(
        "cd /d {} & {steamship} ci --account build_bot",
        root.display()
    );
    #[cfg(unix)]
    let line = format!(
        "cd '{}' && '{steamship}' ci --account build_bot",
        root.display()
    );
    let mut session = Session::start(&line, home.path());
    session.wait_for("script");
    session.wait_for("2");
    session.type_in("2\r");
    session.wait_for("log in here first");
    let _: Option<i32> = session.end().wait().unwrap();
    let seen = session.seen();
    let demo = Path::new("steam").join("app_build_demo.vdf");
    assert!(
        seen.contains(&format!("  2         {}\n", demo.display())),
        "{seen}"
    );
    assert!(
        seen.contains("\u{2713} 1000, 1 depot, 1 file, checked"),
        "{seen}"
    );
}

#[cfg(unix)]
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

#[cfg(unix)]
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
        stdout.contains("  steam     \u{2717} Steam refused the saved login\n"),
        "{stdout}"
    );
    assert_eq!(failure(&stderr), "not logged in: Expired Login Auth Code");
    assert!(
        stderr.ends_with("log the build account in again with steamship login\n"),
        "{stderr}"
    );
    assert!(!format!("{stdout}{stderr}").contains("build_bot"));
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
#[test]
fn status_with_steamcmd_altered_says_so_and_exits_4() {
    let home = faked();
    let program = home.path().join(install::FOLDER).join("steamcmd.sh");
    fs::write(&program, "#!/bin/sh\necho changed\n").unwrap();
    let (code, stdout, stderr) = steamship(&["status"], Some(home.path()), &[]);
    assert_eq!(code, Some(4_i32), "{stdout}{stderr}");
    assert!(
        stdout.ends_with("  steamcmd  \u{2717} not as pinned\n"),
        "{stdout}"
    );
    assert!(
        failure(&stderr).contains("steamcmd.sh has changed"),
        "{stderr}"
    );
}

/// A home whose recorded steamcmd is a script that prints what it was started with and exits
/// with `STEAMSHIP_FAKE_EXIT`, or 0. With `STEAMSHIP_FAKE_CHANGE` set it first changes itself, as
/// an update would; with `STEAMSHIP_FAKE_SAVED` set it logs in with the login it saved.
#[cfg(unix)]
fn faked() -> tempfile::TempDir {
    faked_with(
        "#!/bin/sh\n\
         [ -n \"$STEAMSHIP_FAKE_SAVED\" ] && echo 'Logging in using cached credentials.'\n\
         echo \"args: $*\"\n\
         echo \"home: $HOME\"\n\
         echo \"folder: $PWD\"\n\
         [ -n \"$STEAMSHIP_FAKE_CHANGE\" ] && echo >> \"$0\"\n\
         exit \"$((STEAMSHIP_FAKE_EXIT + 0))\"\n",
    )
}

/// A home whose recorded steamcmd is `script`.
#[cfg(unix)]
fn faked_with(script: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    use steamship::digest;

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
    let root = fs::canonicalize(home.path().join(install::FOLDER)).unwrap();
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

#[cfg(unix)]
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
            "\u{2713} already logged in\n    uploads use the saved login until it expires; \
             `steamship logout` forgets it\n"
        ),
        "{stdout}"
    );
}

#[cfg(unix)]
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
/// logs in only when that was `hunter2`, refusing it as steamcmd does otherwise.
///
/// With `STEAMSHIP_FAKE_GUARD` set to `code` it then asks for a Steam Guard code, kept in `code`
/// and taken only when it is `AB12C`; set to `app`, it waits for the login to be approved in the
/// Steam Mobile app, saying something steamship has no meaning for meanwhile, and with
/// `STEAMSHIP_FAKE_DENY` set the approval never comes.
#[cfg(unix)]
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
#[cfg(unix)]
fn login_typing(
    home: &Path,
    typed: &str,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_steamship"))
        .args(["login", "--account", "build_bot"])
        .env_remove("STEAMSHIP_ACCOUNT")
        .env("STEAMSHIP_HOME", home)
        .envs(variables.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(typed.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

/// steamship on a terminal of its own, as a person runs it, with what it shows read as it comes.
struct Session {
    terminal: Option<Terminal>,
    input: Box<dyn Write>,
    chunks: mpsc::Receiver<Vec<u8>>,
    reader: Reader,
    /// The lines shown so far, and the one being written.
    shown: String,
    pending: String,
}

impl Session {
    /// Starts `line` in the system's shell on a terminal, in a home of its own with no account in
    /// the environment and GitHub never asked for a newer steamship.
    fn start(line: &str, home: &Path) -> Self {
        Self::start_with(line, home, ("STEAMSHIP_NO_UPDATE_CHECK", "1"))
    }

    /// [`Session::start`], with `variable` set.
    fn start_with(line: &str, home: &Path, (name, value): (&str, &str)) -> Self {
        #[cfg(windows)]
        let (shell, args) = (r"C:\Windows\System32\cmd.exe", ["/d", "/s", "/c"]);
        #[cfg(unix)]
        let (shell, args) = ("/bin/sh", ["-c"]);
        let args: Vec<OsString> = args
            .iter()
            .copied()
            .chain([line])
            .map(OsString::from)
            .collect();
        let environment = [
            (
                OsString::from("STEAMSHIP_HOME"),
                home.as_os_str().to_owned(),
            ),
            (OsString::from(name), OsString::from(value)),
        ];
        let (terminal, mut output, input) =
            Terminal::start(Path::new(shell), &args, &environment, home).unwrap();
        let (sender, chunks) = mpsc::channel();
        let _reading = thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            while let Ok(length) = output.read(&mut chunk) {
                if length == 0 || sender.send(chunk.get(..length).unwrap().to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            terminal: Some(terminal),
            input: Box::new(input),
            chunks,
            reader: Reader::default(),
            shown: String::new(),
            pending: String::new(),
        }
    }

    /// Everything shown so far.
    fn seen(&self) -> String {
        format!("{}{}", self.shown, self.pending)
    }

    /// Reads on until `text` has been shown, or fails the test after half a minute.
    fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
        while !self.seen().contains(text) {
            let left = deadline.saturating_duration_since(Instant::now());
            let chunk = self.chunks.recv_timeout(left);
            assert!(chunk.is_ok(), "{text:?} never shown in {:?}", self.seen());
            let chunk = chunk.unwrap();
            self.pending.clear();
            for event in self.reader.read(&chunk) {
                match event {
                    Event::Line(line) => {
                        self.shown.push_str(&line);
                        self.shown.push('\n');
                    }
                    Event::Waiting(started) => self.pending = started,
                }
            }
        }
    }

    fn type_in(&mut self, keys: &str) {
        self.input.write_all(keys.as_bytes()).unwrap();
        self.input.flush().unwrap();
    }

    /// The shell's terminal, to wait for.
    const fn end(&mut self) -> Terminal {
        self.terminal.take().unwrap()
    }
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
    #[cfg(windows)]
    let line = format!("set STEAMSHIP_ACCOUNT=& {steamship} login & set /p after=next: ");
    #[cfg(unix)]
    let line = format!("unset STEAMSHIP_ACCOUNT; '{steamship}' login; echo \"exited $?\"; stty -a");
    line
}

#[test]
fn login_on_a_terminal_asks_for_the_account_shows_it_as_typed_and_refuses_a_bad_one() {
    let home = tempfile::tempdir().unwrap();
    let mut session = Session::start(&login_then_look(), home.path());
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
    assert!(!home.path().join("account").exists());
}

/// A home in which GitHub was last found, a moment ago, to have steamship 99.0.0 out.
fn told_of_99() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    fs::write(
        home.path().join("latest-release"),
        format!("{} 99.0.0\n", now.as_secs()),
    )
    .unwrap();
    home
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

#[test]
fn a_newer_steamship_is_told_at_a_terminal_with_how_to_get_it() {
    let home = told_of_99();
    let steamship = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship} logout");
    #[cfg(unix)]
    let line = format!("'{steamship}' logout");
    let mut session = Session::start_with(&line, home.path(), ("CI", ""));
    session.wait_for("download it from https://github.com/Aureliolo/steamship/releases/latest");
    let _: Option<i32> = session.end().wait().unwrap();
    assert!(
        session.seen().contains(&format!(
            "\n\n  \u{2191} steamship 99.0.0 is out, this is {}\n",
            env!("CARGO_PKG_VERSION")
        )),
        "{}",
        session.seen()
    );
}

#[test]
fn a_newer_steamship_is_not_told_in_a_log() {
    let home = told_of_99();
    let (code, stdout, stderr) = steamship(&["logout"], Some(home.path()), &[("CI", "")]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(!stdout.contains("99.0.0"), "{stdout}");
}

/// Checks that `session`'s terminal is as steamship found it, once steamship has `exited` so.
#[cfg(windows)]
fn looks_as_it_was(session: &mut Session, _exited: &str) {
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

#[cfg(unix)]
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
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(!seen.contains("Mobile app..."), "{seen}");
    assert!(seen.contains("  account   build_bot\n"), "{seen}");
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
        "name the build account with --account or STEAMSHIP_ACCOUNT"
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

fn git(folder: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=steamship",
            "-c",
            "user.email=steamship@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(folder)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A game's scripts and content, committed or not, in a folder named the way a gate worktree is.
fn project(committed: bool) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fgm gate (x86) 1a2b");
    let put = |relative: &str, contents: &str| {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    put("content/game.txt", "the game");
    put(
        "steam/depot.vdf",
        r#""DepotBuild" { "DepotID" "1001" "FileMapping" { "LocalPath" "*" "DepotPath" "." "Recursive" "1" } }"#,
    );
    put(
        "steam/app_build.vdf",
        r#""AppBuild" { "AppID" "1000" "ContentRoot" "../content" "SetLive" "testing" "Depots" { "1001" "depot.vdf" } }"#,
    );
    if committed {
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "scripts"]);
    }
    let script = root.join("steam").join("app_build.vdf");
    (temp, script)
}

fn upload(
    script: &Path,
    home: &Path,
    extra: &[&str],
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let script = script.to_str().unwrap();
    let mut args = vec!["upload", script, "--account", "build_bot"];
    args.extend(extra);
    steamship(&args, Some(home), variables)
}

#[test]
fn upload_refuses_a_version_a_description_cannot_carry_and_exits_2() {
    let (_project, script) = project(true);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(&script, home.path(), &["--version", "1.0\""], &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        failure(&stderr).starts_with("the version \"1.0\\\"\" is not"),
        "{stderr}"
    );
}

#[test]
fn upload_refuses_scripts_outside_a_repository_and_exits_2() {
    let (_project, script) = project(false);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(&script, home.path(), &["--version", "1.0"], &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        stderr
            .trim_end()
            .ends_with("which the build description names"),
        "{stderr}"
    );
}

#[test]
fn upload_runs_check_first_and_exits_2_on_a_refusal() {
    let (project, script) = project(true);
    fs::write(
        project
            .path()
            .join("fgm gate (x86) 1a2b/content/steam_appid.txt"),
        "1000",
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.0"], &[]);
    assert_eq!(code, Some(2_i32));
    assert_eq!(stdout, "steamship upload\n");
    assert!(failure(&stderr).starts_with("refused: "), "{stderr}");
}

/// A token that logs the account in without a password, and a value from its local settings.
#[cfg(unix)]
const SECRETS: [&str; 2] = [
    "eyJhbGciOiJFZERTQSJ9.token.that.logs.in.without.a.password",
    "localconfig_secret_value_0123456789",
];

/// A home with a saved login holding [`SECRETS`], where steamcmd is a script that does what the
/// setup-steamcmd action once did (GHSA-mj96-mh85-r574): prints steamcmd's login files, into its
/// console and its build log. Then it reports the build finished; or, with
/// `STEAMSHIP_FAKE_LOGGED_OUT` set, that it has no login; or, with `STEAMSHIP_FAKE_FAIL` set,
/// that the server refused the build.
#[cfg(unix)]
fn leaking() -> tempfile::TempDir {
    let home = faked_with(
        "#!/bin/sh\n\
         for arg in \"$@\"; do [ \"$previous\" = +run_app_build ] && script=\"$arg\"; previous=\"$arg\"; done\n\
         output=$(sed -n 's/^[[:space:]]*\"BuildOutput\"[[:space:]]*\"\\(.*\\)\"$/\\1/p' \"$script\")\n\
         cat \"$HOME/Steam/config/config.vdf\" \"$HOME\"/Steam/userdata/*/config/localconfig.vdf\n\
         if [ -n \"$STEAMSHIP_FAKE_LOGGED_OUT\" ]; then\n\
           echo 'Cached credentials not found.'\n\
           echo 'FAILED (No cached credentials and @NoPromptForPassword is set)'\n\
           exit 5\n\
         fi\n\
         if [ -n \"$STEAMSHIP_FAKE_FAIL\" ]; then\n\
           echo '[..]: ERROR! Failed to initialize build on server (Access Denied)' \
             > \"$output/app_build_1000.log\"\n\
           exit 6\n\
         fi\n\
         { cat \"$HOME/Steam/config/config.vdf\"; \
           echo 'Successfully finished AppID 1000 build (BuildID 4242).'; } \
           > \"$output/app_build_1000.log\"\n",
    );
    let [token, local] = SECRETS;
    let put = |relative: &str, contents: String| {
        let path = home.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    put(
        "Steam/config/config.vdf",
        format!(
            "\"InstallConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \
             \"ConnectCache\" {{ \"3a1f0c2e\" \"{token}\" }} }} }} }} }}\n"
        ),
    );
    put(
        "Steam/userdata/12345/config/localconfig.vdf",
        format!("\"UserLocalConfigStore\" {{ \"WebStorage\" {{ \"cookie\" \"{local}\" }} }}\n"),
    );
    home
}

/// Every file under `folder` but those in `except`, relative to `folder`, with its contents.
#[cfg(unix)]
fn written(folder: &Path, except: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(folder)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !except.contains(&relative.as_str()) {
                found.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    found
}

#[cfg(unix)]
fn assert_no_secret_in(what: &str, contents: &[u8]) {
    // A failure names the secret by its place in SECRETS: printing it would be the leak itself.
    for (index, secret) in SECRETS.iter().enumerate() {
        assert!(
            !contents
                .windows(secret.len())
                .any(|window| window == secret.as_bytes()),
            "{what} holds secret {index}"
        );
    }
}

#[cfg(unix)]
#[test]
fn nothing_upload_prints_or_writes_holds_the_login() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  app       \u{2713} 1000, 1 depot, 1 file, checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\u{2713} app 1000: BuildID 4242, set live on testing\n"),
        "{stdout}"
    );
    assert_no_secret_in("stdout", stdout.as_bytes());
    let except = [
        "Steam/config/config.vdf",
        "Steam/userdata/12345/config/localconfig.vdf",
        "apps/1000/output/app_build_1000.log",
    ];
    let files = written(home.path(), &except);
    assert!(
        files
            .iter()
            .any(|(path, _)| path == "apps/1000/output/steamcmd.log"),
        "steamcmd's output is kept"
    );
    for (path, contents) in files {
        assert_no_secret_in(&path, &contents);
    }
}

#[cfg(unix)]
#[test]
fn an_upload_in_a_github_actions_step_hands_the_build_id_on_as_its_output() {
    let home = leaking();
    let (project, script) = project(true);
    let outputs = project.path().join("github_output");
    fs::write(&outputs, "earlier=kept\n").unwrap();
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("GITHUB_OUTPUT", outputs.to_str().unwrap())],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert_eq!(
        fs::read_to_string(&outputs).unwrap(),
        "earlier=kept\nbuild-id=4242\n"
    );
    let missing = project.path().join("no such folder").join("output");
    let (still, _, said) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("GITHUB_OUTPUT", missing.to_str().unwrap())],
    );
    assert_eq!(still, Some(0_i32), "the upload is done: {said}");
    assert!(
        said.contains("the BuildID could not be handed to the workflow"),
        "{said}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_without_a_login_says_to_log_in_and_exits_3() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_LOGGED_OUT", "1")],
    );
    assert_eq!(code, Some(3_i32));
    assert_eq!(
        failure(&stderr),
        "not logged in: Cached credentials not found."
    );
    assert!(
        stderr.ends_with("log the build account in again with steamship login\n"),
        "{stderr}"
    );
    assert_no_secret_in("stdout", stdout.as_bytes());
    assert_no_secret_in("stderr", stderr.as_bytes());
}

#[cfg(unix)]
#[test]
fn a_preview_says_nothing_was_uploaded() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--preview"],
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(
        stdout.starts_with("steamship upload, preview\n"),
        "{stdout}"
    );
    assert!(stdout.contains("\n  build     1.4.0 "), "{stdout}");
    assert!(
        stdout.contains("\u{2713} nothing was uploaded, as asked\n"),
        "{stdout}"
    );
    let copy = fs::read_to_string(home.path().join("apps/1000/app_build.vdf")).unwrap();
    assert!(copy.contains("\t\"Preview\"\t\t\"1\"\n"), "{copy}");
}

#[cfg(unix)]
#[test]
fn a_refused_build_names_every_reason_and_where_the_logs_are_and_exits_1() {
    let home = leaking();
    let (_project, script) = project(true);
    let (code, _, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_FAKE_FAIL", "1")],
    );
    assert_eq!(code, Some(1_i32));
    let output = home.path().join("apps/1000/output");
    assert_eq!(
        stderr,
        format!(
            "  \u{2717} Failed to initialize build on server (Access Denied)\n    \
             log  {}, and steamcmd's own in {}\n",
            output.join("steamcmd.log").display(),
            output.join("app_build_1000.log").display()
        )
    );
    assert_no_secret_in("stderr", stderr.as_bytes());
}

/// Whether `condition` comes true within half a minute.
#[cfg(unix)]
fn soon<Condition>(mut condition: Condition) -> bool
where
    Condition: FnMut() -> bool,
{
    let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Sends `which` signal to the process `pid`, and says whether it was there to receive it.
#[cfg(unix)]
fn signal(which: &str, pid: &str) -> bool {
    Command::new("kill")
        .args([which, pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(unix)]
#[test]
fn ctrl_c_during_an_upload_ends_steamcmd_with_steamship() {
    use std::os::unix::process::ExitStatusExt as _;

    let home = faked_with("#!/bin/sh\necho $$ > \"$HOME/steamcmd.pid\"\nexec sleep 60\n");
    let (_project, script) = project(true);
    let mut steamship = Command::new(env!("CARGO_BIN_EXE_steamship"))
        .args(["upload", script.to_str().unwrap(), "--version", "1.4.0"])
        .args(["--account", "build_bot"])
        .env("STEAMSHIP_HOME", home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid_file = home.path().join("steamcmd.pid");
    assert!(soon(
        || fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n'))
    ));
    let steamcmd = fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    assert!(signal("-INT", &steamship.id().to_string()));
    let mut ended = None;
    assert!(soon(|| {
        ended = steamship.try_wait().unwrap();
        ended.is_some()
    }));
    assert_eq!(ended.unwrap().signal(), Some(2_i32), "ended by Ctrl+C");
    assert!(
        soon(|| !signal("-0", &steamcmd)),
        "steamcmd outlived steamship"
    );
}
