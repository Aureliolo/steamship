//! The `steamship` command itself: what it prints and the code it exits with.
//!
//! These run after the library's own tests on purpose: a change that broke the installer would
//! send `install` here to Valve's CDN, and the unit tests that fail at once on such a change are
//! the ones that should say so.
#![expect(
    clippy::unwrap_used,
    reason = "a test reports failure by panicking, its helpers included"
)]

pub mod common;

use common::pe::{settings_ticking, windows_library};

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read as _, Write};
use std::iter;
use std::net::TcpListener;
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use steamship::account::Account;
use steamship::install;
use steamship::itself;
use steamship::keychain;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::run;
use steamship::scripts::Problem;
use steamship::terminal::{Event, Reader};
#[cfg(unix)]
use steamship::unix::Terminal;
use steamship::webapi::STAND_IN;
#[cfg(windows)]
use steamship::windows::Terminal;
#[cfg(unix)]
use steamship::{ci, steamcmd};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// The `steamship` built for these tests, which on Linux finds this process's own Secret Service
/// and never the session's.
fn steamship_command() -> Command {
    let mut command = run::command(env!("CARGO_BIN_EXE_steamship"));
    let _: &mut Command = command.envs(common::store::environment());
    command
}

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
    let mut command = steamship_command();
    let _: &mut Command = command.current_dir(folder);
    // The runner's own CI variables would make every test a CI run, writing to the job's real
    // outputs; a test that wants them sets its own.
    let _: &mut Command = command
        .args(args)
        .env_remove("STEAMSHIP_ACCOUNT")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("GITHUB_OUTPUT");
    let _: &mut Command = match home {
        Some(home) => command.env("STEAMSHIP_HOME", home),
        None => command.env_clear(),
    };
    // After the environment may have been cleared, so that nothing reaches the session's store.
    let _: &mut Command = command.envs(common::store::environment());
    let output = run::output(command.envs(variables.iter().copied())).unwrap();
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
fn install_waits_for_another_steamship_using_the_home_then_goes_on() {
    let home = recorded("");
    let held = File::create(home.path().join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let path = home.path().to_path_buf();
    let second = thread::spawn(move || run(Some(&path)));
    thread::sleep(Duration::from_secs(2));
    assert!(!second.is_finished(), "it waits while the home is held");
    held.unlock().unwrap();
    let (code, stdout, stderr) = second.join().unwrap();
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(
        stdout.contains(&format!(
            "  steamcmd  waiting for another steamship using {} to finish...\n",
            home.path().display()
        )) && stdout.contains("\u{2713} free after ")
            && stdout.contains(", pinned and verified\n"),
        "{stdout}"
    );
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
         api key   none kept\n  \
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
        "  login          Log in to Steam, once, for uploads\n",
        "  status         Show the login and steamcmd, checking the login with Steam\n",
        "  check          Check the build scripts, without logging in\n",
        "  upload         Check, build and upload, then print the build ID\n",
        "  workshop       Upload a Workshop item, then print its ID\n",
        "  builds         Show an app's branches and last builds\n",
        "  achievements   Show an app's achievements and stats, check them against files, or write \
         their import\n",
        "  assets         Check artwork and icons against Valve's sizes, before they are \
         uploaded\n",
        "  drm-wrap       Wrap a Windows executable in Steam DRM, before it is uploaded\n",
        "  settings       Show an app's settings in Steamworks, or check them against a snapshot\n",
        "  leaderboards   Show an app's leaderboards, or check them against a file, making or \
         deleting the rest\n",
        "  rich-presence  Upload an app's rich presence localisation\n",
        "  text           Write a game's store, Workshop and rich presence text in every language \
         it speaks\n",
        "  promote        Set an uploaded build live on a branch\n",
        "  branch         Set the description players see for a beta branch\n",
        "  ci             Set up uploads from CI, the login kept as a secret\n",
        "  logout         Forget the saved login\n",
        "  init           Write starter build scripts for an app\n",
        "  install        Install or verify the pinned steamcmd\n",
        "  completions    Print tab completion for bash, zsh, fish, PowerShell or elvish\n",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
}

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
        &["init", "490", "--folder", "steam/demo"],
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
            "{} is already there, and is left as it is",
            Path::new("steam").join("app_build.vdf").display()
        )
    );
    assert_eq!(fs::read_to_string(&script).unwrap(), before);
}

#[test]
fn init_leaves_the_branch_to_choose_for_scripts_no_game_keeps() {
    let folder = tempfile::tempdir().unwrap();
    let home = folder.path().join("home");
    let (code, stdout, stderr) = steamship_in(
        folder.path(),
        &["init", "480", "--folder", "tools/steam"],
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
fn completions_are_printed_for_each_shell_and_name_every_command() {
    for (shell, opening) in [
        ("bash", "_steamship() {"),
        ("zsh", "#compdef steamship"),
        ("fish", "complete -c steamship"),
        (
            "powershell",
            "Register-ArgumentCompleter -Native -CommandName 'steamship'",
        ),
        ("elvish", "set edit:completion:arg-completer[steamship]"),
    ] {
        let (code, stdout, stderr) = steamship(&["completions", shell], Some(Path::new(".")), &[]);
        assert_eq!(code, Some(0_i32), "{shell}: {stderr}");
        assert!(stdout.contains(opening), "{shell}: {stdout}");
        for command in ["upload", "workshop", "promote", "completions"] {
            assert!(stdout.contains(command), "{shell} completes {command}");
        }
    }
}

/// What Explorer, Properties and Task Manager show for steamship.exe, as Windows reads it back.
#[cfg(windows)]
#[test]
fn the_windows_program_carries_its_icon_and_version_details() {
    let script = "$ErrorActionPreference = 'Stop'
        Add-Type -Namespace Shell -Name Icons -MemberDefinition '
          [DllImport(\"shell32.dll\", CharSet = CharSet.Unicode)]
          public static extern uint ExtractIconExW(string file, int index, IntPtr[] large, IntPtr[] small, uint count);'
        $v = (Get-Item -LiteralPath $env:PROGRAM).VersionInfo
        $v.CompanyName, $v.FileDescription, $v.ProductName, $v.ProductVersion,
          $v.FileVersionRaw.ToString(), $v.LegalCopyright, $v.OriginalFilename,
          [Shell.Icons]::ExtractIconExW($env:PROGRAM, -1, $null, $null, 0)";
    let output = run::output(
        run::command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("PROGRAM", env!("CARGO_BIN_EXE_steamship")),
    )
    .unwrap();
    let said = String::from_utf8(output.stdout).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        said.lines().collect::<Vec<_>>(),
        [
            "Aurelio Amoroso",
            "steamship",
            "steamship",
            version,
            &format!("{version}.0"),
            "Copyright (c) 2026 Aurelio Amoroso",
            "steamship.exe",
            "1",
        ],
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
             api key   none kept\n  \
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

/// The login `saved_login` makes, packed as `steamship ci` packs it, on 2026-09-28.
#[cfg(unix)]
fn packed_login() -> String {
    let home = tempfile::tempdir().unwrap();
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    let account = Account::parse("build_bot").unwrap();
    let login = ci::Login::saved(home.path(), Platform::THIS, account).unwrap();
    login.packed("2026-09-28").to_string()
}

/// A packed login without the day it was packed, for one `steamship ci` packed today, which a
/// test cannot know ahead.
#[cfg(unix)]
fn undated(packed: &str) -> String {
    let rest = packed.strip_prefix("steamship-login-2:").unwrap();
    rest.split_once(':').unwrap().1.to_owned()
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
#[cfg(unix)]
const KILLED: &str = "#!/bin/sh\n\
    logs=\"$HOME/Steam/logs\"\n\
    [ \"$(uname)\" = Darwin ] && logs=\"$HOME/Library/Application Support/Steam/logs\"\n\
    mkdir -p \"$logs\"\n\
    echo 'this run: connecting' >> \"$logs/connection_log.txt\"\n\
    echo 'Loading Steam API...OK'\n\
    exit 137\n";

#[cfg(unix)]
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

/// A steamcmd that has Valve build the upload as `BuildID` 777, then fails setting it live: it
/// writes the app and depot logs where it was told to, adds to its own content log, and exits 6.
#[cfg(unix)]
const BUILT_THEN_FAILED: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    logs=\"$HOME/Steam/logs\"\n\
    [ \"$(uname)\" = Darwin ] && logs=\"$HOME/Library/Application Support/Steam/logs\"\n\
    mkdir -p \"$output\" \"$logs\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 777).' > \"$output/app_build_1000.log\"\n\
    echo 'depot 1001: 1 file' > \"$output/depot_build_1001.log\"\n\
    echo 'this run: chunks sent' >> \"$logs/content_log.txt\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    echo '::set-output name=forged::yes'\n\
    echo 'ERROR! Failed to set build live on branch testing'\n\
    exit 6\n";

/// A steamcmd whose build Steam keeps but will not set live, reporting it as steamcmd 1788292693
/// did for Falcon Ridge Overseer's `SetLive` branch before the app had one: no build ID, only
/// "Failed to commit build".
#[cfg(unix)]
const COMMIT_REFUSED: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo '[..]: ERROR! Failed to commit build for AppID 1000 : Failure' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    exit 6\n";

/// A steamcmd that has Valve build the upload as `BuildID` 4242, set live as the script asks.
#[cfg(unix)]
const BUILT: &str = "#!/bin/sh\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

/// `GetAppBetas` showing `build` live on testing.
#[cfg(unix)]
fn testing_live(build: u64) -> &'static str {
    Box::leak(
        format!(
            r#"{{"response": {{"result": 1, "betas": {{"testing": {{"BuildID": {build}}}}}}}}}"#
        )
        .into_boxed_str(),
    )
}

#[cfg(unix)]
#[test]
fn an_upload_with_a_key_at_hand_is_confirmed_live_on_its_branch_or_fails() {
    let home = faked_with(BUILT);
    let (_project, script) = project(true);

    let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("testing: BuildID 4242, as Steam shows it"),
        "{stdout}"
    );
    assert!(
        stdout.contains("app 1000: BuildID 4242, set live on testing"),
        "{stdout}"
    );

    // Steam shows the build it had until the new one shows, as it may just after setting it.
    let (late, _late_asked) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(7)),
        ("200 OK", testing_live(4242)),
    ]);
    let (caught_up, late_out, late_err) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &late)],
    );
    assert_eq!(caught_up, Some(0_i32), "{late_out}{late_err}");
    assert!(
        late_out.contains("testing: BuildID 4242, as Steam shows it"),
        "{late_out}"
    );

    // Steam goes on showing another build, however often it is asked.
    let (other, _asked) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
        ("200 OK", BETAS),
    ]);
    let (refused, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &other)],
    );
    assert_eq!(refused, Some(1_i32), "{said}{why}");
    assert!(
        said.contains("testing: BuildID 7, as Steam shows it"),
        "{said}"
    );
    for line in [
        "app 1000: BuildID 4242 uploaded, but Steam does not show it live on testing",
        "set it live with steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(why.contains(line), "{line:?} in {why}");
    }
}

/// A steamcmd that, asked for app 1000's info, prints it as [`app_info`] does with the
/// `ReleaseState` `state`, and otherwise has Valve build the upload as `BuildID` 4242, as [`BUILT`]
/// does, adding a line to `builds` in the home.
#[cfg(unix)]
fn released_as(state: &str) -> String {
    format!(
        "#!/bin/sh\n\
         case \" $* \" in *' +app_info_print '*)\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
         printf '\"1000\"\\n{{\\n\\t\"common\"\\n\\t{{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"ReleaseState\"\\t\\t\"{state}\"\\n\\t}}\\n}}\\n'\n\
         echo 'Unloading Steam API...OK'\n\
         exit 0;;\n\
         esac\n\
         echo built >> \"$HOME/builds\"\n\
         output=\"$HOME/apps/1000/output\"\n\
         mkdir -p \"$output\"\n\
         echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n"
    )
}

#[cfg(unix)]
#[test]
fn an_upload_asked_for_default_too_is_set_live_there_while_steam_shows_the_app_unreleased() {
    let home = faked_with(&released_as("unavailable"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"default\":true")
            && stdout.contains("\"live\":true")
            && stdout.contains("\"outcome\":\"uploaded\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("unreleased, as Steam shows it (unavailable)")
            && stderr.contains("  default   \u{2713} set live\n")
            && stderr.contains("app 1000: BuildID 4242, set live on testing and default"),
        "{stderr}"
    );
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000&buildid=4242&betakey=public"),
        "{setting}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_asked_for_default_too_once_steam_shows_the_app_released_goes_to_its_branch_alone() {
    let home = faked_with(&released_as("released"));
    saved_login(home.path());
    let (_project, script) = project(true);
    // Only what the upload asks of Steam either way: a third request would fail the run.
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"default\":false")
            && stdout.contains("\"live\":true")
            && stdout.contains("\"outcome\":\"uploaded\""),
        "{stdout}"
    );
    assert!(
        stderr.contains("released, as Steam shows it (released)")
            && stderr.contains(
                "app 1000 is released, so its default branch is set live in Steamworks, where \
                 Valve asks for a Steam Mobile confirmation; the build goes up without it"
            )
            && stderr.contains("app 1000: BuildID 4242, set live on testing")
            && !stderr.contains("and default"),
        "{stderr}"
    );
    assert!(home.path().join("builds").exists(), "the build went up");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(2)
            .collect();
    assert!(
        asked
            .iter()
            .all(|request| !request.contains("SetAppBuildLive")),
        "{asked:?}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_steam_did_not_set_live_on_default_says_how_to_and_exits_1() {
    let home = faked_with(&released_as("prerelease"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", r#"{"response": {"result": 8}}"#),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"default\":false")
            && stdout.contains("\"live\":true")
            && stdout.contains("\"outcome\":\"not-live\""),
        "{stdout}"
    );
    for line in [
        "app 1000: BuildID 4242 uploaded and live on testing, but Steam did not set it live on \
         default",
        "set it live with steamship promote 1000 --build 4242 --branch default",
    ] {
        assert!(stderr.contains(line), "{line:?} in {stderr}");
    }
}

/// `GetAppList` for a key that holds app 1000.
#[cfg(unix)]
const HOLDS_1000: &str = r#"{"applist": {"apps": {"app": [
    {"appid": 1000, "app_type": "game", "app_name": "Spacewar"}
]}}}"#;

#[cfg(unix)]
#[test]
fn an_upload_to_an_app_with_no_branch_yet_goes_live_on_default_alone_and_says_how_to_go_on() {
    let home = faked_with(&released_as("unavailable"));
    saved_login(home.path());
    let (_project, script) = project(true);
    let (host, requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", HOLDS_1000),
        ("200 OK", SET_LIVE),
    ]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--default", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"branch\":\"testing\"")
            && stdout.contains("\"default\":true")
            && stdout.contains("\"live\":false")
            && stdout.contains("\"outcome\":\"not-live\""),
        "{stdout}"
    );
    for line in [
        "  branch    testing, not on Steam yet\n",
        "app 1000 has no build live on its default branch, and Steam gives it no other branch \
         until it has",
        "app 1000: BuildID 4242, set live on default",
        "not set live on testing: Steam gives app 1000 no branch but default until a build is \
         live there",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(stderr.contains(line), "{line:?} in {stderr}");
    }
    let given = fs::read_to_string(home.path().join("apps/1000/app_build.vdf")).unwrap();
    assert!(!given.contains("SetLive"), "{given}");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000&buildid=4242&betakey=public"),
        "{setting}"
    );

    // Without --default the build is live nowhere, which the upload fails on.
    let (alone, _asked) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", HOLDS_1000),
    ]);
    let (nowhere, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &alone)],
    );
    assert_eq!(nowhere, Some(1_i32), "{said}{why}");
    for line in [
        "app 1000: BuildID 4242 uploaded, but set live nowhere",
        "app 1000 has no branch but default until a build is live there, so steamcmd was not \
         asked to set it live on testing",
        "set it live on default with steamship promote 1000 --build 4242 --branch default",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000 --build 4242 --branch testing",
    ] {
        assert!(why.contains(line), "{line:?} in {why}");
    }
}

#[test]
fn an_upload_asked_for_default_too_cannot_be_a_preview() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = project(false);
    let (code, _, stderr) = steamship(
        &[
            "upload",
            script.to_str().unwrap(),
            "--version",
            "1.4.0",
            "--preview",
            "--default",
        ],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

/// A steamcmd whose first `failing` runs run the shell commands `fail`, which fail the build, and
/// exit 6; any run after has Valve build the upload as `BuildID` 4242. Each run adds a line to
/// `runs` in the home.
#[cfg(unix)]
fn failing_then_built(fail: &str, failing: u32) -> String {
    format!(
        "#!/bin/sh\n\
         output=\"$HOME/apps/1000/output\"\n\
         mkdir -p \"$output\"\n\
         echo run >> \"$HOME/runs\"\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n\
         runs=$(wc -l < \"$HOME/runs\")\n\
         if [ $runs -le {failing} ]; then\n\
         {fail}\
         exit 6\n\
         fi\n\
         echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n"
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd 1791918946 did for Tidal Mill Inn 16
/// minutes after the branch's last build, the CDN not serving that build's manifest yet, named
/// here at `cdn`.
#[cfg(unix)]
fn unserved_then_built(cdn: &str, failing: u32) -> String {
    let manifest = format!(
        "{}/depot/1001/manifest/6076939079705630407/5/7060534466868254360",
        cdn.strip_prefix("http://").unwrap()
    );
    failing_then_built(
        &format!(
            "echo '[2026-10-15 18:05:09]: Building depot 1001...'\n\
             echo '[2026-10-15 18:06:11]: ERROR! Failed to download manifest \"{manifest}\" (HTTP 404)'\n\
             echo '[2026-10-15 18:06:11]: ERROR! Build for depot 1001 failed : Failure' > \"$output/app_build_1000.log\"\n"
        ),
        failing,
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd 1788292693 did once in CI, having
/// reached Steam but not its build server.
#[cfg(unix)]
fn unconnected_then_built(failing: u32) -> String {
    failing_then_built(
        "echo '[2026-10-12 15:35:56]: Building depot 1001...'\n\
         echo '[2026-10-12 15:35:56]: ERROR! Failed to initialize build on server (No Connection) '\n\
         echo '[2026-10-12 15:35:56]: ERROR! Build for depot 1001 failed : Failure' > \"$output/app_build_1000.log\"\n",
        failing,
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd 1791918946 did once in CI, Steam
/// dropping it before it logged in.
#[cfg(unix)]
fn disconnected_then_built(failing: u32) -> String {
    failing_then_built(
        "echo 'Waiting for user info...'\n\
         echo 'steamcmd has been disconnected from steam with result 3 (No Connection)'\n\
         echo 'ERROR! (Timed out)'\n",
        failing,
    )
}

/// A steamcmd whose first `failing` runs fail as steamcmd did on every system of one CI run
/// within the same minute, Steam sending it no client config.
#[cfg(unix)]
fn no_client_config_then_built(failing: u32) -> String {
    failing_then_built(
        "echo 'Waiting for client config...ERROR! Info request for AppId 7 returned error Timeout.'\n\
         echo 'OK'\n\
         echo 'ERROR! Failed to load platform info'\n",
        failing,
    )
}

/// How many times the steamcmd of `failing_then_built` ran in `home`.
#[cfg(unix)]
fn runs(home: &Path) -> usize {
    fs::read_to_string(home.join("runs"))
        .unwrap()
        .lines()
        .count()
}

#[cfg(unix)]
#[test]
fn an_upload_waits_for_the_cdn_to_serve_the_last_manifest_then_uploads_again() {
    let (cdn, requests) = web_api(vec![("200 OK", "manifest")]);
    let home = faked_with(&unserved_then_built(&cdn, 1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    assert!(
        requests.recv().unwrap().starts_with(
            "GET /depot/1001/manifest/6076939079705630407/5/7060534466868254360 HTTP/1.1\r\n"
        ),
        "the manifest steamcmd could not download is asked for"
    );
    for line in [
        "\u{2717} Steam's CDN does not serve depot 1001's last manifest yet\n",
        "  cdn       \u{2713} served after ",
        "  steam     \u{2713} uploaded in ",
        "app 1000: BuildID 4242, set live on testing",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
}

#[cfg(unix)]
#[test]
fn an_upload_stops_asking_steamcmd_after_four_runs_the_cdn_never_served() {
    let (cdn, _requests) = web_api(vec![("401 Refused", ""); 3]);
    let home = faked_with(&unserved_then_built(&cdn, 99));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 4);
    assert_eq!(stdout.matches("answered 401 after ").count(), 3, "{stdout}");
    for reason in [
        "Build for depot 1001 failed : Failure",
        "Steam's CDN still does not serve depot 1001's last manifest, which steamcmd builds the \
         upload against; upload again later",
    ] {
        assert!(stderr.contains(reason), "{reason:?} in {stderr}");
    }
}

#[cfg(unix)]
#[test]
fn an_upload_runs_steamcmd_once_more_when_it_did_not_reach_steams_build_server() {
    let home = faked_with(&unconnected_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} no connection to Steam's build server after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let unreached = faked_with(&unconnected_then_built(99));
    let (failed, said, why) = upload(&script, unreached.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(unreached.path()), 2, "only once more");
    assert!(
        why.contains("Failed to initialize build on server (No Connection)"),
        "{why}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_runs_steamcmd_once_more_when_steam_disconnects_it() {
    let home = faked_with(&disconnected_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} Steam disconnected steamcmd after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let dropped = faked_with(&disconnected_then_built(99));
    let (failed, said, why) = upload(&script, dropped.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(dropped.path()), 2, "only once more");
    assert!(why.contains("Timed out"), "{why}");
}

#[cfg(unix)]
#[test]
fn an_upload_runs_steamcmd_once_more_when_steam_sends_it_no_client_config() {
    let home = faked_with(&no_client_config_then_built(1));
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(runs(home.path()), 2);
    for line in [
        "  steam     \u{2717} Steam sent steamcmd no client config after ",
        "  steam     \u{2713} uploaded in ",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }

    let unsent = faked_with(&no_client_config_then_built(99));
    let (failed, said, why) = upload(&script, unsent.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(failed, Some(1_i32), "{said}{why}");
    assert_eq!(runs(unsent.path()), 2, "only once more");
    assert!(why.contains("Failed to load platform info"), "{why}");
}

/// The one line of JSON a `--json` command printed on standard output, which must be all there
/// is.
fn summary_of(stdout: &str) -> serde_json::Value {
    let mut lines = stdout.lines();
    let line = lines.next().unwrap_or_default();
    assert_eq!(lines.next(), None, "one line only: {stdout}");
    serde_json::from_str(line).unwrap()
}

#[cfg(unix)]
#[test]
fn an_upload_with_json_prints_only_what_came_of_it_on_standard_output() {
    let home = faked_with(BUILT);
    let (_project, script) = project(true);
    let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", testing_live(4242))]);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0", "--json"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let mut summary = summary_of(&stdout);
    // The description names the commit the test's repository is at, which differs each run.
    let description = summary
        .as_object_mut()
        .and_then(|fields| fields.remove("description"));
    assert!(
        description
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .is_some_and(|description| description.starts_with("1.4.0 ")),
        "{description:?}"
    );
    assert_eq!(
        summary,
        serde_json::json!({
            "outcome": "uploaded",
            "app": 1000_u32,
            "build_id": 4242_u64,
            "branch": "testing",
            "live": true,
            "default": false,
        })
    );
    assert!(
        stderr.contains("app 1000: BuildID 4242, set live on testing"),
        "what is shown for a person goes to standard error: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_with_json_draws_its_spinner_on_the_terminal_standard_error_is() {
    // Slow enough that the spinner is drawn before the upload ends.
    let home = faked_with(&format!("{BUILT}sleep 1\n"));
    let (folder, script) = project(true);
    let summary = folder.path().join("summary.json");
    let line = format!(
        "'{}' upload '{}' --version 1.4.0 --account build_bot --json > '{}'; echo \"exited $?\"",
        env!("CARGO_BIN_EXE_steamship"),
        script.display(),
        summary.display()
    );
    let mut session = Session::start(&line, home.path());
    // Drawn with how long it has been, where elsewhere it would be written once, ending "...".
    session.wait_for(" uploading 0 s");
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(!seen.contains("uploading..."), "{seen}");
    assert!(seen.contains("app 1000: BuildID 4242"), "{seen}");
    assert_eq!(session.end().wait().unwrap(), Some(0_i32));
    let written = fs::read_to_string(&summary).unwrap();
    assert!(written.starts_with("{\"app\":1000,"), "{written}");
}

#[cfg(unix)]
#[test]
fn a_failed_upload_with_json_still_says_what_steam_kept() {
    let home = faked_with(BUILT_THEN_FAILED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) =
        upload(&script, home.path(), &["--version", "1.4.0", "--json"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let summary = summary_of(&stdout);
    for (field, expected) in [
        ("outcome", serde_json::json!("built-then-failed")),
        ("build_id", serde_json::json!(777_u64)),
        ("live", serde_json::json!(false)),
    ] {
        assert_eq!(summary.get(field), Some(&expected), "{field}: {summary}");
    }
}

#[cfg(unix)]
#[test]
fn a_build_steam_kept_but_set_live_nowhere_is_said_to_be_there_and_found_with_a_key() {
    let home = faked_with(COMMIT_REFUSED);
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    for said in [
        "Failed to commit build for AppID 1000 : Failure",
        "the build is on Steam but not live: it is listed in Steamworks under SteamPipe, Builds",
        "\"SetLive\" names \"testing\": Steam sets no build live on a branch the app does not have",
        "see the app's branches with steamship builds 1000",
    ] {
        assert!(stderr.contains(said), "{said:?} in {stderr}");
    }
    let description = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  build     "))
        .unwrap()
        .to_owned();

    // With a key, the build is looked up by the description it was uploaded with.
    let builds: &'static str = Box::leak(
        format!(
            r#"{{"response": {{"builds": {{"20000001": {{"Description": "{description}"}}}}}}}}"#
        )
        .into_boxed_str(),
    );
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", builds)]);
    let (keyed, keyed_out, keyed_err) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(keyed, Some(1_i32), "{keyed_out}{keyed_err}");
    for said in [
        "app 1000: built as BuildID 20000001, then steamcmd failed",
        "\"SetLive\" names \"testing\": Steam sets no build live on a branch the app does not have",
        "set it live with steamship promote 1000 --build 20000001 --branch testing",
    ] {
        assert!(keyed_err.contains(said), "{said:?} in {keyed_err}");
    }
    assert!(!keyed_err.contains("listed in Steamworks"), "{keyed_err}");
    let _branches = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamApps/GetAppBuilds/v1/?") && asked.contains("appid=1000"),
        "{asked}"
    );
}

#[cfg(unix)]
#[test]
fn an_upload_built_then_failed_keeps_its_build_id_and_shows_steams_logs_in_actions() {
    let home = faked_with(BUILT_THEN_FAILED);
    let logs = steamcmd::state_folder(home.path(), Platform::THIS).join("logs");
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("content_log.txt"), "an older run\n").unwrap();
    let outputs = home.path().join("github_output");
    fs::write(&outputs, "").unwrap();
    let (_project, script) = project(true);
    let (code, stdout, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_OUTPUT", outputs.to_str().unwrap()),
        ],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    for said in [
        "app 1000: built as BuildID 777, then steamcmd failed",
        "Failed to set build live on branch testing",
        "set it live with steamship promote 1000 --build 777 --branch testing",
    ] {
        assert!(stderr.contains(said), "{said:?} in {stderr}");
    }
    assert_eq!(fs::read_to_string(&outputs).unwrap(), "build-id=777\n");
    assert!(stdout.contains("::add-mask::build_bot\n"), "{stdout}");
    for group in [
        "::group::steamcmd's console\n",
        "::group::app_build_1000.log\n",
        "::group::depot_build_1001.log\n",
        "::group::steamcmd's content_log.txt, from this run\n::stop-commands::",
    ] {
        assert!(stdout.contains(group), "{group:?} in {stdout}");
    }
    assert_eq!(
        stdout.matches("::group::").count(),
        4,
        "the console and app log once each, and no other log taken for a depot's: {stdout}"
    );
    assert!(stdout.contains("this run: chunks sent"), "{stdout}");
    assert!(!stdout.contains("an older run"), "{stdout}");
    let forged = stdout.find("::set-output name=forged::yes").unwrap();
    let stopped = stdout.find("::stop-commands::").unwrap();
    assert!(
        stopped < forged,
        "a command in a log is shown only with commands stopped"
    );
}

/// A steamcmd that logs in, fails to reach the build server, says where it is, and then waits as
/// one that has hung. A run that ended so would be run again, were steamship not told to stop.
#[cfg(unix)]
const HUNG: &str = "#!/bin/sh\n\
    echo ran >> \"$HOME/runs\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n\
    echo '[2026-10-12 15:35:56]: ERROR! Failed to initialize build on server (No Connection) '\n\
    echo $$ > \"$HOME/steamcmd.pid\"\n\
    exec sleep 120\n";

#[cfg(unix)]
#[test]
fn an_upload_told_to_stop_stops_steamcmd_shows_what_it_said_and_ends_by_that_signal() {
    use std::os::unix::process::ExitStatusExt as _;

    for (name, number) in [("INT", 2_i32), ("HUP", 1_i32), ("TERM", 15_i32)] {
        let home = faked_with(HUNG);
        let outputs = home.path().join("github_output");
        fs::write(&outputs, "").unwrap();
        let (_project, script) = project(true);
        let mut command = steamship_command();
        let _: &mut Command = command
            .args(["upload", script.to_str().unwrap(), "--account", "build_bot"])
            .args(["--version", "1.4.0"])
            .env_remove("CI")
            .env("STEAMSHIP_HOME", home.path())
            .env("STEAMSHIP_LOGIN", packed_login())
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_OUTPUT", &outputs)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = run::spawn(&mut command).unwrap();
        let pid_file = home.path().join("steamcmd.pid");
        let waiting = Instant::now();
        let steamcmd = loop {
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .filter(|pid| pid.ends_with('\n'))
            {
                break pid.trim().to_owned();
            }
            assert!(
                waiting.elapsed() < Duration::from_secs(60),
                "steamcmd never started"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let pid = started.id().to_string();
        let signalled = run::output(run::command("kill").args(["-s", name, &pid])).unwrap();
        assert!(signalled.status.success(), "{signalled:?}");
        let (ending, endings) = mpsc::channel();
        let waited = thread::spawn(move || ending.send(started.wait_with_output().unwrap()));
        let Ok(ended) = endings.recv_timeout(Duration::from_secs(30)) else {
            drop(run::output(run::command("kill").args(["-s", "KILL", &pid])));
            panic!("{name}: steamship did not end");
        };
        waited.join().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(home.path().join("runs")).unwrap(),
            "ran\n",
            "{name}: steamcmd ran again after steamship was told to stop"
        );
        let stdout = String::from_utf8_lossy(&ended.stdout);
        let stderr = String::from_utf8_lossy(&ended.stderr);
        assert_eq!(
            ended.status.signal(),
            Some(number),
            "{name}: {stdout}{stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "steamship was told to stop by SIG{name}, and stopped steamcmd"
            )) && stderr.contains("Failed to initialize build on server (No Connection)"),
            "{name}: {stderr}"
        );
        assert!(
            stdout.contains("::group::steamcmd's console\n")
                && stdout.contains("Logging in user build_bot to Steam Public...OK"),
            "{name}: {stdout}"
        );
        let gone = run::output(run::command("kill").args(["-0", &steamcmd])).unwrap();
        assert!(!gone.status.success(), "{name}: steamcmd still runs");
    }
}

#[cfg(unix)]
#[test]
fn a_failed_upload_outside_actions_shows_no_log_groups() {
    let home = faked_with(BUILT_THEN_FAILED);
    let (_project, script) = project(true);
    let (code, stdout, _) = upload(&script, home.path(), &["--version", "1.4.0"], &[]);
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(!stdout.contains("::group::"), "{stdout}");
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
        "STEAMSHIP_LOGIN does not hold a login packed by `steamship ci`"
    );
    assert!(!format!("{stdout}{stderr}").contains("secret_name"));
}

#[cfg(unix)]
#[test]
fn a_login_handed_over_that_is_not_text_is_refused() {
    use std::os::unix::ffi::OsStrExt as _;

    let home = tempfile::tempdir().unwrap();
    let output = run::output(
        steamship_command()
            .arg("status")
            .env("STEAMSHIP_HOME", home.path())
            .env(
                "STEAMSHIP_LOGIN",
                OsStr::from_bytes(b"steamship-login-2:\xff"),
            ),
    )
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
fn ci_checks_the_scripts_alone_before_the_build_exists_but_needs_a_github_origin_and_exits_2() {
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    fs::remove_dir_all(root.join("content")).unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_in(&root, &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let found = Path::new("steam").join("app_build.vdf");
    assert!(
        stdout.contains(&format!("  script    {}, found\n", found.display())),
        "{stdout}"
    );
    assert!(
        stdout.contains("  app       \u{2713} 1000, 1 depot, scripts checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  content   not checked; each upload checks it\n"),
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
    let root = project.path().join("gate (x86) 1a2b");
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
    let root = project.path().join("gate (x86) 1a2b");
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
    assert_eq!(undated(&written), undated(&packed_login()));
    assert!(!stdout.contains(&written), "the login is never shown");
    let unpacked = ci::Login::unpack(&written).unwrap();
    assert_eq!(unpacked.account().name(), "build_bot");
    assert!(unpacked.packed_on().is_some());
}

#[cfg(unix)]
#[test]
fn ci_with_no_saved_login_says_to_log_in_first_and_exits_3() {
    let home = answering("", 0);
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
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
    let root = project.path().join("gate (x86) 1a2b");
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
/// `STEAMSHIP_FAKE_GH_REFUSE` is set; `gh api` answers a commit, `STEAMSHIP_FAKE_GH_COMMIT` if
/// set, or fails when `STEAMSHIP_FAKE_GH_OFFLINE` is set.
#[cfg(unix)]
fn fake_gh() -> (tempfile::TempDir, String) {
    use std::env;

    let bin = tempfile::tempdir().unwrap();
    let gh = bin.path().join("gh");
    write_program(
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
         echo \"${STEAMSHIP_FAKE_GH_COMMIT:-0123456789abcdef0123456789abcdef01234567}\" ;;\n\
         esac\n",
    );
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
    assert!(!stdout.contains(&secret), "the login is never shown");
    assert_eq!(undated(&secret), undated(&packed_login()));
    assert!(ci::Login::unpack(&secret).unwrap().packed_on().is_some());
}

#[cfg(unix)]
#[test]
fn ci_refuses_a_login_too_large_for_a_github_secret_before_asking_gh() {
    let home = answering(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, format!("\"token\" \"{}\"", "t".repeat(30_000))).unwrap();
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) =
        steamship_in(&root, &["ci"], Some(home.path()), &[("PATH", &path)]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "the packed login is too large for a GitHub secret: 59 KB, where a secret holds 48 KB"
    );
    assert!(
        stderr
            .ends_with("write it to a file for another CI with steamship ci --output login.txt\n"),
        "{stderr}"
    );
    assert!(!gh.path().join("args").exists(), "gh is never asked");
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
    let git = run::output(run::command("sh").args(["-c", "command -v git"])).unwrap();
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

#[cfg(unix)]
#[test]
fn ci_with_a_gh_that_cannot_start_says_so_and_exits_1() {
    use std::os::unix::fs::symlink;

    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let bin = tempfile::tempdir().unwrap();
    let git = run::output(run::command("sh").args(["-c", "command -v git"])).unwrap();
    symlink(
        String::from_utf8(git.stdout).unwrap().trim(),
        bin.path().join("git"),
    )
    .unwrap();
    fs::write(bin.path().join("gh"), "not a program").unwrap();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", bin.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} gh could not start\n"),
        "{stdout}"
    );
    assert!(failure(&stderr).starts_with("gh: "), "{stderr}");
}

#[cfg(unix)]
#[test]
fn ci_pins_the_tag_when_github_answers_something_that_is_not_a_commit() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (_gh, path) = fake_gh();
    let version = env!("CARGO_PKG_VERSION");
    for answer in ["0123abc", &"g".repeat(40)] {
        let (code, stdout, _) = steamship_in(
            &root,
            &["ci"],
            Some(home.path()),
            &[("PATH", &path), ("STEAMSHIP_FAKE_GH_COMMIT", answer)],
        );
        assert_eq!(code, Some(0_i32), "{stdout}");
        assert!(
            stdout.contains(&format!(
                "uses: Aureliolo/steamship@v{version} # v{version}\n"
            )),
            "{answer}: {stdout}"
        );
    }
}

#[test]
fn ci_run_outside_the_repository_reads_origin_where_the_script_is() {
    let (_project, root) = github_project();
    let outside = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let script = root.join("steam").join("app_build.vdf");
    let (code, stdout, stderr) = steamship_in(
        outside.path(),
        &["ci", "--script", script.to_str().unwrap()],
        Some(home.path()),
        &[("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  repo      Aureliolo/some-game, from origin\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "name the build account with --account or STEAMSHIP_ACCOUNT"
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
        seen.contains("\u{2713} 1000, 1 depot, scripts checked"),
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
        "not logged in: Expired Login Auth Code; the login in STEAMSHIP_LOGIN was packed on \
         2026-09-28"
    );
    assert!(
        stderr.ends_with(
            "log in again with steamship login, then run steamship ci to pack the new login\n"
        ),
        "{stderr}"
    );
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
    use steamship::digest;

    let hash = digest::hex(&digest::sha256(&mut script.as_bytes()).unwrap());
    let home = recorded(&format!("file {hash} steamcmd.sh\n"));
    let program = home.path().join(install::FOLDER).join("steamcmd.sh");
    write_program(&program, script);
    home
}

/// Writes `program` as an executable from a process of its own. Written here, its handle would
/// be open in this process while another test's thread started a program, which holds a copy of
/// every handle until it runs; running `program` in those moments fails with "Text file busy".
#[cfg(unix)]
fn write_program(program: &Path, script: &str) {
    let mut writer = run::spawn(
        run::command("sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(program)
            .stdin(Stdio::piped()),
    )
    .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(writer.wait().unwrap().success(), "{}", program.display());
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
    let mut child = run::spawn(
        steamship_command()
            .args(["login", "--account", "build_bot"])
            .env_remove("STEAMSHIP_ACCOUNT")
            .env("STEAMSHIP_HOME", home)
            .envs(variables.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
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
        Self::start_with(line, home, &[("STEAMSHIP_NO_UPDATE_CHECK", "1")])
    }

    /// [`Session::start`], with `variables` set instead.
    fn start_with(line: &str, home: &Path, variables: &[(&str, &str)]) -> Self {
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
        let environment: Vec<(OsString, OsString)> =
            iter::once(("STEAMSHIP_HOME", home.as_os_str()))
                .chain(
                    variables
                        .iter()
                        .chain(&common::store::environment())
                        .map(|&(name, value)| (name, OsStr::new(value))),
                )
                .map(|(name, value)| (OsString::from(name), value.to_owned()))
                .collect();
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
        self.wait_until(text, |seen| seen.contains(text));
    }

    /// Reads on until `text` has been shown once more than it has so far.
    #[cfg(unix)]
    fn wait_for_another(&mut self, text: &str) {
        let shown = self.seen().matches(text).count();
        self.wait_until(text, |seen| seen.matches(text).count() > shown);
    }

    /// Reads on until what has been shown is `done`, or fails the test after half a minute saying
    /// that `text` never was.
    fn wait_until<Done>(&mut self, text: &str, done: Done)
    where
        Done: Fn(&str) -> bool,
    {
        let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
        while !done(&self.seen()) {
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
    let mut session = Session::start_with(&line, home.path(), &[("CI", "")]);
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
    // Then the Web API key is offered, and Enter alone skips it.
    session.wait_for("Enter skips it");
    session.type_in("\r");
    session.wait_for("exited 0");
    let seen = session.seen();
    assert!(
        seen.contains("  api key   skipped; `steamship login --web-api-key` keeps one\n"),
        "{seen}"
    );
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
    let status = run::spawn(
        run::command("git")
            .args([
                "-c",
                "user.name=steamship",
                "-c",
                "user.email=steamship@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(folder),
    )
    .and_then(|mut git| git.wait())
    .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A game's scripts and content, committed or not, in a folder named the way a gate worktree is.
fn project(committed: bool) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("gate (x86) 1a2b");
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
fn upload_says_git_is_missing_rather_than_blaming_the_folder_and_exits_2() {
    let (_project, script) = project(true);
    let home = tempfile::tempdir().unwrap();
    let nowhere = tempfile::tempdir().unwrap();
    let (code, _, stderr) = upload(
        &script,
        home.path(),
        &["--version", "1.0"],
        &[("PATH", nowhere.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(2_i32));
    assert_eq!(
        failure(&stderr).trim_end(),
        "Git is not installed, or not on the PATH, and the build description names the commit \
         the scripts are at"
    );
}

#[test]
fn upload_runs_check_first_and_exits_2_on_a_refusal() {
    let (project, script) = project(true);
    fs::write(
        project
            .path()
            .join("gate (x86) 1a2b/content/steam_appid.txt"),
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
         finished='build (BuildID 4242).'\n\
         grep -q '\"Preview\"[[:space:]]*\"1\"' \"$script\" && finished='build preview.'\n\
         { cat \"$HOME/Steam/config/config.vdf\"; \
           echo \"Successfully finished AppID 1000 $finished\"; } \
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

/// A Workshop item's script, content and preview, in a folder named the way a gate worktree is;
/// `extra` goes into the script as it is.
fn workshop_item(extra: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("my game (x86)").join("workshop");
    fs::create_dir_all(folder.join("hat")).unwrap();
    fs::write(folder.join("hat").join("hat.mdl"), "model").unwrap();
    fs::write(folder.join("hat.png"), "png").unwrap();
    let script = folder.join("item.vdf");
    fs::write(
        &script,
        format!(
            r#""workshopitem" {{ "appid" "480" "contentfolder" "hat" "previewfile" "hat.png" "title" "A green hat" {extra} }}"#
        ),
    )
    .unwrap();
    (temp, script)
}

/// A home whose steamcmd, given a Workshop item's script, says `said`, exits `code`, and gives
/// the item ID 777 in that script when it had none, as steamcmd does with an item it makes.
#[cfg(unix)]
fn publishing(said: &str, code: u8) -> tempfile::TempDir {
    let home = faked_with(&format!(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
         grep -q publishedfileid \"$script\" || \
         printf '\"workshopitem\"\\n{{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}}\\n' > \"$script\"\n\
         printf '%s\\r\\n' '{said}'\n\
         exit {code}\n"
    ));
    saved_login(home.path());
    home
}

#[cfg(unix)]
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
        stdout.contains("  item      \u{2713} app 480, a new item, 1 file, checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  \u{2713} app 480: Workshop item 777\n    a new item: add \"publishedfileid\" \"777\" \
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

#[cfg(unix)]
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
                 Workshop item; add the ID of the item to update, or pass --new to make one",
                script.display()
            )
        );
    }
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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
    assert_eq!(failure(&stderr), "not logged in: Expired Login Auth Code");
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    let log = home.path().join("workshop/480/example/steamcmd.log");
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
                "  \u{2713} app 480: Workshop item 777\n    a new item, kept in {} as example: \
                 commit it so that later uploads update it\n",
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

    // Another item uploaded after it keeps its console beside its own copy, leaving the first's.
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
            .contains("; add \"example\": 777 to its \"items\", or the next upload makes another"),
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
            "refused: {} has no item example, so every CI run would make a new Workshop item; \
             make it outside CI and commit the file, or pass --new to make one",
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

    for (args, why) in [
        (
            vec!["workshop", script.to_str().unwrap(), "--app", "480"],
            "cannot be used with",
        ),
        (
            vec!["workshop", "--app", "480", "--name", "example"],
            "the following required arguments were not provided",
        ),
        (vec!["workshop"], "there is no game in "),
    ] {
        let (code, _, stderr) = steamship(&args, Some(home.path()), &[]);
        assert_eq!(code, Some(2_i32), "{args:?}: {stderr}");
        assert!(stderr.contains(why), "{args:?}: {stderr}");
    }
}

#[test]
fn builds_with_no_key_set_or_kept_and_no_one_to_ask_says_how_to_keep_one_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(stdout.contains("  app       1000010\n"), "{stdout}");
    assert_eq!(failure(&stderr), "no Web API key");
    assert!(
        stderr.ends_with(
            "run steamship login --web-api-key at a terminal to keep one, or set \
             STEAMSHIP_WEB_API_KEY\n"
        ),
        "{stderr}"
    );
}

#[test]
fn login_with_web_api_key_refuses_what_is_not_a_key_before_asking_steam() {
    let home = tempfile::tempdir().unwrap();
    let mut command = steamship_command();
    let _: &mut Command = command
        .args(["login", "--web-api-key"])
        .env("STEAMSHIP_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = run::spawn(&mut command).unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not_a_key_1234\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2_i32), "{stdout}{stderr}");
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

const KEY: &str = "0123456789abcdef0123456789abcdef";

const APPS: &str = r#"{"applist": {"apps": {"app": [
    {"appid": 1000010, "app_type": "game", "app_name": "Falcon Ridge Overseer"},
    {"appid": 1000030, "app_type": "game", "app_name": "Nocturne"}
]}}}"#;

/// `GetAppBetas` and `GetAppBuilds` answering with one branch and one build.
const BETAS: &str = r#"{"response": {"result": 1, "betas": {"testing": {"BuildID": 7}}}}"#;
const BUILDS: &str = r#"{"response": {"builds": {"7": {"Description": "0.1.0"}}}}"#;

/// `SetAppBuildLive` answering that the build is live.
const SET_LIVE: &str = r#"{"response": {"result": 1}}"#;

/// A stand-in for Steam's partner Web API on this machine, which a debug build of steamship is
/// sent to through `STEAMSHIP_WEB_API_STAND_IN`. It answers one request with each status and body
/// in `answers`, in turn, and hands over each request it read. A status can carry header lines
/// after it, as Steam's service methods give their result in one.
fn web_api<Body>(answers: Vec<(&'static str, Body)>) -> (String, mpsc::Receiver<String>)
where
    Body: AsRef<[u8]> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let _serving = thread::spawn(move || {
        for (status, body) in answers {
            let (mut stream, _) = listener.accept().unwrap();
            // The head first, then as much body as it says there is, which can come later.
            let mut received = Vec::new();
            let mut chunk = [0_u8; 8192];
            let request = loop {
                let read = stream.read(&mut chunk).unwrap();
                received.extend_from_slice(chunk.get(..read).unwrap());
                let text = String::from_utf8_lossy(&received).into_owned();
                let whole = text.split_once("\r\n\r\n").is_some_and(|(head, sent)| {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    sent.len() >= length
                });
                if whole || read == 0 {
                    break text;
                }
            };
            let body = body.as_ref();
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            drop(sender.send(request));
        }
    });
    (format!("http://{address}"), requests)
}

/// A home whose key `steamship logout` forgets once the test is done with it, since on macOS only
/// the program that kept a key may remove it.
fn forgetting_home() -> common::store::Home {
    common::store::Home::new(logout)
}

fn logout(home: &Path) {
    drop(steamship(&["logout"], Some(home), &[]));
}

/// `steamship login --web-api-key` with the Web API at `host`, `typed` piped to its prompt, and
/// `variables` set besides.
fn login_with_key(
    home: &Path,
    host: &str,
    typed: &str,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut child = run::spawn(
        steamship_command()
            .args(["login", "--web-api-key"])
            .env("STEAMSHIP_HOME", home)
            .env(STAND_IN, host)
            .envs(variables.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{typed}\n").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn login_with_web_api_key_keeps_a_key_steam_takes_and_logout_forgets_it() {
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
fn login_with_web_api_key_keeps_nothing_steam_refuses() {
    let home = forgetting_home();
    let (host, _requests) = web_api(vec![("403 Forbidden", "")]);
    let (code, stdout, stderr) = login_with_key(home.path(), &host, KEY, &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("Steam refused the Web API key; "),
        "{stderr}"
    );
    let (_, status, _) = steamship(&["status"], Some(home.path()), &[]);
    assert!(status.contains("  api key   none kept\n"), "{status}");
}

#[test]
fn builds_at_a_terminal_asks_for_the_key_then_offers_to_keep_it() {
    let steamship_program = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship_program} builds 1000010");
    #[cfg(unix)]
    let line = format!("'{steamship_program}' builds 1000010");
    for (answer, shown) in [("\r", "\u{2713} kept in"), ("n\r", "not kept")] {
        let home = forgetting_home();
        let (host, _requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
        let mut session = Session::start_with(
            &line,
            home.path(),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1"), (STAND_IN, &host)],
        );
        session.wait_for("api key");
        session.type_in(&format!("{KEY}\r"));
        session.wait_for("keep it");
        session.type_in(answer);
        session.wait_for(&format!("  api key   {shown}"));
        let _: Option<i32> = session.end().wait().unwrap();
        let seen = session.seen();
        assert!(seen.contains("1 branch and 1 build"), "{seen}");
        assert!(!seen.contains(KEY), "the key was shown: {seen}");
        let (_, status, _) = steamship(&["status"], Some(home.path()), &[]);
        let kept = status.contains(&format!("  api key   kept in {}\n", keychain::STORE));
        assert_eq!(kept, answer == "\r", "{answer:?}: {status}\nshown: {seen}");
    }
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

#[test]
fn builds_and_promote_use_the_key_set_and_say_what_steam_answered() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
        ("200 OK", SET_LIVE),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let (built, listed, said) = steamship(&["builds", "1000010"], Some(home.path()), &set);
    assert_eq!((built, said.as_str()), (Some(0_i32), ""), "{listed}");
    assert!(
        listed.contains("  api key   from STEAMSHIP_WEB_API_KEY\n"),
        "{listed}"
    );
    assert!(listed.contains("1 branch and 1 build"), "{listed}");
    assert!(
        !listed.contains("keep it"),
        "a key set is not offered for keeping: {listed}"
    );
    let (promoted, live, stderr) = steamship(
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
        Some(home.path()),
        &set,
    );
    assert_eq!((promoted, stderr.as_str()), (Some(0_i32), ""), "{live}");
    assert!(live.contains("  steam     \u{2713} set live\n"), "{live}");
    assert!(
        live.contains("app 1000010: BuildID 7 live on testing"),
        "{live}"
    );
    assert!(!format!("{listed}{live}").contains(KEY), "never shown");
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    let setting = asked.last().unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/SetAppBuildLive/v2/ ")
            && setting.ends_with("appid=1000010&buildid=7&betakey=testing"),
        "{setting}"
    );
}

/// `GetAppBetas` and `GetAppBuilds` as Steam answered them for Falcon Ridge Overseer, the builds
/// trimmed to two and their depots left out.
const SAMPLE_BETAS: &str = r#"{"response":{"betas":{"public":{"BuildID":20000001,"Description":"Public default branch","ReqPassword":0,"ReqLocalCS":0},"testing":{"BuildID":20000003,"Description":"0.1.0 (a1b2c3d)","ReqPassword":1,"ReqLocalCS":0}},"result":1,"message":""}}"#;
const SAMPLE_BUILDS: &str = r#"{"response":{"builds":{"20000003":{"BuildID":20000003,"CreationTime":1791910891,"Description":"0.1.0 a1b2c3d4e5f6","AccountIDCreator":123456789},"20000002":{"BuildID":20000002,"CreationTime":1791906331,"Description":"0.1.0 f6e5d4c3b2a1","AccountIDCreator":123456789}},"result":1,"message":""}}"#;

#[test]
fn builds_json_prints_the_branches_and_builds_alone_on_standard_output() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", SAMPLE_BETAS), ("200 OK", SAMPLE_BUILDS)]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010", "--count", "2", "--json"],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        summary_of(&stdout),
        serde_json::json!({
            "app": 1_000_010_u32,
            "branches": [
                {"name": "default", "build_id": 20_000_001_u64,
                 "description": "Public default branch", "password": false},
                {"name": "testing", "build_id": 20_000_003_u64,
                 "description": "0.1.0 (a1b2c3d)", "password": true},
            ],
            "builds": [
                {"build_id": 20_000_003_u64, "created": "2026-10-13T17:01:31Z",
                 "description": "0.1.0 a1b2c3d4e5f6", "version": "0.1.0",
                 "commit": "a1b2c3d4e5f6", "live_on": ["testing"]},
                {"build_id": 20_000_002_u64, "created": "2026-10-13T15:45:31Z",
                 "description": "0.1.0 f6e5d4c3b2a1", "version": "0.1.0",
                 "commit": "f6e5d4c3b2a1", "live_on": []},
            ],
        })
    );
    assert!(stderr.contains("2 branches and 2 builds"), "{stderr}");
    assert!(!stderr.contains(KEY), "never shown");
}

#[test]
fn builds_json_names_an_app_with_no_build_live_apart_from_steam_failing() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    for (app, code) in [("1000030", "no-build-live"), ("480", "http-error")] {
        let (exit, stdout, stderr) = steamship(&["builds", app, "--json"], Some(home.path()), &set);
        assert_eq!(exit, Some(1_i32), "{stdout}{stderr}");
        let said = summary_of(&stdout);
        let app_id: u32 = app.parse().unwrap();
        assert_eq!(said.get("app"), Some(&app_id.into()), "{said}");
        assert_eq!(said.pointer("/error/code"), Some(&code.into()), "{said}");
        let message = said
            .pointer("/error/message")
            .and_then(|text| text.as_str());
        assert!(
            message.is_some_and(|text| text.starts_with("Steam answered HTTP 500")),
            "{said}"
        );
    }
}

#[test]
fn builds_json_with_no_key_asks_for_none_and_prints_no_json() {
    let _store = common::store::lock();
    let home = tempfile::tempdir().unwrap();
    let steamship_program = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship_program} builds 1000010 --json");
    #[cfg(unix)]
    let line = format!("'{steamship_program}' builds 1000010 --json");
    let mut session = Session::start(&line, home.path());
    session.wait_for("no Web API key");
    assert_eq!(session.end().wait().unwrap(), Some(2_i32));
    let seen = session.seen();
    assert!(!seen.contains('{'), "{seen}");
}

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
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

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
            "app_build.vdf: \"SetLive\" names \"beta2\", but app 480 has no branch by that name; \
             create it in Steamworks under SteamPipe, Builds first"
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
fn check_goes_on_when_steam_cannot_say_or_no_key_is_at_hand() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("500 Internal Server Error", "")]);
    let (_folder, script) = setting_live("beta2");
    let script = script.to_string_lossy().into_owned();
    let (unanswered, said, _) = steamship(
        &["check", &script],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
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
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &holding),
        ],
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
             until it has: an upload with --default sets the build live on default alone"
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
            "leaves the build uploaded but not live; steamship login --web-api-key keeps"
        ),
        "{warned}"
    );
}

/// [`project`], committed with `content/game.dll` importing `dlls`, and `snapshot` as the
/// settings snapshot beside the script when given.
fn shipping_dll(dlls: &[&str], snapshot: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let (temp, script) = project(false);
    let root = script.parent().unwrap().parent().unwrap().to_path_buf();
    fs::write(root.join("content/game.dll"), windows_library(dlls)).unwrap();
    if let Some(snapshot) = snapshot {
        fs::write(script.with_file_name("settings.vdf"), snapshot).unwrap();
    }
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "-m", "scripts"]);
    (temp, script)
}

const NEEDS_VC: &str = "game.dll: imports vcruntime140.dll, which VC 2015 Redist, VC 2017 \
                        Redist, VC 2019 Redist or VC 2022 Redist installs";

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
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks; tick one under Installation, \
             Redistributables, ship the DLL in a depot, or link its library in statically"
        )),
        "{why}"
    );

    let (unreadable, _, broken) = checked("\"1000\" {");
    assert_eq!(unreadable, Some(2_i32), "{broken}");
    assert!(broken.contains("settings.vdf: "), "{broken}");
}

/// `steamship check` on the build folder of a game whose `game.dll` needs Visual C++ and whose
/// snapshot is `snapshot`, run in `folder`, or else in the game's own; with the folder's path.
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
            "{} is a build's folder, which holds no scripts; give --scripts-only the app build \
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
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks; tick one under Installation, \
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
fn upload_refuses_a_dll_nothing_installs_before_it_logs_in() {
    let home = tempfile::tempdir().unwrap();
    let (_project, script) = shipping_dll(&["kernel32.dll", "fmod.dll"], None);
    let (code, said, why) = upload(
        &script,
        home.path(),
        &["--version", "1.4.0"],
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(
            "game.dll: imports fmod.dll, which neither Windows, the build's depots nor a \
             Steamworks Common Redistributable has; ship it in a depot, or link its library in \
             statically"
        ),
        "{why}"
    );
    assert!(
        !home.path().join(install::FOLDER).exists(),
        "steamcmd was never installed to log in with"
    );
}

/// A steamcmd that, asked for app 1000's info, prints what the home's `app_info` file holds as
/// the settings, and otherwise has Valve build the upload as [`released_as`] does.
#[cfg(unix)]
const TICKING_ON_STEAM: &str = "#!/bin/sh\n\
    case \" $* \" in *' +app_info_print '*)\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
    cat \"$HOME/app_info\"\n\
    echo 'Unloading Steam API...OK'\n\
    exit 0;;\n\
    esac\n\
    echo built >> \"$HOME/builds\"\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

#[cfg(unix)]
#[test]
fn upload_asks_steam_which_redistributables_the_app_ticks_when_the_build_needs_one() {
    let home = faked_with(TICKING_ON_STEAM);
    // Steam is asked whether the branch exists, and once a build is up whether it is live.
    let uploaded = |ticked: &[&str], answers: Vec<(&'static str, &'static str)>| {
        fs::write(home.path().join("app_info"), settings_ticking(ticked)).unwrap();
        let (_project, script) = shipping_dll(&["kernel32.dll", "vcruntime140.dll"], None);
        let (host, _requests) = web_api(answers);
        upload(
            &script,
            home.path(),
            &["--version", "1.4.0"],
            &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
        )
    };

    let (refused, said, why) = uploaded(&[], vec![("200 OK", BETAS)]);
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains(&format!(
            "{NEEDS_VC}, and app 1000 ticks none in Steamworks"
        )),
        "{why}"
    );
    assert!(!home.path().join("builds").exists(), "nothing went up");

    let (code, shown, _) = uploaded(
        &["228988"],
        vec![("200 OK", BETAS), ("200 OK", testing_live(4242))],
    );
    assert_eq!(code, Some(0_i32), "{shown}");
    assert!(
        shown.contains("  redist    \u{2713} VC 2019 Redist, ticked in Steamworks\n"),
        "{shown}"
    );
    assert!(home.path().join("builds").exists(), "the build went up");
}

#[test]
fn builds_and_promote_that_steam_does_not_answer_say_so_and_exit_1() {
    let home = tempfile::tempdir().unwrap();
    // The second answer is for what `builds` asks after a 500: which apps the key holds.
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    for args in [
        &["builds", "1000010"][..],
        &["promote", "1000010", "--build", "7", "--branch", "testing"],
    ] {
        let (code, stdout, stderr) = steamship(args, Some(home.path()), &set);
        assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
        assert!(
            stdout.contains("  steam     \u{2717} no answer\n"),
            "{stdout}"
        );
        assert!(!stdout.contains("live on"), "{stdout}");
    }
}

/// `GetSchemaForGame` as Steam answered it for Spacewar, trimmed to two achievements.
const SCHEMA: &str = r#"{"game": {"gameName": "Spacewar", "availableGameStats": {"achievements": [
    {"name": "ACH_WIN_ONE_GAME", "displayName": "Winner", "hidden": 0,
     "description": "Win one game.", "icon": "https://cdn/w.jpg", "icongray": "https://cdn/w_bw.jpg"},
    {"name": "ACH_TRAVEL_FAR_SINGLE", "displayName": "Orbiter", "hidden": 1,
     "description": "Travel 500 feet in one life.", "icon": "https://cdn/o.jpg",
     "icongray": "https://cdn/o_bw.jpg"}
]}}}"#;

/// `GetGlobalAchievementPercentagesForApp` as Steam answered it for Team Fortress 2, its two
/// achievements named as Spacewar's are.
const SHARES: &str = r#"{"achievementpercentages":{"achievements":[
    {"name":"ACH_WIN_ONE_GAME","percent":"49.6"},{"name":"ACH_TRAVEL_FAR_SINGLE","percent":"3.1"}
]}}"#;

/// An achievements file for Spacewar, the second achievement's description as `orbiter` says.
fn achievements_file(folder: &Path, orbiter: &str) -> String {
    let path = folder.join("achievements.json");
    fs::write(
        &path,
        format!(
            r#"{{"app": 480, "achievements": [
                {{"api_name": "ACH_WIN_ONE_GAME", "name": "Winner", "description": "Win one game."}},
                {{"api_name": "ACH_TRAVEL_FAR_SINGLE", "name": "Orbiter", "description": "{orbiter}",
                  "hidden": true, "glyph": "rocket"}}
            ]}}"#
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn achievements_are_listed_and_checked_against_a_file_by_api_name() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", SCHEMA),
        ("200 OK", SHARES),
        ("200 OK", SCHEMA),
        ("200 OK", SCHEMA),
        ("500 Internal Server Error", ""),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("  steam     \u{2713} 2 achievements on Steam\n")
            && listed
                .contains("  steam     \u{2713} shares of 2 achievements and totals of 0 stats\n"),
        "{listed}"
    );
    assert!(
        listed.contains("  ACH_WIN_ONE_GAME      Winner, 49.6 % of players: Win one game.\n"),
        "{listed}"
    );
    assert!(
        listed.contains(
            "  ACH_TRAVEL_FAR_SINGLE Orbiter, hidden, 3.1 % of players: Travel 500 feet in one \
             life.\n"
        ),
        "{listed}"
    );
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamUserStats/GetSchemaForGame/v2/?")
            && asked.contains("appid=480")
            && asked.contains("l=english"),
        "{asked}"
    );
    let shared = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        shared.starts_with("GET /ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?")
            && shared.contains("gameid=480"),
        "{shared}"
    );

    let files = tempfile::tempdir().unwrap();
    let matching = achievements_file(files.path(), "Travel 500 feet in one life.");
    let (matched, said, _) = steamship(
        &["achievements", "480", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}");
    let fresh = |request: &str| {
        let (_, after) = request.split_once("steamship=").unwrap();
        after.split([' ', '&']).next().unwrap().to_owned()
    };
    let again = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_ne!(
        fresh(&asked),
        fresh(&again),
        "each ask is one Steam's cache has not answered"
    );
    assert!(said.contains("2 achievements\n"), "{said}");
    assert!(
        said.contains("Steam's achievements match the file\n"),
        "{said}"
    );

    let drifted = achievements_file(files.path(), "Go far.");
    let (refused, _, why) = steamship(
        &["achievements", "480", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "ACH_TRAVEL_FAR_SINGLE: the description is \"Travel 500 feet in one life.\" on Steam \
             and \"Go far.\" in the file"
        ),
        "{why}"
    );
    assert!(
        why.ends_with("1 of 2 achievements match: 1 differs\n"),
        "{why}"
    );

    // Steam failing is a failure, not drift.
    let (failed, _, failure) = steamship(
        &["achievements", "480", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(failed, Some(1_i32), "{failure}");
    assert!(failure.contains("Steam answered HTTP 500"), "{failure}");
}

/// `GetSchemaForGame` for Tidal Mill Inn's first sluice in English, as Steam answered it, its
/// icons trimmed.
const CELLAR: &str = r#"{"game": {"gameName": "Tidal Mill Inn", "availableGameStats": {
    "achievements": [{"name": "FIRST_ANCHOR", "displayName": "Down by the Wheel", "hidden": 0,
     "description": "Drain a mill's first sluice.", "icon": "https://cdn/c.jpg",
     "icongray": "https://cdn/c_bw.jpg"}]}}}"#;

/// The same in German.
const KELLER: &str = r#"{"game": {"gameName": "Tidal Mill Inn", "availableGameStats": {
    "achievements": [{"name": "FIRST_ANCHOR", "displayName": "Hinab zum Rad", "hidden": 0,
     "description": "Gib die erste Schleuse eines Mahlwerks frei.", "icon": "https://cdn/c.jpg",
     "icongray": "https://cdn/c_bw.jpg"}]}}}"#;

#[test]
fn achievements_are_checked_in_each_language_a_localisation_has() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", CELLAR),
        ("200 OK", KELLER),
        ("200 OK", CELLAR),
        ("200 OK", CELLAR),
        ("200 OK", CELLAR),
        ("500 Internal Server Error", ""),
        ("200 OK", CELLAR),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "achievement", "steam": "FIRST_ANCHOR", "name": "Down by the Wheel",
            "description": "Drain a mill's first sluice."}]"#,
    )
    .unwrap();
    let localisation = files.path().join("localisation.csv");
    fs::write(
        &localisation,
        "api_name (required; must match an api_name from the definitions CSV),\
         field (name or description),english,german\n\
         FIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad\n\
         FIRST_ANCHOR,description,Drain a mill's first sluice.,Gib die erste Schleuse eines Mahlwerks frei.\n",
    )
    .unwrap();
    let args = [
        "achievements",
        "1000050",
        "--check",
        &records.to_string_lossy(),
        &localisation.to_string_lossy(),
    ];

    let (code, said, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}");
    assert!(
        said.contains("localisation.csv, 1 achievement in 2 languages\n"),
        "{said}"
    );
    assert!(
        said.contains("  steam     \u{2713} in 1 language as well\n"),
        "{said}"
    );
    assert!(
        said.contains("Steam's achievements and stats match the files\n"),
        "{said}"
    );
    let first = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(first.contains("l=english"), "{first}");
    assert!(second.contains("l=german"), "{second}");

    // Steam answers a language it holds nothing in with the English texts.
    let (refused, _, why) = steamship(&args, Some(home.path()), &set);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "Steam holds nothing in german, so shows 1 achievement in English: import the \
             localisation in Steamworks, under Stats & Achievements, and publish\n"
        ),
        "{why}"
    );
    assert!(
        why.ends_with("0 of 1 achievements match: 1 differs\n"),
        "{why}"
    );

    // Steam failing in one of the languages is a failure, not drift.
    let (failed, _, failure) = steamship(&args, Some(home.path()), &set);
    assert_eq!(failed, Some(1_i32), "{failure}");
    assert!(failure.contains("Steam answered HTTP 500"), "{failure}");

    fs::write(
        &localisation,
        "api_name,field,english\n\
         FIRST_ANCHOR,name,Down by the Wheel\n\
         FIRST_ANCHOR,description,Drain a mill's first sluice.\n",
    )
    .unwrap();
    let (alone, english, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(alone, Some(0_i32), "{english}");
    assert!(
        !english.contains("as well"),
        "English alone is what Steam answered first: {english}"
    );

    fs::write(&localisation, "api_name,name,german\n").unwrap();
    let (unread, _, why_not) = steamship(&args, Some(home.path()), &set);
    assert_eq!(unread, Some(2_i32), "{why_not}");
    assert!(
        why_not.contains(
            "localisation.csv: is not the localisation Steamworks' achievement editor exports"
        ),
        "{why_not}"
    );
}

/// Tidal Mill Inn's achievement records, a games-kit mod's, with the cellar's in a set and an
/// icon for each.
const INN_RECORDS: &str = r#"[
    {"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"},
    {"type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar"},
    {"type": "achievement", "id": "base:five_levels", "steam": "FIVE_LEVELS",
     "name": "Five Turns Round", "description": "Spin the wheel five times in a tide.",
     "stat": "floors", "at": 5, "icon": "icons/five_levels.png"},
    {"type": "achievement", "id": "base:first_anchor", "steam": "FIRST_ANCHOR",
     "name": "Down by the Wheel", "description": "Drain a mill's first sluice.",
     "set": "cellar", "icon": "icons/first_anchor.png",
     "icon_locked": "icons/first_anchor_locked.png"}
]"#;

/// Its German catalogue.
const INN_GERMAN: &str = "msgid \"\"\nmsgstr \"Language: de\\n\"\n\n\
    msgctxt \"achievement.name\"\nmsgid \"base:five_levels\"\nmsgstr \"F\u{fc}nf Runden\"\n\n\
    msgctxt \"achievement.description\"\nmsgid \"base:five_levels\"\n\
    msgstr \"Drehe das Rad f\u{fc}nf Mal in einer Flut.\"\n\n\
    msgctxt \"achievement.name\"\nmsgid \"base:first_anchor\"\nmsgstr \"Hinab zum Rad\"\n\n\
    msgctxt \"achievement.description\"\nmsgid \"base:first_anchor\"\n\
    msgstr \"Gib die erste Schleuse eines Mahlwerks frei.\"\n\n\
    msgctxt \"achievement_set.name\"\nmsgid \"base:cellar\"\nmsgstr \"Der Keller\"\n";

/// A square icon of `size` pixels: a field of `shade` with a light square in its middle.
fn icon_png(size: u32, shade: u8) -> Vec<u8> {
    let quarter = size.checked_div(4).unwrap();
    let middle = quarter..size.saturating_sub(quarter);
    let picture = image::RgbaImage::from_fn(size, size, |x, y| {
        if middle.contains(&x) && middle.contains(&y) {
            image::Rgba([240, 220, 160, 255])
        } else {
            image::Rgba([shade, 40, 90, 255])
        }
    });
    let mut bytes = io::Cursor::new(Vec::new());
    picture
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

/// A games-kit mod holding [`INN_RECORDS`] under its `data` folder and their icons, with the
/// German catalogue beside it, as the game keeps them.
fn inn_mod() -> tempfile::TempDir {
    let game = tempfile::tempdir().unwrap();
    let base = game.path().join("mods").join("base");
    fs::create_dir_all(base.join("data")).unwrap();
    fs::create_dir_all(base.join("icons")).unwrap();
    fs::write(base.join("manifest.json"), "{}").unwrap();
    fs::write(base.join("data").join("achievements.json"), INN_RECORDS).unwrap();
    for (icon, size, shade) in [
        ("five_levels", 256, 200),
        ("first_anchor", 256, 120),
        ("first_anchor_locked", 64, 60),
    ] {
        fs::write(
            base.join("icons").join(format!("{icon}.png")),
            icon_png(size, shade),
        )
        .unwrap();
    }
    fs::create_dir_all(game.path().join("locale")).unwrap();
    fs::write(game.path().join("locale").join("de.po"), INN_GERMAN).unwrap();
    game
}

/// `steamship achievements --write` run on [`inn_mod`]'s records and catalogue in `game`, into
/// its `steam/achievements`, or checking it is `unchanged`.
fn write_inn(game: &Path, unchanged: bool) -> (Option<i32>, String, String) {
    let named = [
        game.join("mods/base/data/achievements.json"),
        game.join("locale/de.po"),
        game.join("steam").join("achievements"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german, out] = named.each_ref().map(String::as_str);
    let mut args = vec![
        "achievements",
        "1000050",
        "--write",
        records,
        german,
        "--out",
        out,
    ];
    if unchanged {
        args.push("--unchanged");
    }
    steamship(&args, Some(game), &[("STEAMSHIP_NO_UPDATE_CHECK", "1")])
}

#[test]
fn the_achievement_editors_files_are_written_from_records_catalogues_and_icons() {
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    let (missing, _, why) = write_inn(game.path(), true);
    assert_eq!(missing, Some(2_i32), "{why}");
    assert!(
        why.contains("1000050-achievements-definitions.csv is missing\n"),
        "{why}"
    );

    let (code, said, noted) = write_inn(game.path(), false);
    assert_eq!(code, Some(0_i32), "{said}{noted}");
    assert!(said.contains("de.po, german\n"), "{said}");
    assert!(!said.contains("unchecked"), "{said}");
    assert!(
        said.contains("holds the 6 files the achievement editor imports\n"),
        "{said}"
    );
    assert!(
        said.contains("drop every CSV and PNG file in ")
            && said.contains(
                " and Save imported data, staying on the page until Pending Changes is gone; then \
                 drop 1000050-achievements-localization.csv again on its own the same way"
            )
            && said.contains("publish, and run this again with --imported\n"),
        "{said}"
    );
    assert!(
        noted.contains("each set has to be made in the achievement editor before the import\n"),
        "{noted}"
    );
    assert!(
        noted.contains("no locked icon for 1 achievement: tick Automatically generate"),
        "{noted}"
    );
    let read = |name: &str| fs::read_to_string(out.join(name)).unwrap();
    assert!(read("1000050-achievements-definitions.csv").ends_with(
        "\nFIVE_LEVELS,,Client,false,false,floors,0,5\nFIRST_ANCHOR,1,Client,false,false,,0,0\n"
    ));
    assert!(
        read("1000050-achievements-localization.csv")
            .contains("\nFIRST_ANCHOR,name,Down by the Wheel,Hinab zum Rad\n")
    );
    assert_eq!(
        read("1000050-achievement-sets-localization.csv"),
        "setid (required; the id of a set that already exists),english,german\n\
         1,The Cellar,Der Keller\n"
    );
    assert_eq!(
        fs::read(out.join("FIRST_ANCHOR_UNACHIEVED.png")).unwrap(),
        icon_png(64, 60),
        "64 pixels is the least the editor takes"
    );
    assert!(!noted.contains("no icon for"), "{noted}");
    assert!(out.join("FIVE_LEVELS_ACHIEVED.png").is_file());

    let (same, unchanged, _) = write_inn(game.path(), true);
    assert_eq!(same, Some(0_i32), "{unchanged}");
    assert!(unchanged.contains("the 6 files in "), "{unchanged}");
}

#[test]
fn an_achievement_unlocked_by_the_first_floor_built_is_written_without_a_bar_and_its_old_one_named()
{
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    let (first, said, why) = write_inn(game.path(), false);
    assert_eq!(first, Some(0_i32), "{said}{why}");
    fs::write(
        game.path().join("mods/base/data/achievements.json"),
        INN_RECORDS.replace(r#""at": 5"#, r#""at": 1"#),
    )
    .unwrap();

    let (stale, _, why_stale) = write_inn(game.path(), true);
    assert_eq!(stale, Some(2_i32), "{why_stale}");
    assert!(
        why_stale
            .contains("1000050-achievements-definitions.csv differs from what the files make\n"),
        "{why_stale}"
    );
    let (rewritten, again, noted) = write_inn(game.path(), false);
    assert_eq!(rewritten, Some(0_i32), "{again}{noted}");
    assert!(
        fs::read_to_string(out.join("1000050-achievements-definitions.csv"))
            .unwrap()
            .contains("\nFIVE_LEVELS,,Client,false,false,,0,0\n"),
        "{again}"
    );
    assert!(
        noted.contains(
            "FIVE_LEVELS's progress bar goes from floors 0 - 5 to no progress bar, which Steam's \
             Web API cannot show: look at it in the editor once imported, and set it there by \
             hand if it has not moved, before publishing\n"
        ),
        "{noted}"
    );
    assert!(
        !noted.contains("each stat a progress bar counts"),
        "no bar is left to need its stat first: {noted}"
    );
}

#[test]
fn an_achievement_reworded_without_new_icons_is_named_to_look_at_them() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    let (first, said, why) = write_inn(game.path(), false);
    assert_eq!(first, Some(0_i32), "{said}{why}");
    let reworded = INN_RECORDS
        .replace("Down by the Wheel", "Under the Taproom")
        .replace(
            "Drain a mill's first sluice.",
            "Dig under an inn's taproom.",
        );
    fs::write(&records, &reworded).unwrap();
    let (code, _, noted) = write_inn(game.path(), false);
    assert_eq!(code, Some(0_i32), "{noted}");
    assert!(
        noted.contains(
            "FIRST_ANCHOR's English name and English description changed and its icons did not: \
             look at them, so that they still show what it asks for\n"
        ),
        "{noted}"
    );
    assert!(!noted.contains("FIVE_LEVELS's English"), "{noted}");

    // Drawn afresh with the new words, the icons are taken to show them.
    fs::write(
        &records,
        reworded.replace("Under the Taproom", "Below the Taproom"),
    )
    .unwrap();
    fs::write(
        game.path().join("mods/base/icons/first_anchor.png"),
        icon_png(256, 140),
    )
    .unwrap();
    let (redrawn, _, quiet) = write_inn(game.path(), false);
    assert_eq!(redrawn, Some(0_i32), "{quiet}");
    assert!(!quiet.contains("its icons did not"), "{quiet}");
}

#[test]
fn a_file_written_otherwise_or_left_over_is_named_and_rewritten_or_removed() {
    let game = inn_mod();
    let out = game.path().join("steam").join("achievements");
    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    fs::write(out.join("OLD_ACHIEVED.png"), png_header(64, 64)).unwrap();
    fs::write(out.join("notes.md"), "kept\n").unwrap();
    // The cellar's set dropped, and the floors counted and drawn no more.
    let records = INN_RECORDS
        .replace(
            r#"{"type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar"},"#,
            "",
        )
        .replace(r#""set": "cellar", "#, "")
        .replace(
            r#""stat": "floors", "at": 5, "icon": "icons/five_levels.png""#,
            r#""hidden": true"#,
        );
    fs::write(
        game.path().join("mods/base/data/achievements.json"),
        records,
    )
    .unwrap();
    let (stale, _, why_stale) = write_inn(game.path(), true);
    assert_eq!(stale, Some(2_i32), "{why_stale}");
    for why in [
        "1000050-achievements-definitions.csv differs from what the files make\n",
        "1000050-achievement-sets-localization.csv is left over\n",
        "FIVE_LEVELS_ACHIEVED.png is left over\n",
        "OLD_ACHIEVED.png is left over\n",
    ] {
        assert!(why_stale.contains(why), "{why} in {why_stale}");
    }
    let (rewritten, again, noted) = write_inn(game.path(), false);
    assert_eq!(rewritten, Some(0_i32), "{again}");
    for said in [
        "  removed   OLD_ACHIEVED.png\n",
        "  removed   1000050-achievement-sets-localization.csv\n",
        "  wrote     1000050-achievements-definitions.csv\n",
    ] {
        assert!(again.contains(said), "{said} in {again}");
    }
    assert!(
        !again.contains("wrote     1000050-achievements-localization.csv"),
        "a file already as written is left alone: {again}"
    );
    assert!(
        noted.contains("no icon for 1 achievement: Steam shows its own\n")
            && noted.contains(
                "FIVE_LEVELS's progress bar goes from floors 0 - 5 to no progress bar, which \
                 Steam's Web API cannot show: look at it in the editor once imported, and set it \
                 there by hand if it has not moved, before publishing\n"
            ),
        "{noted}"
    );
    for unsaid in [
        "each set",
        "each stat a progress bar counts",
        "no locked icon",
    ] {
        assert!(!noted.contains(unsaid), "{unsaid} in {noted}");
    }
    assert!(!out.join("OLD_ACHIEVED.png").exists());
    assert!(
        out.join("notes.md").is_file(),
        "a file of no kind written is left"
    );
    assert_eq!(write_inn(game.path(), true).0, Some(0_i32));
}

/// [`INN_SCHEMA`] holding [`INN_RECORDS`]' stat and achievements, with Steam's images of five
/// floors' icon and the cellar's two named by `icons` in turn.
fn inn_schema(icons: [&str; 3]) -> &'static str {
    let [five_levels, first_anchor, first_anchor_locked] = icons;
    INN_SCHEMA
        .replace(
            r#",{"name":"staff","defaultvalue":0,"displayName":"Staff working the stones"}"#,
            "",
        )
        .replace("LONG_JOURNEY", "FIVE_LEVELS")
        .replace("Rising Water", "Five Turns Round")
        .replace(
            "Bring the millpond up to its brim.",
            "Spin the wheel five times in a tide.",
        )
        .replace("CRAFTED_ALONE", "FIRST_ANCHOR")
        .replace("Bushel by Bushel", "Down by the Wheel")
        .replace(
            "Bag a sack of flour by hand.",
            "Drain a mill's first sluice.",
        )
        .replace("f895e737db45e18257a61fac5b6d409aa6872b82", five_levels)
        .replace("41373b0acc59cfbe2c973699975640bc1db1026a", first_anchor)
        .replace(
            "c4d7c246b05fc3bacc39c9106295794b52faee45",
            first_anchor_locked,
        )
        .leak()
}

/// `steamship achievements` in `game`, an [`inn_mod`], with its `steam/achievements` folder as
/// `--out`, and the Web API answering with `answers`: `--write`, `--imported` and its records
/// and catalogue when `imported`, or else `--check` and its records.
fn inn_on_steam<Body>(
    game: &Path,
    imported: bool,
    answers: Vec<(&'static str, Body)>,
) -> (Option<i32>, String, String)
where
    Body: AsRef<[u8]> + Send + 'static,
{
    let (host, _requests) = web_api(answers);
    inn_at(game, imported, &host)
}

/// [`inn_on_steam`] with the Web API's stand-in at `host`.
fn inn_at(game: &Path, imported: bool, host: &str) -> (Option<i32>, String, String) {
    let named = [
        game.join("mods/base/data/achievements.json"),
        game.join("locale/de.po"),
        game.join("steam").join("achievements"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german, out] = named.each_ref().map(String::as_str);
    let mut all = vec!["achievements", "1000050"];
    if imported {
        all.extend(["--write", records, german, "--imported"]);
    } else {
        all.extend(["--check", records]);
    }
    all.extend(["--out", out]);
    steamship(
        &all,
        Some(game),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, host),
        ],
    )
}

/// What a stand-in answers when steamship is refused before it asks Steam anything.
const UNASKED: [(&str, &str); 0] = [];

/// The icons the inn's import holds, in the order its records give them.
const INN_ICONS: [&str; 3] = [
    "FIVE_LEVELS_ACHIEVED.png",
    "FIRST_ANCHOR_ACHIEVED.png",
    "FIRST_ANCHOR_UNACHIEVED.png",
];

/// A JPEG of the PNG `file`, as Steam serves an icon it took.
fn served(file: &[u8]) -> Vec<u8> {
    let picture = image::load_from_memory_with_format(file, image::ImageFormat::Png)
        .unwrap()
        .to_rgb8();
    let mut bytes = io::Cursor::new(Vec::new());
    picture
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

/// Steam's answers to `--imported` in `game`, an [`inn_mod`] whose files are written, once Steam
/// took them in the import whose images `icons` names: the schema, then each image it serves.
fn inn_served(game: &Path, icons: [&'static str; 3]) -> Vec<(&'static str, Vec<u8>)> {
    let out = game.join("steam").join("achievements");
    iter::once(("200 OK", inn_schema(icons).as_bytes().to_vec()))
        .chain(INN_ICONS.map(|name| ("200 OK", served(&fs::read(out.join(name)).unwrap()))))
        .collect()
}

/// The names [`INN_SCHEMA`] gives Steam's images of five floors' icon and the cellar's two, as
/// the inn's first import made them.
const FIRST_IMPORT: [&str; 3] = [
    "f895e737db45e18257a61fac5b6d409aa6872b82",
    "41373b0acc59cfbe2c973699975640bc1db1026a",
    "c4d7c246b05fc3bacc39c9106295794b52faee45",
];

/// Steam's answers to a check of the inn's records, its icons' images named by `icons`.
fn inn_checked(icons: [&'static str; 3]) -> Vec<(&'static str, &'static str)> {
    iter::once(("200 OK", inn_schema(icons)))
        .chain(TTI_UNAGGREGATED.into_iter().take(1))
        .collect()
}

/// An [`inn_mod`] with its files written and `--imported` run once Steam took them in
/// [`FIRST_IMPORT`]; and what `--imported` printed and noted.
fn inn_imported() -> (tempfile::TempDir, String, String) {
    let game = inn_mod();
    let (written, wrote, why) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why}");
    let (recorded, said, noted) =
        inn_on_steam(game.path(), true, inn_served(game.path(), FIRST_IMPORT));
    assert_eq!(recorded, Some(0_i32), "{said}{noted}");
    (game, said, noted)
}

/// The receipt in `game`, an [`inn_mod`].
fn inn_receipt(game: &Path) -> PathBuf {
    game.join("steam")
        .join("achievements")
        .join("steam-icons.json")
}

#[test]
fn imported_records_each_icon_steam_holds_and_write_leaves_the_receipt_alone() {
    let (game, said, noted) = inn_imported();
    assert!(
        said.contains("  steam     \u{2713} 3 of 3 icons as the files\n")
            && said.contains("  wrote     steam-icons.json\n")
            && said.contains("records the 3 icons Steam has published, each as its file\n"),
        "{said}"
    );
    assert!(!noted.contains("published:"), "{noted}");
    let receipt = inn_receipt(game.path());
    let kept: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
    assert_eq!(
        kept.get("app").and_then(serde_json::Value::as_u64),
        Some(1_000_050)
    );
    assert_eq!(
        kept.pointer("/icons/FIRST_ANCHOR_UNACHIEVED.png/steam")
            .and_then(serde_json::Value::as_str),
        Some("c4d7c246b05fc3bacc39c9106295794b52faee45.jpg")
    );
    assert_eq!(
        kept.pointer("/icons/FIVE_LEVELS_ACHIEVED.png/sha256")
            .and_then(serde_json::Value::as_str)
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        write_inn(game.path(), true).0,
        Some(0_i32),
        "the receipt is no file --write makes, nor one left over"
    );
    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    assert!(receipt.is_file(), "--write leaves the receipt alone");

    let (matched, shown, _) = inn_on_steam(game.path(), false, inn_checked(FIRST_IMPORT));
    assert_eq!(matched, Some(0_i32), "{shown}");
    assert!(
        shown.contains("Steam's achievements and stats match the file\n"),
        "{shown}"
    );
}

#[test]
fn an_icon_changed_in_the_files_is_named_until_steam_takes_its_import() {
    let (game, _, _) = inn_imported();
    let first = FIRST_IMPORT;
    let checked = inn_checked;
    fs::write(
        game.path().join("mods/base/icons/five_levels.png"),
        icon_png(128, 20),
    )
    .unwrap();
    let (changed, _, why_changed) = inn_on_steam(game.path(), false, checked(first));
    assert_eq!(changed, Some(2_i32), "{why_changed}");
    assert!(
        why_changed.contains(
            "FIVE_LEVELS: the icon changed in the files since the import steam-icons.json \
             records: import it, publish, then run --imported\n"
        ) && why_changed.contains("1 of 2 achievements match: 1 differs\n")
            && why_changed.contains(
                "Steam answers with what is published: an import saved in Steamworks counts \
                 here once it is published\n"
            ),
        "{why_changed}"
    );

    let (unwritten, told, why_unwritten) = inn_on_steam(game.path(), true, UNASKED.to_vec());
    assert_eq!(unwritten, Some(2_i32), "{why_unwritten}");
    assert!(
        why_unwritten.contains("FIVE_LEVELS_ACHIEVED.png differs from what the files make\n")
            && told.contains("run it without --imported to write "),
        "{told}{why_unwritten}"
    );

    assert_eq!(write_inn(game.path(), false).0, Some(0_i32));
    let (untaken, _, why_untaken) =
        inn_on_steam(game.path(), true, vec![("200 OK", inn_schema(first))]);
    assert_eq!(untaken, Some(2_i32), "{why_untaken}");
    assert!(
        why_untaken.contains(
            "FIVE_LEVELS: Steam still has the icon the last import made, where the file changed \
             since\n"
        ),
        "{why_untaken}"
    );
    let again = [
        "0d2c5c6b7e3a1f4e9b8a7c6d5e4f3a2b1c0d9e8f",
        first[1],
        first[2],
    ];
    let (taken, said_again, noted_again) =
        inn_on_steam(game.path(), true, inn_served(game.path(), again));
    assert_eq!(taken, Some(0_i32), "{said_again}{noted_again}");
    let (matched, shown, _) = inn_on_steam(game.path(), false, checked(again));
    assert_eq!(matched, Some(0_i32), "{shown}");
}

#[test]
fn imported_names_once_an_achievement_steam_does_not_have() {
    let game = inn_mod();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let unpublished = inn_schema(FIRST_IMPORT)
        .replace(r#""name":"FIRST_ANCHOR""#, r#""name":"LAST_CELLAR""#)
        .leak();
    let (refused, _, why) = inn_on_steam(game.path(), true, vec![("200 OK", unpublished)]);
    assert_eq!(refused, Some(2_i32), "{why}");
    assert_eq!(
        why.matches("FIRST_ANCHOR").count(),
        1,
        "the achievement, not each of its two icons: {why}"
    );
    assert!(why.contains("FIRST_ANCHOR: not on Steam\n"), "{why}");
    assert!(!inn_receipt(game.path()).exists());
}

#[test]
fn imported_refuses_an_icon_steam_has_that_the_files_give_none_of() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    fs::write(
        &records,
        INN_RECORDS.replace(r#", "icon": "icons/five_levels.png""#, ""),
    )
    .unwrap();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let retired = r#""achievements":[{"name":"OLD_TAPROOM","defaultvalue":0,"displayName":"Old Taproom","hidden":0,"description":"Keep the old taproom.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/9e1f0a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f.jpg","icongray":""},"#;
    let uploaded = inn_schema(FIRST_IMPORT)
        .replacen(r#""achievements":["#, retired, 1)
        .leak();
    let (refused, told, why) = inn_on_steam(game.path(), true, vec![("200 OK", uploaded)]);
    assert_eq!(refused, Some(2_i32), "{told}{why}");
    assert!(
        why.contains("FIVE_LEVELS: the files give no icon, where Steam has one\n"),
        "{why}"
    );
    assert!(
        !why.contains("OLD_TAPROOM"),
        "an achievement the files leave out is --check's to name: {why}"
    );
    assert!(
        !why.contains("no locked icon"),
        "Steam makes a locked icon itself when asked to: {why}"
    );
    assert!(
        told.contains("give each of those achievements its `icon` in the files, run it without ")
            && !told.contains("icons as the files"),
        "{told}"
    );
    assert!(!inn_receipt(game.path()).exists());
}

#[test]
fn imported_refuses_an_icon_steam_serves_otherwise_than_the_file() {
    let game = inn_mod();
    let (written, wrote, why_written) = write_inn(game.path(), false);
    assert_eq!(written, Some(0_i32), "{wrote}{why_written}");
    let mut answers = inn_served(game.path(), FIRST_IMPORT);
    let [_, five_levels, first_anchor, first_anchor_locked] = answers.as_mut_slice() else {
        panic!("the schema and three images");
    };
    *five_levels = ("200 OK", served(&icon_png(256, 20)));
    *first_anchor = ("404 Not Found", Vec::new());
    *first_anchor_locked = ("200 OK", b"not a JPEG".to_vec());
    let (host, requests) = web_api(answers);
    let (refused, told, why) = inn_at(game.path(), true, &host);
    assert_eq!(refused, Some(2_i32), "{told}{why}");
    for said in [
        "FIVE_LEVELS: Steam's icon is not the file's: 49152 of 65536 pixels more than 64 off, ",
        "FIRST_ANCHOR: Steam's icon could not be compared with the file: Steam's image host \
         answered HTTP 404\n",
        "FIRST_ANCHOR: Steam's locked icon could not be compared with the file: Steam's image: ",
        "Steam answers with what is published: an import saved in Steamworks counts here once \
         it is published\n",
    ] {
        assert!(why.contains(said), "{said} in {why}");
    }
    assert!(
        told.contains("  steam     \u{2717} 0 of 3 icons as the files\n")
            && told.contains("drop the PNG files in ")
            && told.contains(
                " again, Save imported data, staying on the page until Pending Changes is gone, \
                 and publish, then run --imported again\n"
            ),
        "{told}"
    );
    assert!(!inn_receipt(game.path()).exists());

    let asked: Vec<String> = requests.try_iter().collect();
    let images: Vec<&String> = asked
        .iter()
        .filter(|request| request.contains("/community_assets/images/apps/1000050/"))
        .collect();
    assert_eq!(images.len(), 3, "{asked:?}");
    assert!(
        images.first().is_some_and(|request| {
            request.starts_with(
                "GET /community_assets/images/apps/1000050/\
                 f895e737db45e18257a61fac5b6d409aa6872b82.jpg ",
            )
        }),
        "{images:?}"
    );
    assert!(
        images
            .iter()
            .all(|request| !request.to_ascii_lowercase().contains("x-webapi-key")),
        "the key goes to the Web API alone: {images:?}"
    );
}

#[test]
fn an_icon_steam_made_anew_from_the_file_imported_is_named() {
    let (game, _, _) = inn_imported();
    let [five_levels, first_anchor, _] = FIRST_IMPORT;
    let redrawn = [
        five_levels,
        first_anchor,
        "5b1e0c2d9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c",
    ];
    let (moved, _, why_moved) = inn_on_steam(game.path(), false, inn_checked(redrawn));
    assert_eq!(moved, Some(2_i32), "{why_moved}");
    assert!(
        why_moved.contains(
            "FIRST_ANCHOR: the locked icon on Steam changed since the import steam-icons.json \
             records, and not from the files: import it again, publish, then run --imported\n"
        ),
        "{why_moved}"
    );
}

#[test]
fn a_check_against_no_receipt_or_another_apps_is_refused_and_imported_replaces_one() {
    let bare = inn_mod();
    assert_eq!(write_inn(bare.path(), false).0, Some(0_i32));
    let (unrecorded, _, why) = inn_on_steam(bare.path(), false, UNASKED.to_vec());
    assert_eq!(unrecorded, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "achievements: has no steam-icons.json: import and publish the files --write wrote \
             there, then run --imported\n"
        ),
        "{why}"
    );
    let unreadable = inn_receipt(bare.path());
    fs::create_dir_all(&unreadable).unwrap();
    let (failed, _, why_failed) = inn_on_steam(bare.path(), false, UNASKED.to_vec());
    assert_eq!(failed, Some(1_i32), "{why_failed}");
    assert!(
        why_failed.contains(&format!("{}: ", unreadable.display()))
            && !why_failed.contains("has no steam-icons.json"),
        "a receipt that cannot be read is not taken for none: {why_failed}"
    );

    let (game, _, _) = inn_imported();
    let receipt = inn_receipt(game.path());
    let kept = fs::read_to_string(&receipt).unwrap();
    fs::write(&receipt, kept.replace("1000050", "480")).unwrap();
    let (elsewhere, _, why_elsewhere) = inn_on_steam(game.path(), false, UNASKED.to_vec());
    assert_eq!(elsewhere, Some(2_i32), "{why_elsewhere}");
    assert!(
        why_elsewhere.contains("steam-icons.json: records app 480's icons, not app 1000050's\n"),
        "{why_elsewhere}"
    );
    fs::write(&receipt, "{}").unwrap();
    let (unread, _, why_unread) = inn_on_steam(game.path(), false, UNASKED.to_vec());
    assert_eq!(unread, Some(2_i32), "{why_unread}");
    assert!(
        why_unread.contains("steam-icons.json: \"app\" is not an app ID\n"),
        "{why_unread}"
    );
    let (replaced, _, _) = inn_on_steam(game.path(), true, inn_served(game.path(), FIRST_IMPORT));
    assert_eq!(
        replaced,
        Some(0_i32),
        "--imported replaces a receipt it cannot use"
    );
    assert!(
        fs::read_to_string(&receipt)
            .unwrap()
            .contains("\"app\": 1000050")
    );
}

#[test]
fn a_receipts_flags_are_refused_where_they_mean_nothing() {
    let game = tempfile::tempdir().unwrap();
    for (args, refusal) in [
        (
            ["--out", "steam/achievements", "--json"].as_slice(),
            "the following required arguments were not provided",
        ),
        (
            [
                "--write",
                "a.json",
                "--out",
                "o",
                "--imported",
                "--unchanged",
            ]
            .as_slice(),
            "cannot be used with",
        ),
        (["--imported"].as_slice(), "there is no game in "),
    ] {
        let mut all = vec!["achievements", "1000050"];
        all.extend(args);
        let (refused, _, why) = steamship(&all, Some(game.path()), &[]);
        assert_eq!(refused, Some(2_i32), "{args:?}: {why}");
        assert!(why.contains(refusal), "{args:?}: {why}");
    }
}

#[test]
fn the_achievement_editors_files_are_refused_as_the_editor_would_refuse_them() {
    let game = inn_mod();
    let base = game.path().join("mods/base");
    let records = base.join("data/achievements.json");
    let german = game.path().join("locale/de.po");
    let out = game.path().join("out");
    let write = |files: &[&Path]| {
        let mut args = vec!["achievements", "1000050", "--write"];
        let named: Vec<String> = files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        args.extend(named.iter().map(String::as_str));
        let out = out.to_string_lossy().into_owned();
        args.extend(["--out", &out]);
        let (code, _, why) = steamship(
            &args,
            Some(game.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(2_i32), "{why}");
        why
    };

    fs::write(base.join("icons/five_levels.png"), png_header(256, 128)).unwrap();
    let oblong = write(&[&records]);
    assert!(
        oblong.contains(
            "FIVE_LEVELS's icon is a PNG of 256 by 128 pixels, where the achievement editor \
             takes a square PNG of 64 pixels or more, 256 at best\n"
        ),
        "{oblong}"
    );
    fs::write(base.join("icons/five_levels.png"), png_header(32, 32)).unwrap();
    let small = write(&[&records]);
    assert!(
        small.contains("FIVE_LEVELS's icon is a PNG of 32 by 32 pixels"),
        "{small}"
    );
    fs::write(base.join("icons/five_levels.png"), png_header(256, 256)).unwrap();

    let english = game.path().join("locale/en.po");
    fs::write(&english, "msgid \"\"\nmsgstr \"Language: en\\n\"\n").unwrap();
    let in_english = write(&[&records, &english]);
    assert!(
        in_english
            .contains("en.po: is in English, the language the game's own files are written in\n"),
        "{in_english}"
    );
    let again = game.path().join("locale/de_DE.po");
    fs::write(&again, INN_GERMAN).unwrap();
    let twice = write(&[&records, &german, &again]);
    assert!(
        twice.contains(&format!(
            "de_DE.po: is in german, as {} is\n",
            german.display()
        )),
        "{twice}"
    );

    fs::write(&german, INN_GERMAN.replace("\"Hinab zum Rad\"", "\"\"")).unwrap();
    let untranslated = write(&[&records, &german]);
    assert!(
        untranslated.contains(
            "de.po has no name of FIRST_ANCHOR, under msgctxt \"achievement.name\" and msgid \
             \"base:first_anchor\"\n"
        ),
        "{untranslated}"
    );

    let klingon = game.path().join("locale/tlh.po");
    fs::write(&klingon, "msgid \"x\"\nmsgstr \"y\"\n").unwrap();
    let unknown = write(&[&records, &klingon]);
    assert!(
        unknown.contains("tlh.po: is in the locale tlh, which Steam has no language for\n"),
        "{unknown}"
    );

    fs::write(&records, INN_RECORDS.replace("\"stat\": \"floors\", ", "")).unwrap();
    let uncounted = write(&[&records]);
    assert!(
        uncounted.contains(
            "FIVE_LEVELS unlocks at 5 but gives no \"stat\" for its progress bar to count\n"
        ),
        "{uncounted}"
    );
    assert!(
        !out.exists(),
        "nothing is written while the files are refused"
    );

    fs::write(&records, INN_RECORDS).unwrap();
    fs::write(&out, "notes\n").unwrap();
    let filed = write(&[&records]);
    assert!(
        filed.contains(&format!(
            "{}: is a file, where --out names the folder the files are written into\n",
            out.display()
        )),
        "{filed}"
    );
    assert_eq!(fs::read_to_string(&out).unwrap(), "notes\n");
}

#[test]
fn an_achievement_and_a_stat_of_one_api_name_are_refused_before_anything_is_written() {
    let game = inn_mod();
    let records = game.path().join("mods/base/data/achievements.json");
    fs::write(
        &records,
        INN_RECORDS.replace("\"steam\": \"floors\"", "\"steam\": \"FIVE_LEVELS\""),
    )
    .unwrap();
    let out = game.path().join("out");
    let named = [&records, &out].map(|path| path.to_string_lossy().into_owned());
    for args in [
        ["--write", &named[0], "--out", &named[1]].as_slice(),
        ["--check", &named[0]].as_slice(),
    ] {
        let (code, _, why) = steamship(
            &[["achievements", "1000050"].as_slice(), args].concat(),
            Some(game.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(2_i32), "{args:?}: {why}");
        assert!(
            why.contains(&format!(
                "FIVE_LEVELS is both an achievement and a stat in {}: Steamworks publishes no \
                 achievement and stat of one API name, so rename one\n",
                records.display()
            )),
            "{args:?}: {why}"
        );
    }
    assert!(
        !out.exists(),
        "nothing is written while the files are refused"
    );
}

#[test]
fn more_than_a_hundred_achievements_are_said_to_need_a_vetted_app() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("achievements.json");
    let out = folder.path().join("out");
    for (count, said) in [(100_u32, false), (101_u32, true)] {
        let achievements: Vec<String> = (0_u32..count)
            .map(|each| format!(r#"{{"api_name": "A{each}", "name": "A"}}"#))
            .collect();
        fs::write(
            &file,
            format!(r#"{{"achievements": [{}]}}"#, achievements.join(",")),
        )
        .unwrap();
        let (code, _, noted) = steamship(
            &[
                "achievements",
                "480",
                "--write",
                &file.to_string_lossy(),
                "--out",
                &out.to_string_lossy(),
            ],
            Some(folder.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(code, Some(0_i32), "{noted}");
        assert_eq!(
            noted.contains(
                "Steam takes more than 100 achievements only once Valve has vetted the app\n"
            ),
            said,
            "{count}: {noted}"
        );
    }
}

#[test]
fn achievements_are_checked_in_each_language_a_catalogue_is_in() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![("200 OK", CELLAR), ("200 OK", KELLER)]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "achievement", "id": "base:first_anchor", "steam": "FIRST_ANCHOR",
            "name": "Down by the Wheel", "description": "Drain a mill's first sluice."}]"#,
    )
    .unwrap();
    let german = files.path().join("de.po");
    fs::write(
        &german,
        "msgctxt \"achievement.name\"\nmsgid \"base:first_anchor\"\nmsgstr \"Hinab zum Rad\"\n\
         msgctxt \"achievement.description\"\nmsgid \"base:first_anchor\"\n\
         msgstr \"Gib die erste Schleuse eines Mahlwerks frei.\"\n",
    )
    .unwrap();
    let args = [
        "achievements",
        "1000050",
        "--check",
        &records.to_string_lossy(),
        &german.to_string_lossy(),
    ];

    let (code, said, _) = steamship(&args, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}");
    assert!(said.contains("de.po, german\n"), "{said}");
    assert!(
        said.contains("Steam's achievements and stats match the files\n"),
        "{said}"
    );
    let first = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(first.contains("l=english"), "{first}");
    assert!(second.contains("l=german"), "{second}");
}

#[test]
fn achievements_json_prints_them_alone_on_standard_output_and_not_with_check() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", SCHEMA),
        ("403 Forbidden", "{}"),
        ("200 OK", WITH_STATS),
        ("403 Forbidden", "{}"),
        ("200 OK", NOT_AGGREGATED),
        ("200 OK", TOTALS),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let (code, stdout, stderr) =
        steamship(&["achievements", "480", "--json"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert_eq!(
        summary_of(&stdout),
        serde_json::json!({
            "app": 480_u32,
            "achievements": [
                {"api_name": "ACH_WIN_ONE_GAME", "name": "Winner", "description": "Win one game.",
                 "hidden": false, "icon": "https://cdn/w.jpg", "icon_locked": "https://cdn/w_bw.jpg",
                 "percent": null},
                {"api_name": "ACH_TRAVEL_FAR_SINGLE", "name": "Orbiter",
                 "description": "Travel 500 feet in one life.", "hidden": true,
                 "icon": "https://cdn/o.jpg", "icon_locked": "https://cdn/o_bw.jpg", "percent": null},
            ],
            "stats": [],
        })
    );
    assert!(
        stderr.contains("2 achievements on Steam")
            && stderr.contains("Steam gives no shares of players or totals yet"),
        "{stderr}"
    );

    let (with_stats, statted, told) =
        steamship(&["achievements", "480", "--json"], Some(home.path()), &set);
    assert_eq!(with_stats, Some(0_i32), "{statted}{told}");
    assert_eq!(
        summary_of(&statted).get("stats"),
        Some(&serde_json::json!([
            {"api_name": "NumGames", "name": "Games played", "default": 0.0_f64,
             "total": 211_140_488_449_u64},
            {"api_name": "AverageSpeed", "name": "", "default": 1.5_f64, "total": null},
        ]))
    );
    assert!(
        told.contains("shares of 0 achievements and totals of 1 stat"),
        "{told}"
    );
    let totals: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(6)
            .filter(|request| request.contains("GetGlobalStatsForGame"))
            .collect();
    assert!(
        totals.len() == 2
            && totals.iter().all(|asked| asked.contains("appid=480"))
            && totals.first().is_some_and(|asked| {
                asked.contains("count=2")
                    && asked.contains("=NumGames")
                    && asked.contains("=AverageSpeed")
            })
            && totals.last().is_some_and(|asked| {
                asked.contains("count=1") && !asked.contains("AverageSpeed")
            }),
        "Steam is asked again without the stat it does not add up: {totals:?}"
    );

    let (refused, nothing, why) = steamship(
        &[
            "achievements",
            "480",
            "--json",
            "--check",
            "achievements.json",
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!((refused, nothing.as_str()), (Some(2_i32), ""), "{why}");
    assert!(why.contains("cannot be used with"), "{why}");
}

/// `GetSchemaForGame` for an app with stats and no achievements.
const WITH_STATS: &str = r#"{"game": {"availableGameStats": {"stats": [
    {"name": "NumGames", "defaultvalue": 0, "displayName": "Games played"},
    {"name": "AverageSpeed", "defaultvalue": 1.5}
]}}}"#;

/// `GetGlobalStatsForGame` refusing a stat it keeps no totals of, as Steam answered for Spacewar,
/// the stat named as `WITH_STATS` names it.
const NOT_AGGREGATED: &str =
    r#"{"response":{"result":8,"error":"Stat 'AverageSpeed' is not an aggregated stat"}}"#;

/// `GetGlobalStatsForGame` as Steam answered it for Spacewar, trimmed to one stat.
const TOTALS: &str =
    r#"{"response":{"globalstats":{"NumGames":{"total":"211140488449"}},"result":1}}"#;

#[test]
fn stats_are_listed_and_checked_only_when_the_file_lists_them() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("200 OK", WITH_STATS),
        ("403 Forbidden", "{}"),
        // A stat steamship never asked for, as Steam named one for Spacewar: asking again
        // without it would get the same answer, so totals are given up rather than asked for
        // without end.
        (
            "200 OK",
            r#"{"response":{"result":8,"error":"Stat 'NumWins' is not an aggregated stat"}}"#,
        ),
        ("200 OK", WITH_STATS),
        ("200 OK", WITH_STATS),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("\u{2713} 0 achievements and 2 stats on Steam\n")
            && listed.contains("  stat NumGames     Games played, starts at 0\n")
            && listed.contains("  stat AverageSpeed starts at 1.5\n")
            && !listed.contains("Stats & Achievements"),
        "stats alone are something on Steam: {listed}"
    );

    let unlisted = files.path().join("unlisted.json");
    fs::write(&unlisted, r#"{"achievements": []}"#).unwrap();
    let (matched, said, _) = steamship(
        &[
            "achievements",
            "480",
            "--check",
            &unlisted.to_string_lossy(),
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(
        matched,
        Some(0_i32),
        "no list, so the stats are not checked: {said}"
    );
    assert!(
        said.contains("Steam's achievements match the file\n"),
        "{said}"
    );

    let drifted = files.path().join("drifted.json");
    fs::write(
        &drifted,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "default": 0},
            {"api_name": "AverageSpeed", "default": 2}
        ]}"#,
    )
    .unwrap();
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &drifted.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(shown.contains("0 achievements and 2 stats\n"), "{shown}");
    assert!(
        why.contains("stat AverageSpeed: the default is \"1.5\" on Steam and \"2\" in the file"),
        "{why}"
    );
    assert!(
        why.ends_with("1 of 2 stats match: 1 differs\n") && !shown.contains("achievements match"),
        "the file lists no achievements, so only the stats are counted: {shown}{why}"
    );
}

#[test]
fn whether_a_stat_is_aggregated_is_checked_against_the_totals_steam_keeps() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", WITH_STATS),
        ("200 OK", NOT_AGGREGATED),
        ("200 OK", TOTALS),
        ("200 OK", WITH_STATS),
        // As Steam answered for Tidal Mill Inn before its release.
        (
            "200 OK",
            r#"{"response":{"result":24,"error":"Insufficient Privilege"}}"#,
        ),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let both = files.path().join("both.json");
    fs::write(
        &both,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "aggregated": true},
            {"api_name": "AverageSpeed", "default": 1.5, "aggregated": true},
            {"api_name": "Unmade", "aggregated": true}
        ]}"#,
    )
    .unwrap();
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &both.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        shown.contains("  steam     \u{2713} 1 of 2 stats aggregated\n"),
        "{shown}"
    );
    assert!(
        why.contains(
            "stat AverageSpeed: aggregated is \"false\" on Steam and \"true\" in the file\n"
        ) && why.ends_with("1 of 3 stats match: 1 differs, 1 not on Steam\n")
            && !why.contains("went unchecked"),
        "Steam is asked only of the stats it holds, and says of each: {why}"
    );
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(3)
            .collect();
    assert!(
        asked.get(1).is_some_and(|first| first
            .starts_with("GET /ISteamUserStats/GetGlobalStatsForGame/v1/?")
            && first.contains("count=2")
            && !first.contains("Unmade"))
            && asked
                .last()
                .is_some_and(|again| again.contains("count=1") && !again.contains("AverageSpeed")),
        "{asked:?}"
    );

    let held = files.path().join("held.json");
    fs::write(
        &held,
        r#"{"achievements": [], "stats": [
            {"api_name": "NumGames", "name": "Games played", "aggregated": true},
            {"api_name": "AverageSpeed", "default": 1.5, "aggregated": true}
        ]}"#,
    )
    .unwrap();
    let (matched, said, noted) = steamship(
        &["achievements", "480", "--check", &held.to_string_lossy()],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}{noted}");
    assert!(
        noted.contains(
            "whether 2 stats are aggregated went unchecked: Steam answered \"Insufficient \
             Privilege\""
        ) && said.contains("Steam's achievements and stats match the file\n"),
        "{said}{noted}"
    );
}

#[test]
fn a_check_against_an_app_with_nothing_on_steam_says_so_once_for_each_kind_in_any_language() {
    let game = inn_mod();
    let named = [
        game.path().join("mods/base/data/achievements.json"),
        game.path().join("locale/de.po"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    let [records, german] = named.each_ref().map(String::as_str);
    // One answer only: asking for the achievements in German as well would find none.
    let (host, _requests) = web_api(vec![("200 OK", r#"{"game": {}}"#)]);
    let (refused, shown, why) = steamship(
        &["achievements", "1000050", "--check", records, german],
        Some(game.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains(
            "Steam holds none of the file's 2 achievements: they count once entered and \
             published in Steamworks\n"
        ) && why.contains(
            "Steam holds none of the file's 1 stat: it counts once entered and published in \
             Steamworks\n"
        ) && !why.contains("stat floors"),
        "{why}"
    );
    assert!(!shown.contains("more language"), "{shown}");
}

#[test]
fn an_app_with_nothing_on_steam_is_listed_as_none_with_where_they_are_made() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("200 OK", r#"{"game": {}}"#),
        ("200 OK", r#"{"game": {}}"#),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

    let (code, listed, _) = steamship(&["achievements", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("\u{2713} 0 achievements on Steam\n")
            && listed.contains(
                "\n  next      make and publish app 480's achievements and stats in Steamworks, \
                 under Stats & Achievements\n"
            ),
        "in the same column as the lines above it: {listed}"
    );

    let file = achievements_file(files.path(), "Travel 500 feet in one life.");
    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &file],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains("Steam holds none of the file's 2 achievements")
            && why.ends_with("0 of 2 achievements match: 2 not on Steam\n")
            && !shown.contains("Stats & Achievements"),
        "{shown}{why}"
    );
    assert!(
        !shown.contains("more language"),
        "Steam has a text in no language of an achievement it does not hold: {shown}"
    );
}

#[test]
fn a_check_with_drift_in_its_stats_still_says_its_achievements_match() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", SCHEMA)]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let file = achievements_file(files.path(), "Travel 500 feet in one life.");
    let listed = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        listed.replacen(
            r#""achievements":"#,
            r#""stats": [{"api_name": "years_open", "default": 0}], "achievements":"#,
            1,
        ),
    )
    .unwrap();

    let (refused, shown, why) = steamship(
        &["achievements", "480", "--check", &file],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{shown}{why}");
    assert!(shown.contains("2 of 2 achievements match\n"), "{shown}");
    assert!(
        why.contains("Steam holds none of the file's 1 stat: it counts once entered and published")
            && why.ends_with("0 of 1 stats match: 1 not on Steam\n"),
        "{why}"
    );
}

/// `GetSchemaForGame` as Steam answered it for Tidal Mill Inn, trimmed to two stats and two
/// achievements.
const INN_SCHEMA: &str = r#"{"game":{"gameName":"Tidal Mill Inn","gameVersion":"2","availableGameStats":{"stats":[{"name":"floors","defaultvalue":0,"displayName":"Floodwater held behind the sluice"},{"name":"staff","defaultvalue":0,"displayName":"Staff working the stones"}],"achievements":[{"name":"LONG_JOURNEY","defaultvalue":0,"displayName":"Rising Water","hidden":0,"description":"Bring the millpond up to its brim.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/f895e737db45e18257a61fac5b6d409aa6872b82.jpg","icongray":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/a25a9ca72da3dae23b05f2fb6f712527cf3634d9.jpg"},{"name":"CRAFTED_ALONE","defaultvalue":0,"displayName":"Bushel by Bushel","hidden":0,"description":"Bag a sack of flour by hand.","icon":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/41373b0acc59cfbe2c973699975640bc1db1026a.jpg","icongray":"https://steamcdn-a.akamaihd.net/steamcommunity/public/images/apps/1000050/c4d7c246b05fc3bacc39c9106295794b52faee45.jpg"}]}}}"#;

/// What Steam answers for Tidal Mill Inn when asked for the totals of its stats, which it keeps
/// none of, naming each in turn.
const TTI_UNAGGREGATED: [(&str, &str); 2] = [
    (
        "200 OK",
        r#"{"response":{"result":8,"error":"Stat 'floors' is not an aggregated stat"}}"#,
    ),
    (
        "200 OK",
        r#"{"response":{"result":8,"error":"Stat 'staff' is not an aggregated stat"}}"#,
    ),
];

/// Steam's answers to two checks of Tidal Mill Inn's records.
fn tti_checked_twice() -> Vec<(&'static str, &'static str)> {
    let once = iter::once(("200 OK", INN_SCHEMA)).chain(TTI_UNAGGREGATED);
    once.clone().chain(once).collect()
}

#[test]
fn games_kits_records_are_checked_saying_what_steam_does_not_give() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(tti_checked_twice());
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let records = |by_hand: &str| {
        let path = files.path().join("achievements.json");
        fs::write(
            &path,
            format!(
                r#"[
                {{"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"}},
                {{"type": "stat", "id": "base:staff", "steam": "staff", "kind": "int"}},
                {{"type": "achievement", "id": "base:long_journey", "steam": "LONG_JOURNEY",
                  "name": "Rising Water", "description": "Bring the millpond up to its brim.",
                  "stat": "base:floors", "at": 2}},
                {{"type": "achievement", "id": "base:crafted_alone", "steam": "CRAFTED_ALONE",
                  "name": "Bushel by Bushel", "description": "{by_hand}"}}
            ]"#
            ),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    };

    let matching = records("Bag a sack of flour by hand.");
    let (matched, said, _) = steamship(
        &["achievements", "1000050", "--check", &matching],
        Some(home.path()),
        &set,
    );
    assert_eq!(matched, Some(0_i32), "{said}");
    assert!(said.contains("2 achievements and 2 stats\n"), "{said}");
    assert!(
        said.contains(
            "  unchecked the stats' types and the achievements' progress, which Steam's Web API \
             does not give\n"
        ) && said.contains("  steam     \u{2713} 0 of 2 stats aggregated\n"),
        "a record that does not say so is a stat Steam keeps no total of: {said}"
    );
    assert!(
        said.contains("Steam's achievements and stats match the file\n"),
        "{said}"
    );

    let drifted = records("Bag the flour yourself.");
    let (refused, _, why) = steamship(
        &["achievements", "1000050", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "CRAFTED_ALONE: the description is \"Bag a sack of flour by hand.\" on Steam and \"Bag \
             the flour yourself.\" in the file"
        ) && why.contains("1 of 2 achievements match: 1 differs\n"),
        "{why}"
    );
}

#[test]
fn records_split_over_several_files_are_checked_as_one_and_none_may_repeat() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(tti_checked_twice());
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let file = |name: &str, records: &str| {
        let path = files.path().join(name);
        fs::write(&path, records).unwrap();
        path.to_string_lossy().into_owned()
    };
    let stats = file(
        "stats.json",
        r#"[{"type": "stat", "steam": "floors"}, {"type": "stat", "steam": "staff"}]"#,
    );
    let building = file(
        "building.json",
        r#"[{"type": "achievement", "steam": "LONG_JOURNEY", "name": "Rising Water",
             "description": "Bring the millpond up to its brim."}]"#,
    );
    let by_hand = file(
        "by_hand.json",
        r#"[{"type": "achievement", "steam": "CRAFTED_ALONE", "name": "Bushel by Bushel",
             "description": "Bag a sack of flour by hand."}]"#,
    );

    for args in [
        vec![
            "achievements",
            "1000050",
            "--check",
            &stats,
            &building,
            &by_hand,
        ],
        vec![
            "achievements",
            "1000050",
            "--check",
            &stats,
            "--check",
            &building,
            "--check",
            &by_hand,
        ],
    ] {
        let (matched, said, _) = steamship(&args, Some(home.path()), &set);
        assert_eq!(matched, Some(0_i32), "{said}");
        assert!(
            said.contains("by_hand.json, 1 achievement and 0 stats\n")
                && said.contains("Steam's achievements and stats match the files"),
            "{said}"
        );
    }

    let (refused, _, why) = steamship(
        &[
            "achievements",
            "1000050",
            "--check",
            &by_hand,
            &stats,
            &by_hand,
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!("CRAFTED_ALONE is in both {by_hand} and {by_hand}")),
        "{why}"
    );
}

#[test]
fn an_achievements_file_that_is_not_one_or_names_another_app_is_refused_before_steam() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let broken = files.path().join("broken.json");
    fs::write(&broken, r#"{"achievements": [{"name": "A"}]}"#).unwrap();
    let other = achievements_file(files.path(), "x");
    for (file, why) in [
        (
            broken.to_string_lossy().into_owned(),
            "achievement 1 has no \"api_name\" text",
        ),
        (other, "is for app 480, not app 1000010"),
    ] {
        let (code, _, stderr) = steamship(
            &["achievements", "1000010", "--check", &file],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{stderr}");
        assert!(stderr.contains(why), "{why:?} in {stderr}");
    }
}

/// A rich presence file for `language`, its tokens given as VDF pairs.
fn presence_file(folder: &Path, language: &str, tokens: &str) -> String {
    let path = folder.join(format!("rich_presence_{language}.vdf"));
    fs::write(
        &path,
        format!(
            "\"lang\"\n{{\n\t\"Language\"\t\"{language}\"\n\t\"Tokens\"\n\t{{\n{tokens}\t}}\n}}\n"
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

/// A form's body as it was meant, its `+` and `%XX` escapes undone.
fn unformed(body: &str) -> String {
    let mut bytes = Vec::with_capacity(body.len());
    let mut rest = body.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        rest = after;
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let (hex, beyond) = rest.split_at(2);
                bytes.push(u8::from_str_radix(&String::from_utf8_lossy(hex), 16).unwrap());
                rest = beyond;
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn rich_presence_is_sent_whole_for_every_file_and_a_result_steam_refuses_is_a_failure() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let english = presence_file(
        files.path(),
        "english",
        "\t\t\"#menu\"\t\"At the menu\"\n\t\t\"#season_rainy\"\t\"Autumn\"\n\
         \t\t\"#running\"\t\"In {#season_%season%}\"\n",
    );
    let german = presence_file(files.path(), "german", "\t\t\"#menu\"\t\"Im Men\u{fc}\"\n");
    let (host, requests) = web_api(vec![
        ("200 OK\r\nX-eresult: 1", r#"{"response": {}}"#),
        (
            "200 OK\r\nX-eresult: 8\r\nX-error_message: Invalid token",
            r#"{"response": {}}"#,
        ),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let command = ["rich-presence", "480", &english, &german];

    let (code, stdout, stderr) = steamship(&command, Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  english   3 tokens, from "), "{stdout}");
    assert!(stdout.contains("  german    1 token, from "), "{stdout}");
    assert!(
        stdout.contains("app 480: rich presence replaced for english, german"),
        "{stdout}"
    );
    let request = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        request.starts_with("POST /IProductInfoService/SetRichPresenceLocalization/v1/ ")
            && request
                .to_ascii_lowercase()
                .contains(&format!("x-webapi-key: {KEY}")),
        "{request}"
    );
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    assert_eq!(
        unformed(body),
        "input_json={\"appid\":480,\"languages\":[\
         {\"language\":\"english\",\"tokens\":[\
         {\"token\":\"#menu\",\"value\":\"At the menu\"},\
         {\"token\":\"#season_rainy\",\"value\":\"Autumn\"},\
         {\"token\":\"#running\",\"value\":\"In {#season_%season%}\"}]},\
         {\"language\":\"german\",\"tokens\":[{\"token\":\"#menu\",\"value\":\"Im Men\u{fc}\"}]}]}",
        "every token of every file, and no SteamID"
    );

    let (refused, _, why) = steamship(&command, Some(home.path()), &set);
    assert_eq!(refused, Some(1_i32), "{why}");
    assert!(why.contains("EResult 8, Invalid token"), "{why}");
}

#[test]
fn rich_presence_files_are_checked_before_steam_and_a_preview_sends_nothing() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 0 or 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let english = presence_file(files.path(), "english", "\t\t\"#menu\"\t\"At the menu\"\n");

    let (previewed, stdout, stderr) = steamship(
        &["rich-presence", "480", &english, "--preview"],
        Some(home.path()),
        &set,
    );
    assert_eq!(previewed, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  english   1 token, from "), "{stdout}");
    assert!(stdout.contains("nothing was sent"), "{stdout}");

    let unnamed = presence_file(
        files.path(),
        "french",
        "\t\t\"#running\"\t\"In {#season_%s%}\"\n",
    );
    let unfillable = presence_file(
        files.path(),
        "german",
        "\t\t\"#running\"\t\"Zu 50% fertig\"\n",
    );
    let again = files.path().join("again.vdf");
    let _: u64 = fs::copy(&english, &again).unwrap();
    let missing = files.path().join("missing.vdf");
    for (file, why) in [
        (unnamed, "no token starts #season_"),
        (
            unfillable,
            "#running: \"% fertig\" holds a % that starts no %field%",
        ),
        (
            again.to_string_lossy().into_owned(),
            "is a second file for english",
        ),
        (missing.to_string_lossy().into_owned(), "missing.vdf: "),
    ] {
        let (code, _, said) = steamship(
            &["rich-presence", "480", &english, &file],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{said}");
        assert!(said.contains(why), "{why:?} in {said}");
    }
}

/// `GetLeaderboardsForGame` answering with one leaderboard, shaped as Steam answered in a trial.
const LEADERBOARDS: &str = r#"{"response": {"result": 1, "leaderboards": [
    {"id": 7, "name": "gold", "entries": 12, "sortmethod": "Descending", "displaytype": "Numeric",
     "onlytrustedwrites": true, "onlyfriendsreads": false}
]}}"#;

/// `FindOrCreateLeaderboard` answering that it made `fastest_season`.
const MADE: &str = r#"{"result": {"result": 1, "leaderboard": {"leaderboardName": "fastest_season",
    "leaderBoardID": 8, "leaderBoardEntries": 0, "leaderBoardSortMethod": "Ascending",
    "leaderBoardDisplayType": "Seconds", "onlytrustedwrites": false, "onlyfriendsreads": true}}}"#;

/// A leaderboards file for app 480 with `gold` as Steam has it, or not, and `fastest_season`.
fn leaderboards_file(folder: &Path, gold_trusted: bool) -> String {
    let path = folder.join("leaderboards.json");
    fs::write(
        &path,
        format!(
            r#"{{"app": 480, "leaderboards": [
                {{"name": "gold", "sort": "descending", "display": "numeric",
                  "trusted_writes": {gold_trusted}}},
                {{"name": "fastest_season", "sort": "ascending", "display": "seconds",
                  "friends_only": true}}
            ]}}"#
        ),
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn leaderboards_are_listed_and_checked_and_those_missing_are_made_as_the_file_has_them() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", LEADERBOARDS),
        ("200 OK", LEADERBOARDS),
        ("200 OK", MADE),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

    let (code, listed, _) = steamship(&["leaderboards", "480"], Some(home.path()), &set);
    assert_eq!(code, Some(0_i32), "{listed}");
    assert!(
        listed.contains("  steam     \u{2713} 1 leaderboard on Steam\n"),
        "{listed}"
    );
    assert!(
        listed.contains("  gold      descending, numeric, 12 scores, trusted writes only\n"),
        "{listed}"
    );
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        asked.starts_with("GET /ISteamLeaderboards/GetLeaderboardsForGame/v2/?appid=480&"),
        "{asked}"
    );

    // Not made without --create, and a setting that differs is said with where to change it.
    let drifted = leaderboards_file(files.path(), false);
    let (refused, said, why) = steamship(
        &["leaderboards", "480", "--check", &drifted],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert!(
        why.contains("gold: trusted writes only is \"yes\" on Steam and \"no\" in the file")
            && why.contains("fastest_season: in the file, not on Steam; --create makes it"),
        "{why}"
    );
    assert!(said.contains("change them in Steamworks"), "{said}");
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());

    let matching = leaderboards_file(files.path(), true);
    let (made, making, stderr) = steamship(
        &["leaderboards", "480", "--check", &matching, "--create"],
        Some(home.path()),
        &set,
    );
    assert_eq!(made, Some(0_i32), "{making}{stderr}");
    assert!(
        making.contains("  steam     \u{2713} made fastest_season\n"),
        "{making}"
    );
    assert!(
        making.contains("Steam's leaderboards match the file"),
        "{making}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let creating = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        creating.starts_with("POST /ISteamLeaderboards/FindOrCreateLeaderboard/v2/ ")
            && creating.ends_with(
                "appid=480&name=fastest_season&sortmethod=Ascending&displaytype=Seconds\
                 &createifnotfound=true&onlytrustedwrites=false&onlyfriendsreads=true"
            ),
        "only the one Steam lacks, as the file has it: {creating}"
    );
}

/// `DeleteLeaderboard` answering that it deleted the one asked for, as Steam answered in a trial.
#[cfg(unix)]
const DELETED: &str = r#"{"result": {"result": 1}}"#;

/// A leaderboards file for app 1000 holding `fastest_season` alone, as [`MADE`] makes it.
#[cfg(unix)]
fn without_gold(folder: &Path) -> String {
    let path = folder.join("leaderboards.json");
    fs::write(
        &path,
        r#"{"app": 1000, "leaderboards": [
            {"name": "fastest_season", "sort": "ascending", "display": "seconds",
             "friends_only": true}
        ]}"#,
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
#[test]
fn a_leaderboard_the_file_no_longer_holds_is_deleted_only_while_the_app_is_unreleased() {
    let files = tempfile::tempdir().unwrap();
    let file = without_gold(files.path());
    let command = [
        "leaderboards",
        "1000",
        "--check",
        &file,
        "--create",
        "--delete-missing",
    ];
    let unreleased = faked_with(&released_as("prerelease"));
    saved_login(unreleased.path());
    let (host, requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", DELETED),
        ("200 OK", MADE),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let (code, said, stderr) = steamship(&command, Some(unreleased.path()), &set);
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    for line in [
        "unreleased, as Steam shows it (prerelease)",
        "  steam     \u{2713} deleted gold\n",
        "  steam     \u{2713} made fastest_season\n",
        "Steam's leaderboards match the file",
    ] {
        assert!(said.contains(line), "{line:?} in {said}");
    }
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let deleting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        deleting.starts_with("POST /ISteamLeaderboards/DeleteLeaderboard/v1/ ")
            && deleting.ends_with("appid=1000&name=gold"),
        "{deleting}"
    );

    let released = faked_with(&released_as("released"));
    saved_login(released.path());
    let (asked, _unasked) = web_api(vec![("200 OK", LEADERBOARDS)]);
    let (refused, kept, why) = steamship(
        &command,
        Some(released.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &asked),
        ],
    );
    assert_eq!(refused, Some(2_i32), "{kept}{why}");
    assert_eq!(
        failure(&why),
        "app 1000 is released, so deleting gold would take its players' scores with it: delete \
         it in Steamworks, under Stats & Achievements, Leaderboards, or put it back in the file"
    );
    assert!(!kept.contains("deleted"), "{kept}");

    let (refusing, _refused_requests) = web_api(vec![
        ("200 OK", LEADERBOARDS),
        ("200 OK", r#"{"result": {"result": 2}}"#),
    ]);
    let (failed, _, unsaid) = steamship(
        &command,
        Some(unreleased.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &refusing),
        ],
    );
    assert_eq!(failed, Some(1_i32), "{unsaid}");
    assert!(unsaid.contains("result 2"), "{unsaid}");
}

#[test]
fn a_dated_board_a_template_made_is_checked_against_it_and_never_made_or_deleted() {
    // No steamcmd is installed in the home: asking whether the app is released would fail.
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let records = files.path().join("boards.json");
    fs::write(
        &records,
        r#"[{"type": "leaderboard", "steam": "gold", "sort": "descending", "display": "numeric",
             "trusted_writes": true},
            {"type": "leaderboard", "steam": "DAILY", "sort": "descending", "display": "numeric",
             "period": "day", "details": 5}]"#,
    )
    .unwrap();
    let records = records.to_string_lossy().into_owned();
    let dated = r#"{"response": {"result": 1, "leaderboards": [
        {"id": 7, "name": "gold", "entries": 12, "sortmethod": "Descending",
         "displaytype": "Numeric", "onlytrustedwrites": true, "onlyfriendsreads": false},
        {"id": 9, "name": "DAILY_20261025", "entries": 3, "sortmethod": "Descending",
         "displaytype": "Numeric", "onlytrustedwrites": false, "onlyfriendsreads": false}
    ]}}"#;
    let (host, requests) = web_api(vec![("200 OK", dated)]);
    let (code, said, stderr) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &records,
            "--create",
            "--delete-missing",
        ],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!("{records}, 1 leaderboard, 1 dated template\n"))
            && said.contains("Steam's leaderboards match the file")
            && !said.contains("made")
            && !said.contains("deleted"),
        "{said}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    assert!(
        requests.recv_timeout(Duration::from_secs(1)).is_err(),
        "Steam is asked for the leaderboards alone"
    );
}

#[test]
fn delete_missing_with_nothing_to_delete_asks_steamcmd_nothing() {
    // No steamcmd is installed in the home: asking whether the app is released would fail.
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let file = leaderboards_file(files.path(), true);
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("200 OK", MADE)]);
    let (code, said, stderr) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &file,
            "--create",
            "--delete-missing",
        ],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains("Steam's leaderboards match the file") && !said.contains("deleted"),
        "{said}"
    );
}

#[test]
fn games_kits_leaderboard_records_are_checked_and_a_leaderboard_in_two_files_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS), ("200 OK", LEADERBOARDS)]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    let records = files.path().join("achievements.json");
    fs::write(
        &records,
        r#"[{"type": "stat", "id": "base:gold", "steam": "gold_banked"},
            {"type": "leaderboard", "id": "base:gold", "steam": "gold", "name": "Gold",
             "sort": "descending", "display": "numeric", "upload": "keep_best", "details": 1,
             "trusted_writes": true}]"#,
    )
    .unwrap();
    let records = records.to_string_lossy().into_owned();

    let (code, said, stderr) = steamship(
        &["leaderboards", "480", "--check", &records],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{said}{stderr}");
    assert!(
        said.contains(&format!("{records}, 1 leaderboard\n"))
            && said.contains("Steam's leaderboards match the file")
            && !said.contains("match the files"),
        "{said}"
    );

    let stats = files.path().join("stats.json");
    fs::write(&stats, r#"[{"type": "stat", "steam": "floors"}]"#).unwrap();
    let stats = stats.to_string_lossy().into_owned();
    let (both, matching, unsaid) = steamship(
        &[
            "leaderboards",
            "480",
            "--check",
            &records,
            "--check",
            &stats,
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(both, Some(0_i32), "{matching}{unsaid}");
    assert!(
        matching.contains(&format!("{stats}, 0 leaderboards\n"))
            && matching.contains("Steam's leaderboards match the files"),
        "{matching}"
    );

    let other = leaderboards_file(files.path(), true);
    let (refused, _, why) = steamship(
        &["leaderboards", "480", "--check", &records, &other],
        Some(home.path()),
        &set,
    );
    assert_eq!(refused, Some(2_i32), "{why}");
    assert!(
        why.contains(&format!("gold is in both {records} and {other}")),
        "{why}"
    );
}

#[test]
fn a_leaderboards_file_that_is_not_one_is_refused_before_steam_and_create_needs_check() {
    let home = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    // No stand-in is given: asking Steam would be a failure to reach it, exit 1, not 2.
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    let broken = files.path().join("broken.json");
    fs::write(
        &broken,
        r#"{"leaderboards": [{"name": "gold", "sort": "up"}]}"#,
    )
    .unwrap();
    let other = leaderboards_file(files.path(), true);
    for (file, why) in [
        (
            broken.to_string_lossy().into_owned(),
            "gold: \"sort\" is not ascending or descending",
        ),
        (other, "is for app 480, not app 1000010"),
    ] {
        let (code, _, stderr) = steamship(
            &["leaderboards", "1000010", "--check", &file, "--create"],
            Some(home.path()),
            &set,
        );
        assert_eq!(code, Some(2_i32), "{stderr}");
        assert!(stderr.contains(why), "{why:?} in {stderr}");
    }
    let (code, _, stderr) = steamship(
        &["leaderboards", "1000010", "--create"],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("there is no game in "), "{stderr}");
}

/// `GetAppBetas` as Steam answered for Falcon Ridge Overseer.
const DESCRIBED: &str = r#"{"response": {"betas": {
    "public": {"BuildID": 20000001, "Description": "Public default branch", "ReqPassword": 0},
    "testing": {"BuildID": 20000099, "Description": "test1", "ReqPassword": 1}
}, "result": 1, "message": ""}}"#;

/// `UpdateAppBranchDescription` as Steam answered it.
const UPDATED: &str =
    r#"{"response": {"result": 1, "message": "Successfully updated beta branch description"}}"#;

#[test]
fn a_branch_is_described_only_when_its_description_differs() {
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", DESCRIBED),
        ("200 OK", DESCRIBED),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];

    let (code, stdout, stderr) = steamship(
        &[
            "branch",
            "1000010",
            "testing",
            "--description",
            "0.3.0 (abc1234)",
        ],
        Some(home.path()),
        &set,
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  steam     \u{2713} described as \"test1\"\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("app 1000010: testing described as \"0.3.0 (abc1234)\""),
        "{stdout}"
    );
    drop(requests.recv_timeout(Duration::from_secs(10)).unwrap());
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.starts_with("POST /ISteamApps/UpdateAppBranchDescription/v1/ ")
            && setting.ends_with("appid=1000010&betakey=testing&description=0.3.0+(abc1234)"),
        "{setting}"
    );

    // Already so: nothing is sent, which the stand-in's next answer being a listing shows.
    let (same, said, _) = steamship(
        &["branch", "1000010", "testing", "--description", "test1"],
        Some(home.path()),
        &set,
    );
    assert_eq!(same, Some(0_i32), "{said}");
    assert!(said.contains("testing is already described so"), "{said}");

    let (missing, _, why) = steamship(
        &["branch", "1000010", "beta", "--description", "x"],
        Some(home.path()),
        &set,
    );
    assert_eq!(missing, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "app 1000010 has no branch \"beta\"; create it in Steamworks under SteamPipe, Builds \
             first"
        ),
        "{why}"
    );
}

#[test]
fn the_default_branch_is_described_in_steamworks_and_a_refusal_says_steams_reason() {
    let home = tempfile::tempdir().unwrap();
    // No stand-in for the first: the default branch is refused before Steam is asked.
    let (code, _, stderr) = steamship(
        &["branch", "1000010", "default", "--description", "x"],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        stderr.contains("the default branch's description is set in Steamworks"),
        "{stderr}"
    );

    let (host, _requests) = web_api(vec![
        ("200 OK", DESCRIBED),
        (
            "404 Not Found",
            r#"{"response": {"result": 2, "message": "Unable to find specificed betakey testing"}}"#,
        ),
    ]);
    let (refused, _, why) = steamship(
        &["branch", "1000010", "testing", "--description", "x"],
        Some(home.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!(refused, Some(1_i32), "{why}");
    assert!(
        why.contains("Steam answered HTTP 404: Unable to find specificed betakey testing"),
        "{why}"
    );
}

#[test]
fn a_500_about_an_app_the_key_holds_is_said_to_be_an_app_with_no_build_yet() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
        ("500 Internal Server Error", ""),
        ("200 OK", APPS),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    // As Steam answered for Nocturne's app while no build was live on its default branch.
    let (code, stdout, stderr) = steamship(&["builds", "1000030"], Some(home.path()), &set);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(stderr.contains("Steam answered HTTP 500"), "{stderr}");
    assert!(
        stderr.contains(
            "this key holds app 1000030, and Steam answers so while no build is live on its \
             default branch"
        ),
        "{stderr}"
    );
    // The same answer about an app the key does not hold is left as it came.
    let (other, _, alone) = steamship(&["builds", "480"], Some(home.path()), &set);
    assert_eq!(other, Some(1_i32), "{alone}");
    assert!(alone.contains("Steam answered HTTP 500"), "{alone}");
    assert!(!alone.contains("this key holds"), "{alone}");
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

#[test]
fn a_web_api_key_that_is_not_one_is_refused_without_repeating_it() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["builds", "1000010"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", "not_a_key_1234")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert_eq!(
        failure(&stderr),
        "STEAMSHIP_WEB_API_KEY does not hold a publisher Web API key, which is 32 hexadecimal \
         digits"
    );
    assert!(!format!("{stdout}{stderr}").contains("not_a_key"));
}

#[cfg(unix)]
#[test]
fn promote_sets_the_default_branch_live_only_while_steam_shows_the_app_unreleased() {
    let (_project, script) = project(false);
    let script = script.to_str().unwrap();
    let unreleased = faked_with(&released_as("prerelease"));
    saved_login(unreleased.path());
    let (host, requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (code, stdout, stderr) = steamship(
        &["promote", script, "--build", "1234", "--branch", "public"],
        Some(unreleased.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &host),
        ],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  app       1000\n")
            && stdout.contains("  branch    default\n")
            && stdout.contains("unreleased, as Steam shows it (prerelease)")
            && stdout.contains("app 1000: BuildID 1234 live on default"),
        "{stdout}"
    );
    let setting = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        setting.ends_with("appid=1000&buildid=1234&betakey=public"),
        "{setting}"
    );

    // In CI, Steam is asked as the account the login handed over is for, which no home remembers.
    let handed_over = faked_with(&released_as("unavailable"));
    let (ci_host, _ci_requests) = web_api(vec![("200 OK", SET_LIVE)]);
    let (from_ci, ci_said, ci_why) = steamship(
        &["promote", script, "--build", "1234", "--branch", "default"],
        Some(handed_over.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_LOGIN", &packed_login()),
            (STAND_IN, &ci_host),
        ],
    );
    assert_eq!(from_ci, Some(0_i32), "{ci_said}{ci_why}");
    assert!(
        ci_said.contains("app 1000: BuildID 1234 live on default"),
        "{ci_said}"
    );

    let released = faked_with(&released_as("released"));
    saved_login(released.path());
    // A stand-in that answers nothing: a request would fail with 1, not be refused with 2.
    let (silent, _unasked) = web_api::<&str>(vec![]);
    let (refused, said, why) = steamship(
        &["promote", script, "--build", "1234", "--branch", "default"],
        Some(released.path()),
        &[
            ("STEAMSHIP_WEB_API_KEY", KEY),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            (STAND_IN, &silent),
        ],
    );
    assert_eq!(refused, Some(2_i32), "{said}{why}");
    assert_eq!(
        failure(&why),
        "app 1000 is released, so its default branch is set live in Steamworks, where Valve asks \
         for a Steam Mobile confirmation"
    );
}

#[test]
fn an_app_that_is_neither_an_id_nor_a_script_is_refused_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (zero, _, said) = steamship(&["builds", "0"], Some(home.path()), &[]);
    assert_eq!(
        (zero, failure(&said)),
        (Some(2_i32), "0 is not an app's ID")
    );
    let (missing, _, stderr) = steamship(&["builds", "no_such_app.vdf"], Some(home.path()), &[]);
    assert_eq!(missing, Some(2_i32));
    assert_eq!(failure(&stderr), "refused: no_such_app.vdf: does not exist");
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
    run::spawn(
        run::command("kill")
            .args([which, pid])
            .stderr(Stdio::null()),
    )
    .and_then(|mut kill| kill.wait())
    .unwrap()
    .success()
}

/// SIGHUP, SIGINT and SIGTERM, the signals that end steamship from outside, as bits of the masks
/// Linux shows a process's signals in: signal n is bit n - 1.
#[cfg(target_os = "linux")]
const HANGUP: u64 = 0x1;
#[cfg(target_os = "linux")]
const INTERRUPT: u64 = 0x2;
#[cfg(target_os = "linux")]
const TERMINATE: u64 = 0x4000;
#[cfg(target_os = "linux")]
const ENDING: u64 = HANGUP | INTERRUPT | TERMINATE;

/// Which of the signals that end steamship the process `pid` catches, and which it ignores.
///
/// Looked at directly, because whether a signal sent too early meets steamship's handler or the
/// default is a race that a test only sending signals could lose either way.
#[cfg(target_os = "linux")]
fn ending_signals(pid: u32) -> (u64, u64) {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let mask = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok())
            .unwrap()
            & ENDING
    };
    (mask("SigCgt:"), mask("SigIgn:"))
}

/// steamship uploading through a steamcmd that runs until it is stopped, started through
/// `launcher` when there is one. Returned once steamcmd has started, with steamcmd's process id
/// and the folders the upload uses, which must outlive it.
#[cfg(unix)]
fn uploading(launcher: Option<&str>) -> (Child, String, tempfile::TempDir, tempfile::TempDir) {
    let home = faked_with("#!/bin/sh\necho $$ > \"$HOME/steamcmd.pid\"\nexec sleep 60\n");
    let (project, script) = project(true);
    let steamship = env!("CARGO_BIN_EXE_steamship");
    let started = run::spawn(
        run::command(launcher.unwrap_or(steamship))
            .args(launcher.map(|_| steamship))
            .envs(common::store::environment())
            .args(["upload", script.to_str().unwrap(), "--version", "1.4.0"])
            .args(["--account", "build_bot"])
            .env("STEAMSHIP_HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .unwrap();
    let pid_file = home.path().join("steamcmd.pid");
    assert!(
        soon(|| fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n'))),
        "steamcmd never started"
    );
    let steamcmd = fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    (started, steamcmd, home, project)
}

/// The signal that ended `steamship`, once it has ended.
#[cfg(unix)]
fn ending(steamship: &mut Child) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;

    let mut ended = None;
    assert!(
        soon(|| {
            ended = steamship.try_wait().unwrap();
            ended.is_some()
        }),
        "steamship went on"
    );
    ended?.signal()
}

#[cfg(unix)]
#[test]
fn ctrl_c_during_an_upload_ends_steamcmd_with_steamship() {
    let (mut steamship, steamcmd, _home, _project) = uploading(None);
    #[cfg(target_os = "linux")]
    assert!(
        soon(|| ending_signals(steamship.id()) == (ENDING, 0_u64)),
        "steamship catches every signal that ends it"
    );
    assert!(signal("-INT", &steamship.id().to_string()));
    assert_eq!(ending(&mut steamship), Some(2_i32), "ended by Ctrl+C");
    assert!(
        soon(|| !signal("-0", &steamcmd)),
        "steamcmd outlived steamship"
    );
}

#[cfg(unix)]
#[test]
fn a_signal_steamship_was_started_to_ignore_stays_ignored_and_the_others_still_end_it() {
    let (mut steamship, steamcmd, _home, _project) = uploading(Some("nohup"));
    #[cfg(target_os = "linux")]
    assert!(
        soon(|| ending_signals(steamship.id()) == (INTERRUPT | TERMINATE, HANGUP)),
        "steamship catches the signals that end it but the one nohup ignores"
    );
    let pid = steamship.id().to_string();
    assert!(signal("-HUP", &pid) && signal("-TERM", &pid));
    assert_eq!(
        ending(&mut steamship),
        Some(15_i32),
        "the terminal closing went unheeded, and kill ended it"
    );
    assert!(
        soon(|| !signal("-0", &steamcmd)),
        "steamcmd outlived steamship"
    );
}

/// A steamcmd that prints app 1000's info as steamcmd 1788292693 printed Falcon Ridge
/// Manager's, trimmed, with the executable `executable`, keeping what it was started with in
/// `args`.
#[cfg(unix)]
fn app_info(executable: &str) -> String {
    format!(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo 'Logging in using cached credentials.'\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
         printf '\"1000\"\\n{{\\n'\n\
         printf '\\t\"common\"\\n\\t{{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n\\t}}\\n'\n\
         printf '\\t\"config\"\\n\\t{{\\n\\t\\t\"installdir\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"launch\"\\n\\t\\t{{\\n\\t\\t\\t\"0\"\\n\\t\\t\\t{{\\n'\n\
         printf '\\t\\t\\t\\t\"executable\"\\t\\t\"{executable}\"\\n\\t\\t\\t}}\\n\\t\\t}}\\n\\t}}\\n'\n\
         printf '\\t\"depots\"\\n\\t{{\\n\\t\\t\"1001\"\\n\\t\\t{{\\n'\n\
         printf '\\t\\t\\t\"manifests\"\\n\\t\\t\\t{{\\n\\t\\t\\t}}\\n\\t\\t}}\\n\\t}}\\n}}\\n'\n\
         echo 'Unloading Steam API...OK'\n"
    )
}

#[cfg(unix)]
#[test]
fn settings_are_shown_saved_and_checked_leaving_steams_bookkeeping_out() {
    let home = faked_with(&app_info("spacewar"));
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];

    let (code, stdout, stderr) = steamship(&["settings", "1000"], Some(home.path()), &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"1000\"\n{\n\t\"common\"\n\t{\n\t\t\"name\"\t\t\"Spacewar\"\n\t}\n")
            && stdout.contains("\t\t\t\t\"executable\"\t\t\"spacewar\"\n")
            && stdout.contains("\t\"depots\"\n\t{\n\t\t\"1001\"\n\t\t{\n\t\t}\n\t}\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("ReleaseState") && !stdout.contains("manifests"));
    assert!(
        stdout.contains("  cloud     Steam Cloud not set up in Steamworks\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "  input     no Default Controller Configuration in Steamworks, and no touch default"
        ),
        "{stdout}"
    );
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        args.contains("+login build_bot +app_info_update 1 +app_info_print 1000 +quit"),
        "{args}"
    );

    let kept = files.path().join("settings.vdf");
    let kept_text = kept.to_string_lossy().into_owned();
    let (saved, said, _) = steamship(
        &["settings", "1000", "--save", &kept_text],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(saved, Some(0_i32), "{said}");
    assert!(said.contains("app 1000: settings saved to "), "{said}");
    let (matched, checked, _) = steamship(
        &["settings", "1000", "--check", &kept_text],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(matched, Some(0_i32), "{checked}");
    assert!(
        checked.contains("Steam's settings match the file"),
        "{checked}"
    );

    // Someone changed the executable in Steamworks.
    let changed = faked_with(&app_info("spacewar.x86_64"));
    saved_login(changed.path());
    let (drifted, shown, why) = steamship(
        &["settings", "1000", "--check", &kept_text],
        Some(changed.path()),
        &quiet,
    );
    assert_eq!(drifted, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains(
            "config/launch/0/executable is \"spacewar.x86_64\" on Steam and \"spacewar\" in the \
             file"
        ),
        "{why}"
    );
    assert!(shown.contains("settings --save` the file again"), "{shown}");
}

#[cfg(unix)]
#[test]
fn settings_in_ci_are_asked_for_as_the_account_handed_over() {
    // No login saved and no account remembered: only what `steamship ci` packed.
    let home = faked_with(&app_info("spacewar"));
    let (code, stdout, stderr) = steamship(
        &["settings", "1000"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains("+login build_bot "), "{args}");
    assert!(steamcmd::saved_login(home.path(), Platform::THIS).exists());
}

#[cfg(unix)]
#[test]
fn a_snapshot_for_another_app_is_refused_before_steamcmd_and_an_app_steam_hides_is_a_failure() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         printf '\"1000\"\\n{\\n}\\n'\n",
    );
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let other = files.path().join("other.vdf");
    fs::write(&other, "\"480\"\n{\n}\n").unwrap();
    let (code, _, stderr) = steamship(
        &["settings", "1000", "--check", &other.to_string_lossy()],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("is for app 480, not app 1000"), "{stderr}");
    assert!(!home.path().join("args").exists(), "steamcmd was not run");

    let (hidden, _, why) = steamship(&["settings", "1000"], Some(home.path()), &quiet);
    assert_eq!(hidden, Some(1_i32), "{why}");
    assert!(
        why.contains("Steam shows the build account no settings for app 1000"),
        "{why}"
    );
}

/// A steamcmd that wraps as steamcmd 1788292693 did an executable importing `GetModuleHandleA`:
/// it writes the wrapped one where it was told and says so, keeping what it was started with in
/// `args`.
#[cfg(unix)]
const WRAPS: &str = "#!/bin/sh\n\
    echo \"$*\" > \"$HOME/args\"\n\
    while [ $# -gt 0 ] && [ \"$1\" != +drm_wrap ]; do shift; done\n\
    [ $# -ge 4 ] || exit 1\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    printf \"$STEAMSHIP_FAKE_WRAPPED\" > \"$4\"\n\
    echo \"DRM wrap completed; output is in $4\"\n";

/// A steamcmd whose DRM wrap Valve's tool refuses, as it refused Falcon Ridge Overseer's Godot
/// executable.
#[cfg(unix)]
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

#[cfg(unix)]
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
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZwrapped");
}

#[cfg(unix)]
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
    assert_eq!(fs::read_to_string(&game).unwrap(), "MZplain", "left alone");
    assert_eq!(
        fs::read_dir(files.path()).unwrap().count(),
        1,
        "the empty file is not left beside it"
    );
}

#[cfg(unix)]
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
             currently support adding imports; Unable to add import of \
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

/// The start of a PNG `width` by `height`: its signature, header and end, which is all that is
/// read of it.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(b"\x00\x00\x00\x00IEND\x00\x00\x00\x00");
    png
}

#[test]
fn artwork_is_checked_against_valves_sizes_with_nothing_sent() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("header_capsule.png"),
        png_header(920, 430),
    )
    .unwrap();
    fs::write(
        folder.path().join("small_capsule.png"),
        png_header(462, 174),
    )
    .unwrap();
    fs::write(folder.path().join("notes.txt"), "left alone").unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  header_capsule.png header capsule, 920x430 PNG\n")
            && stdout.contains("  small_capsule.png  small capsule, 462x174 PNG\n")
            && stdout.contains("the artwork is as Steamworks takes it"),
        "lined up however long the names run: {stdout}"
    );

    fs::write(
        folder.path().join("library_hero.png"),
        png_header(1920, 620),
    )
    .unwrap();
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("library_hero.png: the library hero must be 3840x1240, and it is 1920x620"),
        "{why}"
    );

    let empty = tempfile::tempdir().unwrap();
    let (none, _, said) = steamship(&["assets", &empty.path().to_string_lossy()], None, &quiet);
    assert_eq!(none, Some(2_i32), "{said}");
    assert!(
        said.contains("holds no PNG, JPEG, icon or zip, nor a folder for a language with one"),
        "{said}"
    );
}

#[test]
fn a_store_folder_is_checked_with_its_folders_for_each_language_and_the_rest_named() {
    let folder = tempfile::tempdir().unwrap();
    for inner in ["de", "pt_BR", "achievements"] {
        fs::create_dir_all(folder.path().join(inner)).unwrap();
    }
    fs::write(
        folder.path().join("de/header_capsule.png"),
        png_header(920, 430),
    )
    .unwrap();
    fs::write(
        folder.path().join("pt_BR/small_capsule.png"),
        png_header(462, 174),
    )
    .unwrap();
    fs::write(
        folder.path().join("achievements/FIRST_POUR.png"),
        png_header(64, 64),
    )
    .unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  skipped   achievements, named for no language\n")
            && stdout
                .contains("  de/header_capsule.png   header capsule for german, 920x430 PNG\n")
            && stdout
                .contains("  pt_BR/small_capsule.png small capsule for brazilian, 462x174 PNG\n"),
        "{stdout}"
    );

    fs::write(
        folder.path().join("de/small_capsule.png"),
        png_header(460, 174),
    )
    .unwrap();
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("de/small_capsule.png: the small capsule must be 462x174, and it is 460x174"),
        "{why}"
    );
}

#[test]
fn the_mac_and_linux_icons_are_checked_with_every_wrong_file_in_the_zip_named() {
    let folder = tempfile::tempdir().unwrap();
    let zipped = |entries: &[(&str, Vec<u8>)]| {
        let mut zip = ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, contents) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        fs::write(
            folder.path().join("linux_icons.zip"),
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
    };
    zipped(&[
        ("icons/16.png", png_header(16, 16)),
        ("icons/32.png", png_header(32, 32)),
    ]);
    // An Apple icon of one 128x128 image, with a byte of it.
    fs::write(
        folder.path().join("mac_icon.icns"),
        b"icns\x00\x00\x00\x11it32\x00\x00\x00\x09\x00",
    )
    .unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  linux_icons.zip Linux icons, 2 PNGs from 16x16 to 32x32\n")
            && stdout.contains("  mac_icon.icns   Mac icon, 128x128 Apple icon\n"),
        "{stdout}"
    );

    zipped(&[
        ("icons/16.png", png_header(16, 16)),
        ("icons/32.png", png_header(32, 30)),
        ("icons/notes.txt", b"drawn by hand".to_vec()),
    ]);
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("linux_icons.zip: the Linux icons must be square, and icons/32.png is 32x30")
            && why.contains(
                "linux_icons.zip: the Linux icons must be PNGs, and icons/notes.txt is not a PNG, \
             a JPEG, a Windows icon or an Apple icon"
            ),
        "{why}"
    );
}

/// A game laid out as games-kit lays one out, with `toml` as its `steamship.toml` when given,
/// committed and pushed to an origin of its own beside it; and the game's folder.
fn pushed_game(toml: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("inn");
    let put = |relative: &str, contents: &[u8]| {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    };
    put(
        "project.godot",
        b"[application]\nconfig/name=\"Inn\"\nconfig/version=\"0.3.1\"\n",
    );
    put(
        "steam/app_build.vdf",
        br#""AppBuild" { "AppID" "1000050" "ContentRoot" "../export/" "SetLive" "testing" "Depots" { "1000051" "depot.vdf" } }"#,
    );
    put(
        "steam/depot.vdf",
        br#""DepotBuild" { "DepotID" "1000051" "FileMapping" { "LocalPath" "*" "DepotPath" "." "Recursive" "1" } }"#,
    );
    put(
        "steam/settings.vdf",
        b"\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\t}\n}\n",
    );
    put(
        "steam/rich_presence_english.vdf",
        b"\"lang\"\n{\n\t\"Language\"\t\"english\"\n\t\"Tokens\"\n\t{\n\t\t\"#menu\"\t\"At the menu\"\n\t}\n}\n",
    );
    put("mods/base/manifest.json", b"{}");
    put("mods/base/data/achievements.json", INN_RECORDS.as_bytes());
    put("mods/base/data/options.json", br#"{"volume": 1}"#);
    for (icon, size, shade) in [
        ("five_levels", 256, 200),
        ("first_anchor", 256, 120),
        ("first_anchor_locked", 64, 60),
    ] {
        put(
            &format!("mods/base/icons/{icon}.png"),
            &icon_png(size, shade),
        );
    }
    put("game/locale/de.po", INN_GERMAN.as_bytes());
    put(
        "modding/example/manifest.json",
        br#"{"id": "example", "name": "Example Harbour", "version": "1.0.0", "description": "Every feature a mod has."}"#,
    );
    put("modding/example/preview.png", &icon_png(256, 90));
    put(".gitignore", b"export/\n");
    put("export/game.txt", b"[game]\n\tbuilt = before\n");
    if let Some(toml) = toml {
        put("steamship.toml", toml.as_bytes());
    }
    git(
        temp.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            "origin.git",
        ],
    );
    git(&root, &["init", "--quiet", "--initial-branch=main"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            &temp.path().join("origin.git").to_string_lossy(),
        ],
    );
    pushed(&root, "The game");
    (temp, root)
}

/// Commits everything in the game at `root` and pushes it to its origin.
fn pushed(root: &Path, message: &str) {
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "--allow-empty", "-m", message]);
    git(root, &["push", "--quiet", "origin", "main"]);
}

/// `steamship` with `args` in the game at `root`, with its own home.
fn in_game(root: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    in_game_with(root, args, &[])
}

/// [`in_game`] with `variables` set too.
fn in_game_with(
    root: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let home = root.parent().unwrap().join("home");
    fs::create_dir_all(&home).unwrap();
    let mut set = vec![("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    set.extend_from_slice(variables);
    steamship_in(root, args, Some(&home), &set)
}

/// The leaderboard [`LEADERBOARDS`] answers with, as a games-kit record.
const INN_BOARDS: &str = r#"[{"type": "leaderboard", "id": "base:gold", "steam": "gold",
    "name": "Gold", "sort": "descending", "display": "numeric", "upload": "keep_best",
    "details": 1, "trusted_writes": true}]"#;

/// A stand-in for the Web API refusing each of `count` requests, for a command whose part before
/// Steam is what is tested.
fn refusing_web_api(count: usize) -> String {
    web_api(vec![("403 Forbidden", "{}"); count]).0
}

#[test]
fn a_game_writes_its_text_in_every_language_and_then_finds_it_unchanged() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    let english = fs::read(steam.join("rich_presence_english.vdf")).unwrap();
    fs::write(
        steam.join("workshop_english.json"),
        r#"{"title": "Harbours", "description": "Made by players."}"#,
    )
    .unwrap();
    fs::write(steam.join("rich_presence_french.vdf"), "left over").unwrap();
    fs::write(
        steam.join("store_english.json"),
        r#"{"blocks": [{"kind": "short", "text": "An inn."}]}"#,
    )
    .unwrap();

    let (untranslated, _, refusals) = in_game(&root, &["text"]);
    assert_eq!(untranslated, Some(2_i32), "{refusals}");
    for line in [
        "de.po does not translate \"Harbours\", under msgctxt \"steam workshop\"",
        "de.po does not translate \"At the menu\", under msgctxt \"rich presence\"",
    ] {
        assert!(refusals.contains(line), "{line:?} in {refusals}");
    }
    assert!(!steam.join("workshop").exists(), "nothing is written");

    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"H\u{e4}fen\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\n\
         msgstr \"Von Spielern gemacht.\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (stale, said, missing) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{said}");
    for line in [
        "rich_presence_german.vdf is missing",
        "workshop/english.json is missing",
        "workshop/german.json is missing",
        "rich_presence_french.vdf is left over",
    ] {
        assert!(missing.contains(line), "{line:?} in {missing}");
    }

    let (code, wrote, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    let told = format!("{wrote}{why}");
    for line in [
        "  wrote     rich_presence_german.vdf\n",
        "  wrote     workshop/german.json\n",
        "  removed   rich_presence_french.vdf\n",
        " holds the 3 files of the game's text in 2 languages\n",
        "store_english.json names no itemid, the store page's own, so no store upload is written",
    ] {
        assert!(told.contains(line), "{line:?} in {told}");
    }
    assert_eq!(
        wrote
            .lines()
            .filter(|line| line.starts_with("  next"))
            .collect::<Vec<_>>(),
        [
            "  next      enter each file in workshop on the Workshop's Steamworks page, one \
             language to a save",
            "  next      steamship rich-presence sends the rich presence files",
        ]
    );
    assert_eq!(
        fs::read_to_string(steam.join("rich_presence_german.vdf")).unwrap(),
        "\"lang\"\n{\n\t\"Language\"\t\t\"german\"\n\t\"Tokens\"\n\t{\n\
         \t\t\"#menu\"\t\t\"Im Men\u{fc}\"\n\t}\n}\n"
    );
    assert_eq!(
        fs::read_to_string(steam.join("workshop/german.json")).unwrap(),
        "{\n  \"language\": \"german\",\n  \"title\": \"H\u{e4}fen\",\n  \
         \"description\": \"Von Spielern gemacht.\"\n}\n"
    );
    assert!(!steam.join("rich_presence_french.vdf").exists());
    assert_eq!(
        fs::read(steam.join("rich_presence_english.vdf")).unwrap(),
        english,
        "the English kept by hand is left as it is"
    );

    let (same, unchanged, failures) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{failures}");
    assert!(
        unchanged.contains("are as the files make them"),
        "{unchanged}"
    );
}

#[test]
fn a_game_s_rich_presence_names_each_of_its_records_of_a_type_in_every_language() {
    let (_temp, root) = pushed_game(None);
    fs::write(
        root.join("steam/presence_english.json"),
        r##"{"#Status": "{#Bishop_%bishop%} plays", "#Bishop_": {"records": "bishop"}}"##,
    )
    .unwrap();
    fs::write(
        root.join("mods/base/data/bishops.json"),
        r#"[{"type": "bishop", "id": "base:alder", "name": "The Alder Witch"}]"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"{#Bishop_{bishop}} plays\"\n\
         msgstr \"{#Bishop_{bishop}} spielt\"\n\n\
         msgctxt \"bishop.name\"\nmsgid \"base:alder\"\nmsgstr \"Die Erlenhexe\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (code, said, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(
        said.contains("  wrote     rich_presence_english.vdf\n"),
        "{said}"
    );
    assert!(
        fs::read_to_string(root.join("steam/rich_presence_english.vdf"))
            .unwrap()
            .contains("\t\t\"#Bishop_alder\"\t\t\"The Alder Witch\"\n")
    );
    assert_eq!(
        fs::read_to_string(root.join("steam/rich_presence_german.vdf")).unwrap(),
        "\"lang\"\n{\n\t\"Language\"\t\t\"german\"\n\t\"Tokens\"\n\t{\n\
         \t\t\"#Bishop_alder\"\t\t\"Die Erlenhexe\"\n\
         \t\t\"#Status\"\t\t\"{#Bishop_%bishop%} spielt\"\n\t}\n}\n"
    );

    fs::write(root.join("mods/base/data/bishops.json"), "[").unwrap();
    let (unread, _, refusal) = in_game(&root, &["text"]);
    assert_eq!(unread, Some(2_i32));
    assert!(refusal.contains("bishops.json: "), "{refusal}");
}

/// App details for app 1000050 showing `minimum` and `recommended` as its Windows operating
/// system, as Steam's store marks them up.
fn store_details(minimum: &str, recommended: &str) -> String {
    let list = |title: &str, line: &str| {
        format!(
            "<strong>{title}:</strong><br><ul class=\\\"bb_ul\\\"><li><strong>OS:</strong> \
             {line}<br></li></ul>"
        )
    };
    format!(
        r#"{{"1000050":{{"success":true,"data":{{"pc_requirements":{{"minimum":"{}","recommended":"{}"}},"mac_requirements":[],"linux_requirements":[]}}}}}}"#,
        list("Minimum", minimum),
        list("Recommended", recommended)
    )
}

/// [`pushed_game`] giving a minimum and a recommended Windows operating system and a recommended
/// amount of memory, each translated into German.
fn game_with_requirements() -> (tempfile::TempDir, PathBuf) {
    let (temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::write(
        steam.join("store_english.json"),
        r#"{"itemid": 1000090, "blocks": [
            {"kind": "short", "text": "An inn."},
            {"kind": "field", "field": "app[content][sysreqs][windows][min][osversion]",
             "text": "Windows 10"},
            {"kind": "field", "field": "app[content][sysreqs][windows][rec][osversion]",
             "text": "Windows 11"},
            {"kind": "field", "field": "app[content][sysreqs][windows][rec][memory][amount]",
             "text": "8"}
        ]}"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"store page\"\nmsgid \"An inn.\"\nmsgstr \"Gasthaus.\"\n\n\
         msgctxt \"store page\"\nmsgid \"Windows 10\"\nmsgstr \"Windows 10\"\n\n\
         msgctxt \"store page\"\nmsgid \"Windows 11\"\nmsgstr \"Windows 11, 64 Bit\"\n\n\
         msgctxt \"store page\"\nmsgid \"8\"\nmsgstr \"8\"\n",
    );
    fs::write(&locale, german).unwrap();
    (temp, root)
}

#[test]
fn recommended_requirements_leave_the_upload_for_the_basic_info_sheet() {
    let (_temp, root) = game_with_requirements();
    let steam = root.join("steam");
    let (written, wrote, unwritten) = in_game(&root, &["text"]);
    assert_eq!(written, Some(0_i32), "{unwritten}");
    assert_eq!(
        fs::read_to_string(steam.join("basic_info.json")).unwrap(),
        "{\n  \"app[content][sysreqs][windows][rec][osversion][english]\": \"Windows 11\",\n  \
         \"app[content][sysreqs][windows][rec][memory][amount][english]\": \"8\",\n  \
         \"app[content][sysreqs][windows][rec][osversion][german]\": \"Windows 11, 64 Bit\",\n  \
         \"app[content][sysreqs][windows][rec][memory][amount][german]\": \"8\"\n}\n"
    );
    let upload = fs::read_to_string(steam.join("store/german.json")).unwrap();
    assert!(
        upload.contains("\"app[content][sysreqs][windows][min][osversion]\": \"Windows 10\"")
            && !upload.contains("[rec]"),
        "{upload}"
    );
    assert!(
        wrote.contains(
            "  next      type each language's recommended system requirements in basic_info.json \
             on the store page's Basic Info form, as the upload leaves them out; once the page \
             is public, `steamship text --steam` finds any missed\n"
        ),
        "{wrote}"
    );

    let english = steam.join("store_english.json");
    let without = fs::read_to_string(&english)
        .unwrap()
        .replace("[rec][osversion]", "[min][processor]")
        .replace("[rec][memory][amount]", "[min][graphics]");
    fs::write(&english, without).unwrap();
    let (stale, _, left) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{left}");
    assert!(left.contains("basic_info.json is left over"), "{left}");
    let (removed, said, why) = in_game(&root, &["text"]);
    assert_eq!(removed, Some(0_i32), "{why}");
    assert!(said.contains("  removed   basic_info.json\n"), "{said}");
    assert!(!steam.join("basic_info.json").exists());
}

#[test]
fn screenshots_alt_text_goes_to_its_own_sheet_and_an_about_image_s_into_the_upload() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    let english = steam.join("store_english.json");
    fs::write(
        &english,
        r#"{"itemid": 1000090, "screenshots": ["The inn at dusk."], "extras": ["about_hearth"],
            "blocks": [
            {"kind": "short", "text": "An inn."},
            {"kind": "image", "image": "about_hearth", "alt": "A fire in the hearth."}
        ]}"#,
    )
    .unwrap();
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"store page\"\nmsgid \"An inn.\"\nmsgstr \"Gasthaus.\"\n\n\
         msgctxt \"store page\"\nmsgid \"The inn at dusk.\"\nmsgstr \"Das Gasthaus am Abend.\"\n\n\
         msgctxt \"store page\"\nmsgid \"A fire in the hearth.\"\nmsgstr \"Das Feuer im Herd.\"\n",
    );
    fs::write(&locale, german).unwrap();
    let (written, wrote, unwritten) = in_game(&root, &["text"]);
    assert_eq!(written, Some(0_i32), "{unwritten}");
    assert_eq!(
        fs::read_to_string(steam.join("screenshots.json")).unwrap(),
        "{\n  \"english 1\": \"The inn at dusk.\",\n  \
         \"german 1\": \"Das Gasthaus am Abend.\"\n}\n"
    );
    assert!(
        fs::read_to_string(steam.join("store/german.json"))
            .unwrap()
            .contains("\"app[assets][extra][0][alt_text]\": \"Das Feuer im Herd.\""),
    );
    assert!(
        wrote.contains(
            "  next      type each language's screenshot alt text in screenshots.json into the \
             screenshots' Edit Accessibility Text panel on the Store Page tab, one language at a \
             time, as the upload carries none\n"
        ),
        "{wrote}"
    );

    let without = fs::read_to_string(&english)
        .unwrap()
        .replace(r#""screenshots": ["The inn at dusk."], "#, "");
    fs::write(&english, without).unwrap();
    let (stale, _, left) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(stale, Some(2_i32), "{left}");
    assert!(left.contains("screenshots.json is left over"), "{left}");
    let (removed, said, why) = in_game(&root, &["text"]);
    assert_eq!(removed, Some(0_i32), "{why}");
    assert!(said.contains("  removed   screenshots.json\n"), "{said}");
}

/// [`store_details`] with `padding` bytes before the requirements.
fn padded_details(recommended: &str, padding: usize) -> String {
    store_details("Windows 10", recommended).replacen(
        "\"data\":{",
        &format!("\"data\":{{\"padding\":\"{}\",", "x".repeat(padding)),
        1,
    )
}

#[test]
fn a_store_answer_of_megabytes_is_read_whole_and_one_past_eight_is_refused() {
    let (_temp, root) = game_with_requirements();
    let (host, _) = web_api(vec![
        ("200 OK", padded_details("Windows 11", 2 << 20)),
        ("200 OK", padded_details("Windows 11, 64 Bit", 2 << 20)),
    ]);
    let (read, said, why) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(read, Some(0_i32), "{said}{why}");

    let (huge, _) = web_api(vec![("200 OK", padded_details("Windows 11", 9 << 20))]);
    let (refused, _, failure) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &huge)]);
    assert_eq!(refused, Some(1_i32), "{failure}");
    assert!(
        failure.contains("Steam's store's answer could not be read whole"),
        "{failure}"
    );
}

#[test]
fn the_store_page_is_read_in_every_language_and_each_requirement_it_lacks_named() {
    let (_temp, root) = game_with_requirements();
    let (host, requests) = web_api(vec![
        ("200 OK", store_details("Windows 10", "Windows 11")),
        ("200 OK", store_details("Windows 10", "Windows 11")),
    ]);
    let (lacking, said, why) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(lacking, Some(2_i32), "{said}{why}");
    let told = format!("{said}{why}");
    for line in [
        "german: the recommended windows osversion is not on the store page",
        "\"Windows 11, 64 Bit\"",
        "Steam rewords windows memory amount on the store page, so they are not compared",
        "type the recommended ones from basic_info.json on the store page's Basic Info form, \
         upload the files in store/ on its Localization tab for the minimum ones, and run it again",
    ] {
        assert!(told.contains(line), "{line:?} in {told}");
    }
    assert!(!told.contains("english: the"), "{told}");
    for language in ["english", "german"] {
        let asked = requests.recv().unwrap();
        assert!(
            asked.starts_with(&format!("GET /api/appdetails?appids=1000050&l={language} ")),
            "{asked}"
        );
    }

    let (fixed, _) = web_api(vec![
        ("200 OK", store_details("Windows 10", "Windows 11")),
        ("200 OK", store_details("Windows 10", "Windows 11, 64 Bit")),
    ]);
    let (shown, all, unshown) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &fixed)]);
    assert_eq!(shown, Some(0_i32), "{unshown}");
    assert!(
        all.contains(
            "app 1000050's store page shows the 4 system requirements of the files in 2 languages"
        ),
        "{all}"
    );
}

#[test]
fn a_store_page_steam_does_not_show_or_cannot_give_is_named() {
    let (_temp, root) = game_with_requirements();
    let (host, _) = web_api(vec![("200 OK", r#"{"1000050":{"success":false}}"#)]);
    let (private, _, hidden) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &host)]);
    assert_eq!(private, Some(2_i32), "{hidden}");
    assert!(
        hidden.contains(
            "Steam shows no public store page for app 1000050, so its system requirements can \
             be checked only on its Basic Info form"
        ),
        "{hidden}"
    );

    let (failing, _) = web_api(vec![("500 Internal Server Error", "")]);
    let (broken, _, failure) = in_game_with(&root, &["text", "--steam"], &[(STAND_IN, &failing)]);
    assert_eq!(broken, Some(1_i32), "{failure}");
    assert!(
        failure.contains("Steam's store answered HTTP 500"),
        "{failure}"
    );
}

#[test]
fn a_store_page_check_of_a_game_giving_no_requirements_is_refused() {
    let (_temp, root) = pushed_game(None);
    fs::write(
        root.join("steam/store_english.json"),
        r#"{"blocks": [{"kind": "short", "text": "An inn."}]}"#,
    )
    .unwrap();
    let (code, _, why) = in_game(&root, &["text", "--steam"]);
    assert_eq!(code, Some(2_i32), "{why}");
    assert!(
        why.contains(
            "store_english.json gives no system requirements, so there are none to look for on \
             the store page"
        ),
        "{why}"
    );
}

#[test]
fn a_file_of_steam_s_text_that_cannot_be_read_is_named() {
    let (_temp, root) = pushed_game(None);
    fs::create_dir_all(root.join("steam/workshop_english.json")).unwrap();
    let (code, _, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(1_i32), "{why}");
    assert!(why.contains("workshop_english.json: "), "{why}");

    fs::remove_dir(root.join("steam/workshop_english.json")).unwrap();
    fs::remove_file(root.join("steam/rich_presence_english.vdf")).unwrap();
    fs::write(root.join("steam/store_english.json"), "{}").unwrap();
    fs::write(
        root.join("steam/store"),
        "a file where the store's folder goes",
    )
    .unwrap();
    let (kept, _, refusal) = in_game(&root, &["text"]);
    assert_eq!(kept, Some(1_i32), "{refusal}");
    assert!(refusal.contains("store: "), "{refusal}");
}

#[test]
fn a_game_with_no_english_for_steam_s_text_is_told_so_and_has_nothing_written_or_removed() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::remove_file(steam.join("rich_presence_english.vdf")).unwrap();
    let elsewhere = [
        "store/english.json",
        "workshop/german.json",
        "ai_disclosure.json",
        "rich_presence_german.vdf",
    ];
    for name in elsewhere {
        let path = steam.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "written by the game's own tool").unwrap();
    }

    let (same, kept, why_not) = in_game(&root, &["text", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{kept}{why_not}");
    assert!(!why_not.contains("left over"), "{why_not}");
    let (code, said, why) = in_game(&root, &["text"]);
    assert_eq!(code, Some(0_i32), "{why}");
    let all = format!("{said}{why}");
    for line in [
        "there is no store_english.json, workshop_english.json, presence_english.json or \
         rich_presence_english.vdf beside the build scripts, so there is no English to write \
         from",
        " holds none of the game's text\n",
    ] {
        assert!(all.contains(line), "{line:?} in {all}");
    }
    assert!(!all.contains("  next"), "{all}");
    assert!(!all.contains("removed"), "{all}");
    for name in elsewhere {
        assert_eq!(
            fs::read_to_string(steam.join(name)).unwrap(),
            "written by the game's own tool",
            "{name}"
        );
    }
}

#[test]
fn a_game_writes_its_achievement_import_from_its_own_records_and_catalogues() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["achievements", "--write"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let out = root.join("steam").join("achievements");
    assert!(
        out.join("1000050-achievements-definitions.csv").is_file(),
        "{stdout}"
    );
    assert!(
        stdout.contains("achievements.json, 2 achievements and 1 stat"),
        "the records file alone of the base mod's data: {stdout}"
    );
    assert!(stdout.contains("de.po"), "{stdout}");

    let (same, unchanged, why_not) = in_game(&root, &["achievements", "--write", "--unchanged"]);
    assert_eq!(same, Some(0_i32), "{unchanged}{why_not}");

    let inner = root.join("mods").join("base");
    let (from_inside, said, why) = steamship_in(
        &inner,
        &["achievements", "--write", "--unchanged"],
        Some(&root.parent().unwrap().join("home")),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(
        from_inside,
        Some(0_i32),
        "found from a folder inside it: {said}{why}"
    );
}

#[test]
fn a_games_rich_presence_is_read_beside_its_scripts() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["rich-presence", "--preview"]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("  english   1 token, from "), "{stdout}");
    assert!(stdout.contains("nothing was sent"), "{stdout}");
}

#[test]
fn a_game_pinning_another_steamship_is_refused_by_every_command_run_in_it() {
    let (_temp, root) = pushed_game(Some("steamship = \"0.0.1\"\n"));
    for args in [
        ["builds"].as_slice(),
        ["check"].as_slice(),
        ["upload"].as_slice(),
        ["achievements", "--write"].as_slice(),
    ] {
        let (code, stdout, stderr) = in_game(&root, args);
        assert_eq!(code, Some(2_i32), "{args:?}: {stdout}{stderr}");
        assert!(
            failure(&stderr).ends_with(&format!(
                "pins steamship 0.0.1, and this is {}: install 0.0.1, or change the pin",
                env!("CARGO_PKG_VERSION")
            )),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn a_steamship_toml_saying_what_steamship_does_not_take_is_refused_naming_the_file() {
    let (_temp, root) = pushed_game(Some("[workshop]\nmod = \"examples\"\n"));
    let (code, stdout, stderr) = in_game(&root, &["builds"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    // Ends with, as macOS gives the child its temporary folder through /private.
    assert!(
        failure(&stderr).ends_with(&format!(
            "{}: `workshop.mod` is no setting steamship takes; it takes `workshop.mods`, \
             `workshop.items`, `workshop.footer`",
            Path::new("inn").join("steamship.toml").display()
        )),
        "{stderr}"
    );
}

#[test]
fn an_upload_in_a_game_ships_only_its_pushed_commit_against_its_settings_snapshot() {
    let (_temp, root) = pushed_game(None);
    let refused = |said: &str| {
        let (exit, stdout, stderr) = in_game(&root, &["upload"]);
        assert_eq!(exit, Some(2_i32), "{said}: {stdout}{stderr}");
        assert!(failure(&stderr).contains(said), "{said}: {stderr}");
    };

    fs::write(root.join("stray.txt"), "not committed").unwrap();
    fs::write(root.join(".gitignore"), "export/\nchanged/\n").unwrap();
    refused("the game's folder holds what its commit does not (.gitignore, stray.txt)");
    fs::remove_file(root.join("stray.txt")).unwrap();
    git(&root, &["checkout", "--quiet", "--", ".gitignore"]);

    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "Not pushed"],
    );
    refused(", and origin's main at ");
    git(&root, &["push", "--quiet", "origin", "main"]);

    fs::remove_file(root.join("steam").join("settings.vdf")).unwrap();
    pushed(&root, "No snapshot");
    refused("settings.vdf is missing: run steamship settings --save in the game and commit it");

    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"Tidal Mill\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "A spaced install folder");
    refused("the install folder is \"Tidal Mill\", and steamship takes");

    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "The install folder as it should be");
    let (unbuilt, stdout, stderr) = in_game(&root, &["upload", "--version", "0.3.2-rc"]);
    assert!(stdout.contains("  version   0.3.2-rc\n"), "{stdout}");
    assert!(
        stdout
            .contains("  build     none named in steamship.toml, so the content goes up as it is"),
        "{stdout}"
    );
    assert_eq!(unbuilt, Some(2_i32), "no build account: {stderr}");
}

#[test]
fn an_upload_in_a_game_refuses_a_build_that_fails() {
    let failing = "build = [\"git\", \"rev-parse\", \"--verify\", \"--quiet\", \"none\"]\n";
    let (_temp, root) = pushed_game(Some(failing));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("git rev-parse --verify --quiet none failed after "),
        "{stderr}"
    );
    assert!(stdout.contains("  version   0.3.1\n"), "{stdout}");
}

#[test]
fn an_upload_in_a_game_refuses_a_build_that_changes_the_tree() {
    let leaving = "build = [\"git\", \"init\", \"--quiet\", \"made\"]\n";
    let (_temp, root) = pushed_game(Some(leaving));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("the game's folder holds what its commit does not (made/)"),
        "{stderr}"
    );
}

/// A build, as `steamship.toml` names it, that writes the game's one file of content again.
const BUILDING: &str =
    "build = [\"git\", \"config\", \"--file\", \"export/game.txt\", \"game.built\", \"now\"]\n";

#[test]
fn an_upload_in_a_game_refuses_a_file_its_build_did_not_write() {
    let (_temp, root) = pushed_game(Some("build = [\"git\", \"--version\"]\n"));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let stale = Path::new("export").join("game.txt");
    assert_eq!(
        failure(&stderr),
        format!(
            "1 file the depots upload was not written by git --version, so is left over from an \
             earlier build: {}. Have the build empty the folders it writes first, or remove what \
             it left",
            stale.display()
        )
    );

    fs::write(root.join("export").join("notes.txt"), "kept by hand").unwrap();
    fs::write(root.join("export").join("old.txt"), "an earlier export").unwrap();
    fs::write(root.join("steamship.toml"), BUILDING).unwrap();
    pushed(&root, "A build that writes the game");
    // Older than the build by more than any file system's clock is coarse, as an export from an
    // earlier build is.
    thread::sleep(Duration::from_millis(50));
    let (rebuilt, said, why) = in_game(&root, &["upload"]);
    assert_eq!(rebuilt, Some(2_i32), "{said}{why}");
    assert!(
        failure(&why).starts_with(&format!(
            "2 files the depots upload were not written by git config --file export/game.txt \
             game.built now, so are left over from an earlier build: {}, {}. ",
            Path::new("export").join("notes.txt").display(),
            Path::new("export").join("old.txt").display(),
        )),
        "{why}"
    );
}

#[test]
fn an_upload_in_a_game_builds_it_then_checks_its_settings_with_steam() {
    let (_temp, root) = pushed_game(Some(BUILDING));
    let (built, stdout, stderr) = in_game(&root, &["upload", "--preview"]);
    assert!(
        stdout.contains("  build     \u{2713} built in "),
        "{stdout}{stderr}"
    );
    assert!(stdout.contains("steamship settings"), "{stdout}");
    assert_eq!(
        built,
        Some(2_i32),
        "no build account to ask Steam with: {stderr}"
    );
    assert!(
        stderr.contains("name the build account with --account or STEAMSHIP_ACCOUNT"),
        "{stderr}"
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
fn outside_a_game_a_command_needing_one_says_to_start_one_or_name_the_app() {
    let empty = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_in(
        empty.path(),
        &["builds"],
        Some(empty.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).ends_with(
            ": no steamship.toml or steam/app_build.vdf in it or a folder above. Run steamship \
             in the game's folder, start one here with `steamship init <app ID>`, or name the \
             app and its files"
        ),
        "{stderr}"
    );
    // An app named does not stand for a file not named: the game's own is still looked for.
    for args in [
        ["settings"].as_slice(),
        ["settings", "--check"].as_slice(),
        ["settings", "480", "--check"].as_slice(),
        ["settings", "480", "--save"].as_slice(),
        ["achievements", "480", "--check"].as_slice(),
        ["leaderboards", "--check"].as_slice(),
        ["leaderboards", "480", "--check"].as_slice(),
        ["rich-presence"].as_slice(),
        ["rich-presence", "480"].as_slice(),
        ["promote", "--build", "7", "--branch", "testing"].as_slice(),
        ["check"].as_slice(),
    ] {
        let (refused, said, why) = steamship_in(
            empty.path(),
            args,
            Some(empty.path()),
            &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
        );
        assert_eq!(refused, Some(2_i32), "{args:?}: {said}{why}");
        assert!(
            failure(&why).starts_with("there is no game in "),
            "{args:?}: {why}"
        );
    }
}

#[test]
fn an_upload_naming_its_script_needs_the_version_it_goes_up_as() {
    let (_temp, root) = pushed_game(None);
    let (code, stdout, stderr) = in_game(&root, &["upload", "steam/app_build.vdf"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "give the version steam/app_build.vdf goes up as with --version, or run steamship upload \
         with no script in the game's folder, which takes project.godot's"
    );
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
fn a_game_with_no_achievement_records_has_no_import_due_unless_one_is_left() {
    let (_temp, root) = pushed_game(None);
    fs::remove_file(root.join("mods/base/data/achievements.json")).unwrap();
    let unchanged = ["achievements", "--write", "--unchanged"];
    let (code, said, why) = in_game(&root, &unchanged);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(
        said.contains("the game holds no achievement records, so there is no import to write"),
        "{said}"
    );

    let import = root.join("steam").join("achievements");
    fs::create_dir_all(&import).unwrap();
    for name in [
        "1000050-achievements-definitions.csv",
        "FIVE_LEVELS_ACHIEVED.png",
        "steam-icons.json",
    ] {
        fs::write(import.join(name), "kept from before").unwrap();
    }
    let (left, _, named) = in_game(&root, &unchanged);
    assert_eq!(left, Some(2_i32), "{named}");
    for line in [
        "1000050-achievements-definitions.csv is left over",
        "FIVE_LEVELS_ACHIEVED.png is left over",
    ] {
        assert!(named.contains(line), "{line:?} in {named}");
    }
    assert!(!named.contains("steam-icons.json"), "{named}");

    fs::remove_dir_all(&import).unwrap();
    fs::write(&import, "a file where the import's folder goes").unwrap();
    let (unread, _, failed) = in_game(&root, &unchanged);
    assert_eq!(unread, Some(1_i32), "{failed}");
    assert!(failed.contains("achievements: "), "{failed}");

    let (written, _, refused) = in_game(&root, &["achievements", "--write"]);
    assert_eq!(written, Some(2_i32), "{refused}");
    assert!(
        refused.contains("--write names no files, and the game holds no achievement records"),
        "{refused}"
    );
}

#[test]
fn an_upload_in_a_game_asks_origin_and_refuses_when_origin_cannot_say_or_git_is_missing() {
    let (temp, root) = pushed_game(None);
    let nowhere = tempfile::tempdir().unwrap();
    let (missing, stdout, stderr) = in_game_with(
        &root,
        &["upload"],
        &[("PATH", nowhere.path().to_str().unwrap())],
    );
    assert_eq!(missing, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "Git is not installed, or not on the PATH, and steamship ships only a pushed commit"
    );

    git(
        temp.path(),
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            "empty.git",
        ],
    );
    let empty = temp.path().join("empty.git");
    git(
        &root,
        &["remote", "set-url", "origin", &empty.to_string_lossy()],
    );
    let (headless, said, why) = in_game(&root, &["upload"]);
    assert_eq!(headless, Some(2_i32), "{said}{why}");
    assert_eq!(
        failure(&why),
        "origin names no default branch, which is the one a game is shipped from"
    );

    git(&root, &["remote", "remove", "origin"]);
    let (unasked, shown, reason) = in_game(&root, &["upload"]);
    assert_eq!(unasked, Some(2_i32), "{shown}{reason}");
    let unasked_why = failure(&reason);
    assert!(
        unasked_why.starts_with("git ls-remote --symref origin HEAD failed in ")
            && unasked_why.ends_with(": fatal: 'origin' does not appear to be a git repository"),
        "Git's own first line: {reason}"
    );
}

#[test]
fn an_upload_in_a_game_says_when_its_build_cannot_be_started() {
    let (_temp, root) = pushed_game(Some("build = [\"steamship-no-such-builder\"]\n"));
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert!(
        failure(&stderr).starts_with("steamship-no-such-builder could not be started: "),
        "{stderr}"
    );
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
    assert_ne!(
        code,
        Some(0_i32),
        "Steam refused every question: {stdout}{stderr}"
    );
    for section in [
        "steamship check\n",
        "steamship achievements\n",
        "steamship leaderboards\n",
        "steamship rich-presence, preview\n",
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
        stdout.contains("  content   not checked; upload checks it once the game is built\n"),
        "{stdout}"
    );
    // The folder's tail alone, as macOS gives the child its temporary folder through /private.
    let data = Path::new("inn").join("mods").join("base").join("data");
    let script = Path::new("inn").join("steam").join("app_build.vdf");
    for (start, end) in [
        (
            "steamship achievements\n  unchecked no achievement, stat or set records in ",
            format!(
                "{}: name the game's in steamship.toml's records",
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
fn a_games_own_records_are_what_achievements_reads_when_it_is_named_no_files() {
    let (_temp, root) = pushed_game(None);
    let outside = tempfile::tempdir().unwrap();
    let records = outside.path().join("achievements.json");
    fs::write(&records, INN_RECORDS).unwrap();
    let (unwritten, stdout, stderr) = steamship(
        &["achievements", "480", "--write", &records.to_string_lossy()],
        Some(outside.path()),
        &[("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    );
    assert_eq!(unwritten, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "--write needs --out, the folder it writes into"
    );

    let import = root.join("steam").join("achievements");
    fs::create_dir_all(&import).unwrap();
    fs::write(import.join("steam-icons.json"), "{}").unwrap();
    let host = refusing_web_api(8);
    let (asked, said, why) = in_game_with(
        &root,
        &["achievements", "--check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_ne!(asked, Some(0_i32), "{said}{why}");
    assert!(said.contains("achievements.json, 2 achievements"), "{said}");

    fs::remove_dir_all(root.join("mods")).unwrap();
    let (none, shown, reason) = in_game(&root, &["achievements", "--check"]);
    assert_eq!(none, Some(2_i32), "{shown}{reason}");
    assert_eq!(
        failure(&reason),
        "--check names no files, and the game holds no achievement records for it to read"
    );
}

#[test]
fn a_games_leaderboard_records_are_checked_and_one_without_any_is_refused() {
    let (_temp, root) = pushed_game(None);
    let (none, stdout, stderr) = in_game(&root, &["leaderboards", "--create"]);
    assert_eq!(none, Some(2_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "the game holds no leaderboard records to check or make"
    );

    fs::write(
        root.join("mods")
            .join("base")
            .join("data")
            .join("boards.json"),
        INN_BOARDS,
    )
    .unwrap();
    let (host, _requests) = web_api(vec![("200 OK", LEADERBOARDS)]);
    let (code, said, why) = in_game_with(
        &root,
        &["leaderboards", "--check"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{said}{why}");
    assert!(said.contains("boards.json, 1 leaderboard\n"), "{said}");
    assert!(
        said.contains("Steam's leaderboards match the file"),
        "{said}"
    );
}

#[test]
fn a_games_rich_presence_is_sent_from_its_pushed_tree_and_one_without_any_is_refused() {
    let (_temp, root) = pushed_game(None);
    let (host, requests) = web_api(vec![("200 OK\r\nX-eresult: 1", r#"{"response": {}}"#)]);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["rich-presence"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(stdout.contains("as origin's main has it"), "{stdout}");
    assert!(
        stdout.contains("app 1000050: rich presence replaced for english"),
        "{stdout}"
    );
    let sent = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        sent.starts_with("POST /IProductInfoService/SetRichPresenceLocalization/v1/ "),
        "{sent}"
    );

    fs::remove_file(root.join("steam").join("rich_presence_english.vdf")).unwrap();
    let (none, said, why) = in_game(&root, &["rich-presence", "--preview"]);
    assert_eq!(none, Some(2_i32), "{said}{why}");
    let refused = failure(&why);
    assert!(
        refused.starts_with("the game holds no rich_presence_<language>.vdf beside ")
            && refused.ends_with(&format!(
                "{}: write rich_presence_english.vdf there, or name the files to send",
                Path::new("steam").join("app_build.vdf").display()
            )),
        "{why}"
    );
    assert!(said.starts_with("steamship rich-presence\n"), "{said}");
}

#[test]
fn a_games_builds_settings_and_promotion_are_asked_for_its_own_app() {
    let (_temp, root) = pushed_game(None);
    let (host, requests) = web_api(vec![("200 OK", BETAS), ("200 OK", BUILDS)]);
    let (code, stdout, stderr) = in_game_with(
        &root,
        &["builds"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &host)],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let asked = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(asked.contains("appid=1000050"), "{asked}");

    let refusing = refusing_web_api(4);
    let (promoted, said, why) = in_game_with(
        &root,
        &["promote", "--build", "7", "--branch", "testing"],
        &[("STEAMSHIP_WEB_API_KEY", KEY), (STAND_IN, &refusing)],
    );
    assert_ne!(promoted, Some(0_i32), "Steam refused it: {said}{why}");
    assert!(!why.contains("there is no game"), "{why}");

    for args in [
        ["settings"].as_slice(),
        ["settings", "--save"].as_slice(),
        ["settings", "--check"].as_slice(),
    ] {
        let (asked_for, shown, reason) = in_game(&root, args);
        assert_ne!(asked_for, Some(0_i32), "no steamcmd here: {shown}{reason}");
        assert!(!reason.contains("there is no game"), "{args:?}: {reason}");
    }
}

#[test]
fn a_demo_goes_up_as_its_own_app_typed_demo_under_the_full_game_and_never_into_it() {
    let (_temp, root) = pushed_game(None);
    let (none, said, why) = in_game(&root, &["check", "--demo"]);
    assert_eq!(none, Some(2_i32), "{said}{why}");
    assert!(
        failure(&why).starts_with(
            "the game has no demo: write its app build script to steam/demo/app_build.vdf"
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
            ": a demo is its own app, so put the demo's ID in it",
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
        "app 1000070 is typed Game, not Demo: set its type to Demo under General Application \
         Settings in Steamworks",
        "app 1000070's settings name no base game: enter the full game, app 1000050, under \
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

/// A demo's settings snapshot's `common` section, typed Demo under the full game.
const DEMO_TYPED: &str = "\t\t\"type\"\t\t\"Demo\"\n\t\t\"parent\"\t\t\"1000050\"\n";

/// Gives the game at `root` a demo in `steam/demo`: app 1000070 setting `testing` live, its one
/// depot 1000071 shipping everything under `content_root`, and a settings snapshot whose `common`
/// section is `common`.
fn with_demo(root: &Path, content_root: &str, common: &str) {
    let demo = root.join("steam").join("demo");
    fs::create_dir_all(&demo).unwrap();
    fs::write(
        demo.join("app_build.vdf"),
        format!(
            r#""AppBuild" {{ "AppID" "1000070" "ContentRoot" "{content_root}" "SetLive" "testing" "Depots" {{ "1000071" "depot.vdf" }} }}"#
        ),
    )
    .unwrap();
    fs::write(
        demo.join("depot.vdf"),
        r#""DepotBuild" { "DepotID" "1000071" "FileMapping" { "LocalPath" "*" "DepotPath" "." "Recursive" "1" } }"#,
    )
    .unwrap();
    fs::write(
        demo.join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{common}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
}

#[test]
fn a_demo_without_the_full_games_steam_input_defaults_is_refused_until_it_has_them() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/", DEMO_TYPED);
    // As a game's snapshot holds them, with a gamepad and a touch default chosen.
    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
         \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
         \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}\n}\n",
    )
    .unwrap();
    pushed(&root, "A game with Steam Input defaults and a demo without");
    let (refused, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(refused, Some(2_i32), "{shown}{reason}");
    for line in [
        "app 1000070 has no Default Controller Configuration, and the full game, app 1000050, \
         has \"Generic Gamepad\": choose the full game's on app 1000070's Steam Input page in \
         Steamworks",
        "app 1000070 has no Default Touch Configuration, and the full game, app 1000050, has \
         \"Mouse point and click\"",
    ] {
        assert!(reason.contains(line), "{line:?} in {reason}");
    }

    fs::write(
        root.join("steam").join("demo").join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{DEMO_TYPED}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\
             \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
             \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
    pushed(&root, "The demo with the game's Steam Input defaults");
    let (passed, building, stopped) = in_game(&root, &["upload", "--demo"]);
    assert!(!stopped.contains("Steam Input"), "{building}{stopped}");
    assert!(
        building.contains("  build     none named under [demo] in steamship.toml"),
        "on to the upload: {building}{stopped}"
    );
    assert_eq!(passed, Some(2_i32), "stopped before steamcmd: {stopped}");
}

#[test]
fn a_demo_bundling_its_own_steam_input_configuration_goes_up_only_with_its_action_manifest() {
    let (_temp, root) = pushed_game(None);
    with_demo(&root, "../../export/", DEMO_TYPED);
    fs::write(
        root.join("steam").join("settings.vdf"),
        "\"1000050\"\n{\n\t\"config\"\n\t{\n\t\t\"installdir\"\t\t\"TidalMillInn\"\n\
         \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
         \t\t\"steamcontrollertemplateindex\"\t\t\"4\"\n\t}\n}\n",
    )
    .unwrap();
    // As a demo's snapshot holds them once its Steam Input page bundles a Custom Configuration.
    fs::write(
        root.join("steam").join("demo").join("settings.vdf"),
        format!(
            "\"1000070\"\n{{\n\t\"common\"\n\t{{\n{DEMO_TYPED}\t}}\n\t\"config\"\n\t{{\n\
             \t\t\"installdir\"\t\t\"InnDemo\"\n\
             \t\t\"steamcontrollertemplateindex\"\t\t\"13\"\n\
             \t\t\"steamcontrollertouchtemplateindex\"\t\t\"8\"\n\
             \t\t\"steamdecktouchscreen\"\t\t\"1\"\n\
             \t\t\"steaminputmanifestpath\"\t\t\"steam_input/action_manifest.vdf\"\n\
             \t\t\"steamconfigurator3rdpartynative\"\t\t\"65535\"\n\t}}\n}}\n"
        ),
    )
    .unwrap();
    pushed(&root, "A demo bundling its Steam Input configuration");
    let (stopped, shown, reason) = in_game(&root, &["upload", "--demo"]);
    assert_eq!(stopped, Some(2_i32), "{shown}{reason}");
    assert!(
        reason.contains("name the build account") && !reason.contains("Steam Input"),
        "past the demo's Steam Input defaults: {shown}{reason}"
    );

    let script = Path::new("steam").join("demo").join("app_build.vdf");
    let script = script.to_str().unwrap();
    let (refused, checking, unshipped) = in_game(&root, &["check", script]);
    assert_eq!(refused, Some(2_i32), "{checking}{unshipped}");
    assert!(
        unshipped.contains(
            "names \"steam_input/action_manifest.vdf\" as app 1000070's Steam Input action \
             manifest, and the build installs no file there: ship the manifest at that path in \
             the install folder"
        ),
        "{checking}{unshipped}"
    );

    let manifest = root.join("export").join("steam_input");
    fs::create_dir_all(&manifest).unwrap();
    fs::write(
        manifest.join("action_manifest.vdf"),
        "\"Action Manifest\"\n{\n}\n",
    )
    .unwrap();
    let (passed, checked, said) = in_game(&root, &["check", script]);
    assert_eq!(passed, Some(0_i32), "{checked}{said}");
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
        "steam/demo/app_build.vdf is an app build script, which names its own app; give --demo a \
         build's folder, or nothing in the game's folder"
    );

    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(outside.path().join("build")).unwrap();
    let (gameless, headed, told) = in_game(outside.path(), &["check", "--demo", "build"]);
    assert_eq!(gameless, Some(2_i32), "{headed}{told}");
    assert!(
        failure(&told).ends_with(
            " is in no game's folder, so it has no demo; run the check in the game's folder, or \
             leave out --demo"
        ),
        "{told}"
    );
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
             export_presets.cfg makes it without the `demo` feature: map only the folders the \
             demo's presets export to",
        ),
        (
            ["check"].as_slice(),
            "into depot 1000051, the full game's, and the export preset \"Windows Desktop Demo\" \
             in export_presets.cfg makes it with the `demo` feature: map only the folders the full \
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
fn a_game_in_no_repository_is_refused_saying_how_to_ship_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("game");
    fs::create_dir_all(root.join("steam")).unwrap();
    fs::write(
        root.join("steam").join("app_build.vdf"),
        r#""AppBuild" { "AppID" "480" "SetLive" "testing" "Depots" { "481" "depot.vdf" } }"#,
    )
    .unwrap();
    let (code, stdout, stderr) = in_game(&root, &["upload"]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let refused = failure(&stderr);
    assert!(
        refused.ends_with(
            " is in no Git repository, and steamship ships a game only from a commit pushed to \
             origin: make it a repository and push it, or name the app build script and its \
             version, as steamship upload <script> --version <version>"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("fatal:"), "{stderr}");
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

/// A program that prints how many processors it may run on.
#[cfg(windows)]
const COUNTING: [&str; 4] = [
    "powershell",
    "-NoProfile",
    "-Command",
    "[Convert]::ToString([int64][Diagnostics.Process]::GetCurrentProcess().ProcessorAffinity, 2).Replace('0', '').Length",
];
#[cfg(target_os = "linux")]
const COUNTING: [&str; 1] = ["nproc"];

/// The `cores` line an upload in a game held to `cores` shows, and how many processors its build
/// found it could run on, which the build prints.
#[cfg(any(windows, target_os = "linux"))]
fn shipped_on(cores: usize) -> (String, usize) {
    let (_temp, root) = pushed_game(Some(&format!("cores = {cores}\nbuild = {COUNTING:?}\n")));
    let (_code, stdout, stderr) = in_game(&root, &["upload"]);
    let shown = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  cores     "))
        .unwrap_or_default()
        .to_owned();
    let found: Option<usize> = stderr.lines().find_map(|line| line.trim().parse().ok());
    assert!(
        found.is_some(),
        "the build printed no count: {stdout}{stderr}"
    );
    (shown, found.unwrap_or_default())
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn shipping_a_game_holds_the_build_and_steamcmd_to_its_cores() {
    let (program, args) = COUNTING.split_first().unwrap();
    let counted = run::output(run::command(program).args(args)).unwrap();
    let machine: usize = String::from_utf8_lossy(&counted.stdout)
        .trim()
        .parse()
        .unwrap();
    let (unheld, every) = shipped_on(4096);
    assert_eq!(every, machine, "{unheld}");
    assert_eq!(
        unheld,
        format!("all {every}, within the 4096 steamcmd and the build may run on")
    );
    let (held, found) = shipped_on(1);
    assert_eq!(found, 1, "{held}");
    if every > 1 {
        assert_eq!(
            held,
            format!("1 of {every}, the most steamcmd and the build run on")
        );
    } else {
        assert_eq!(
            held,
            "all 1, within the 1 steamcmd and the build may run on"
        );
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
#[test]
fn shipping_a_game_on_macos_says_its_cores_are_not_held() {
    let (_temp, root) = pushed_game(Some("cores = 1\n"));
    let (_code, stdout, stderr) = in_game(&root, &["upload"]);
    assert!(
        stdout.contains("\n  cores     every one, as macOS holds no program to fewer\n"),
        "{stdout}{stderr}"
    );
}

#[cfg(unix)]
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
    // As `publishing` does, keeping each script as it was sent before giving a new item its ID.
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

/// A steamcmd for Tidal Mill Inn, still unreleased, that prints the app's info when asked for it,
/// and otherwise has Valve build the upload as `BuildID` 4242, adding a line to `builds` in the
/// home, or finish the preview the script asks for.
#[cfg(unix)]
const INN_STEAMCMD: &str = "#!/bin/sh\n\
    case \" $* \" in *' +app_info_print '*)\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'AppID : 1000050, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
    printf '\"1000050\"\\n{\\n\\t\"common\"\\n\\t{\\n\\t\\t\"name\"\\t\\t\"Tidal Mill Inn\"\\n'\n\
    printf '\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n\\t}\\n'\n\
    printf '\\t\"config\"\\n\\t{\\n\\t\\t\"installdir\"\\t\\t\"TidalMillInn\"\\n\\t}\\n}\\n'\n\
    echo 'Unloading Steam API...OK'\n\
    exit 0;;\n\
    esac\n\
    for arg in \"$@\"; do [ \"$previous\" = +run_app_build ] && script=\"$arg\"; previous=\"$arg\"; done\n\
    finished='build (BuildID 4242).'\n\
    if grep -q '\"Preview\"[[:space:]]*\"1\"' \"$script\"; then finished='build preview.'; \
    else echo built >> \"$HOME/builds\"; fi\n\
    output=\"$HOME/apps/1000050/output\"\n\
    mkdir -p \"$output\"\n\
    echo \"Successfully finished AppID 1000050 $finished\" > \"$output/app_build_1000050.log\"\n\
    echo 'Logging in user build_bot to Steam Public...OK'\n";

/// The game [`pushed_game`] makes with `toml`, without achievements, beside a home whose
/// steamcmd is [`INN_STEAMCMD`] and whose settings snapshot is Steam's own, pushed.
#[cfg(unix)]
fn settled_inn(toml: &str) -> (tempfile::TempDir, PathBuf, tempfile::TempDir) {
    let (temp, root) = pushed_game(Some(toml));
    fs::remove_file(
        root.join("mods")
            .join("base")
            .join("data")
            .join("achievements.json"),
    )
    .unwrap();
    let home = faked_with(INN_STEAMCMD);
    saved_login(home.path());
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];

    let (saved, said, why) =
        steamship_in(&root, &["settings", "--save"], Some(home.path()), &quiet);
    assert_eq!(saved, Some(0_i32), "{said}{why}");
    pushed(&root, "The settings as Steam has them");
    let (checked, shown, reason) =
        steamship_in(&root, &["settings", "--check"], Some(home.path()), &quiet);
    assert_eq!(checked, Some(0_i32), "{shown}{reason}");
    assert!(shown.contains("Steam's settings match the file"), "{shown}");
    (temp, root, home)
}

/// `steamship upload` in the game at `root`, with a Web API that sets build 4242 live and
/// describes `testing`, and the first five requests it was sent.
#[cfg(unix)]
fn uploaded_inn(root: &Path, home: &Path) -> (Option<i32>, String, String, Vec<String>) {
    let (host, requests) = web_api(vec![
        ("200 OK", BETAS),
        ("200 OK", testing_live(4242)),
        ("200 OK", SET_LIVE),
        ("200 OK", DESCRIBED),
        ("200 OK", UPDATED),
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
    ]);
    let (code, stdout, stderr) = steamship_in(
        root,
        &["upload"],
        Some(home),
        &[
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_WEB_API_KEY", KEY),
            (STAND_IN, &host),
        ],
    );
    let asked = if code == Some(0_i32) {
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(5)
            .collect()
    } else {
        Vec::new()
    };
    (code, stdout, stderr, asked)
}

#[cfg(unix)]
#[test]
fn a_game_is_built_checked_uploaded_live_on_default_too_and_its_branch_described() {
    let (_temp, root, home) = settled_inn(BUILDING);
    let (code, stdout, stderr, asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    for line in [
        "  version   0.3.1\n",
        "  build     \u{2713} built in ",
        "app 1000050: BuildID 4242, set live on testing and default",
        "app 1000050: testing described as \"0.3.1\"",
        "steamship builds\n",
    ] {
        assert!(stdout.contains(line), "{line} in {stdout}{stderr}");
    }
    assert_eq!(
        fs::read_to_string(home.path().join("builds")).unwrap(),
        "built\n"
    );
    let describing = asked.last().unwrap();
    assert!(
        describing.starts_with("POST /ISteamApps/UpdateAppBranchDescription/v1/ ")
            && describing.ends_with("appid=1000050&betakey=testing&description=0.3.1"),
        "{asked:?}"
    );

    let (again, _again_asked) = web_api(vec![("200 OK", BETAS); 4]);
    let (previewed, shown_preview, preview_why) = steamship_in(
        &root,
        &["upload", "--preview"],
        Some(home.path()),
        &[
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_WEB_API_KEY", KEY),
            (STAND_IN, &again),
        ],
    );
    assert_eq!(previewed, Some(0_i32), "{shown_preview}{preview_why}");
    assert!(
        shown_preview.contains("\u{2713} nothing was uploaded, as asked\n")
            && !shown_preview.contains("steamship branch")
            && !shown_preview.contains("steamship builds"),
        "a preview ends with Valve's dry run: {shown_preview}"
    );
}

#[cfg(unix)]
#[test]
fn a_game_whose_steamship_toml_names_a_description_has_its_branch_described_so() {
    let (_temp, root, home) = settled_inn(&format!("description = \"nightly\"\n{BUILDING}"));
    let (code, stdout, stderr, asked) = uploaded_inn(&root, home.path());
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("app 1000050: testing described as \"nightly\""),
        "{stdout}{stderr}"
    );
    assert!(
        asked
            .last()
            .unwrap()
            .ends_with("appid=1000050&betakey=testing&description=nightly"),
        "{asked:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_games_first_upload_to_an_app_with_no_branch_yet_succeeds_and_says_how_to_describe_it() {
    let (_temp, root, home) = settled_inn(&format!("description = \"testing\"\n{BUILDING}"));
    let (host, requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        (
            "200 OK",
            r#"{"applist": {"apps": {"app": [
                {"appid": 1000050, "app_type": "game", "app_name": "Tidal Mill Inn"}
            ]}}}"#,
        ),
        ("200 OK", SET_LIVE),
        ("200 OK", BETAS),
        ("200 OK", BUILDS),
    ]);
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["upload"],
        Some(home.path()),
        &[
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
            ("STEAMSHIP_WEB_API_KEY", KEY),
            (STAND_IN, &host),
        ],
    );
    let shown = format!("{stdout}{stderr}");
    assert_eq!(code, Some(0_i32), "{shown}");
    for line in [
        "app 1000050: BuildID 4242, set live on default",
        "make testing in Steamworks under SteamPipe, Builds, and set the build live there with \
         steamship promote 1000050 --build 4242 --branch testing",
        "once it is made, describe testing with steamship branch 1000050 testing --description \
         \"testing\"",
        "steamship builds\n",
    ] {
        assert!(shown.contains(line), "{line:?} in {shown}");
    }
    let asked: Vec<String> =
        iter::repeat_with(|| requests.recv_timeout(Duration::from_secs(10)).unwrap())
            .take(5)
            .collect();
    assert!(
        !asked
            .iter()
            .any(|request| request.contains("UpdateAppBranchDescription")),
        "a branch Steam does not have yet is not described: {asked:?}"
    );
}

/// The template `text --template` prints for [`TEMPLATED_STORE`], [`TEMPLATED_WORKSHOP`] and
/// [`TEMPLATED_PRESENCE`]: every entry once, by context and English.
const TEMPLATE: &str = concat!(
    "# The Steam text of a game, for its translation template, by steamship: the store page ",
    "under \"store page\",\n",
    "# the Workshop page under \"steam workshop\" and rich presence under \"rich presence\".\n",
    "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n",
    "#. What Steam shows the player's friends. Keep every {field}, which Steam fills with a ",
    "value, and every {#Token_{field}} whole, which Steam fills with that token's own line.\n",
    "msgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"\"\n\n",
    "#. What Steam shows the player's friends. Keep every {field}, which Steam fills with a ",
    "value, and every {#Token_{field}} whole, which Steam fills with that token's own line.\n",
    "msgctxt \"rich presence\"\nmsgid \"Grinding at {mill}\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam Workshop page, beside its picture: the title a short headline, the ",
    "description one or two lines; plain prose, no markup.\n",
    "msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam Workshop page, beside its picture: the title a short headline, the ",
    "description one or two lines; plain prose, no markup.\n",
    "#, max-length:200\n",
    "msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam store page; plain prose, no markup.\n",
    "#, max-length:300\n",
    "msgctxt \"store page\"\nmsgid \"A mill on a tidal river.\"\nmsgstr \"\"\n\n",
    "#. A line of the game's Steam store page outside its description, such as a system ",
    "requirement; plain text, every product and model name kept as written.\n",
    "msgctxt \"store page\"\nmsgid \"All rights reserved.\"\nmsgstr \"\"\n\n",
    "#. On the game's Steam store page; plain prose, no markup.\n",
    "msgctxt \"store page\"\nmsgid \"Grind \\\"grain\\\" at high water.\"\nmsgstr \"\"\n\n",
    "#. The game's statement on its Steam store page of how AI was used in making it, shown ",
    "under About This Game; plain prose, no markup, and the same plain claim in every ",
    "language.\n",
    "msgctxt \"store page\"\nmsgid \"Made with AI assistance for coding.\"\nmsgstr \"\"\n\n",
    "#. Alt text for a screenshot on the game's Steam store page, read aloud by screen readers; ",
    "plain prose.\n",
    "msgctxt \"store page\"\nmsgid \"The harbour at dawn.\"\nmsgstr \"\"\n\n",
    "#. Alt text for a picture in the About This Game of the game's Steam store page, read aloud ",
    "by screen readers; plain prose.\n",
    "msgctxt \"store page\"\nmsgid \"The wheel turning.\"\nmsgstr \"\"\n",
);

const TEMPLATED_STORE: &str = r#"{
  "itemid": 1000090,
  "disclosure": "Made with AI assistance for coding.",
  "screenshots": ["The harbour at dawn."],
  "extras": ["about_mill"],
  "blocks": [
    {"kind": "short", "text": "A mill on a tidal river."},
    {"kind": "paragraph", "text": "Grind \"grain\" at high water."},
    {"kind": "image", "image": "about_mill", "alt": "The wheel turning."},
    {"kind": "field", "field": "app[content][legal]", "text": "All rights reserved."},
    {"kind": "image", "image": "about_mill"},
    {"kind": "paragraph", "text": "A mill on a tidal river."}
  ]
}"#;
const TEMPLATED_WORKSHOP: &str = r#"{"title": "Harbours", "description": "Made by players."}"#;
const TEMPLATED_PRESENCE: &str = r##"{"#menu": "At the menu", "#grinding": "Grinding at %mill%",
    "#Mill_": {"records": "achievement"}}"##;

#[test]
fn a_games_steam_text_is_printed_as_a_template_its_own_takes_in() {
    let (_temp, root) = pushed_game(None);
    let steam = root.join("steam");
    fs::write(steam.join("store_english.json"), TEMPLATED_STORE).unwrap();
    fs::write(steam.join("workshop_english.json"), TEMPLATED_WORKSHOP).unwrap();
    fs::write(steam.join("presence_english.json"), TEMPLATED_PRESENCE).unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(0_i32), "{told}");
    assert_eq!(template, TEMPLATE, "{told}");
    assert!(
        told.contains("the template holds 10 messages of Steam's text"),
        "{told}"
    );
    assert!(
        !steam.join("workshop").exists(),
        "nothing is written beside the scripts"
    );
}

#[test]
fn a_short_description_steam_would_cut_is_refused_before_any_template() {
    let (_temp, root) = pushed_game(None);
    let long = "A mill. ".repeat(40);
    fs::write(
        root.join("steam/store_english.json"),
        format!(r#"{{"blocks": [{{"kind": "short", "text": "{long}"}}]}}"#),
    )
    .unwrap();

    let (code, template, told) = in_game(&root, &["text", "--template"]);
    assert_eq!(code, Some(2_i32), "{told}");
    assert_eq!(template, "");
    assert!(
        told.contains(&format!(
            "store_english.json: {long:?} is 320 characters in english, past the 300 Steam shows: \
             shorten it"
        )),
        "{told}"
    );
}

#[test]
fn a_translation_past_steams_room_is_refused_and_nothing_is_written() {
    let (_temp, root) = pushed_game(None);
    fs::write(root.join("steam/workshop_english.json"), TEMPLATED_WORKSHOP).unwrap();
    let long = "Von Spielern gemacht. ".repeat(10);
    let locale = root.join("game/locale/de.po");
    let mut german = fs::read_to_string(&locale).unwrap();
    german.push_str(
        "\nmsgctxt \"rich presence\"\nmsgid \"At the menu\"\nmsgstr \"Im Men\u{fc}\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Harbours\"\nmsgstr \"H\u{e4}fen\"\n\n\
         msgctxt \"steam workshop\"\nmsgid \"Made by players.\"\n",
    );
    german.push_str(&["msgstr \"", &long, "\"\n"].concat());
    fs::write(&locale, german).unwrap();

    let (code, _, told) = in_game(&root, &["text"]);
    assert_eq!(code, Some(2_i32), "{told}");
    assert!(
        told.contains(&format!(
            "de.po: {long:?} is 220 characters in german, past the 200 Steam shows: shorten it"
        )),
        "{told}"
    );
    assert!(!root.join("steam/workshop").exists(), "nothing is written");
}

#[test]
fn every_locale_is_listed_with_steams_name_for_its_language() {
    let (as_json, listed, told) = steamship(&["languages", "--json"], None, &[]);
    assert_eq!(as_json, Some(0_i32), "{told}");
    let table: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&listed).unwrap();
    assert_eq!(table.len(), 34);
    for (locale, language) in [
        ("pt-BR", "brazilian"),
        ("zh-Hans", "schinese"),
        ("es-419", "latam"),
        ("ko", "koreana"),
    ] {
        assert_eq!(
            table.get(locale).and_then(|named| named.as_str()),
            Some(language)
        );
    }

    let (code, shown, why) = steamship(&["languages"], None, &[]);
    assert_eq!(code, Some(0_i32), "{why}");
    assert!(shown.contains("  pt-BR     brazilian\n"), "{shown}");
}

/// The SBOM a release of this version would publish, giving this system's binary `sha256`.
fn sbom_listing(sha256: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let target = itself::target().unwrap();
    format!(
        r#"{{"spdxVersion": "SPDX-2.3", "files": [{{"fileName": "steamship-{version}-{target}/steamship{}", "checksums": [{{"algorithm": "SHA256", "checksumValue": "{sha256}"}}]}}]}}"#,
        if cfg!(windows) { ".exe" } else { "" }
    )
}

/// `self check`, reading releases from `host`.
fn self_check(host: &str) -> (Option<i32>, String, String) {
    let home = tempfile::tempdir().unwrap();
    steamship(
        &["self", "check"],
        Some(home.path()),
        &[(itself::STAND_IN, host), ("STEAMSHIP_NO_UPDATE_CHECK", "1")],
    )
}

#[test]
fn a_binary_its_release_lists_passes_its_self_check() {
    use steamship::digest;
    let own = digest::hex(
        &digest::sha256(&mut File::open(env!("CARGO_BIN_EXE_steamship")).unwrap()).unwrap(),
    );
    let (host, requests) = web_api(vec![("200 OK", sbom_listing(&own))]);

    let (code, said, told) = self_check(&host);
    assert_eq!(code, Some(0_i32), "{said}{told}");
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        said.contains(&format!("is steamship {version}'s release binary for ")),
        "{said}"
    );
    assert!(said.contains(&own), "{said}");
    assert!(
        requests
            .recv()
            .unwrap()
            .starts_with(&format!("GET /v{version}/steamship-{version}.spdx.json ")),
        "the SBOM of this version's release is read"
    );
}

#[test]
fn a_binary_that_differs_from_its_release_is_refused() {
    let (host, _requests) = web_api(vec![("200 OK", sbom_listing(&"0".repeat(64)))]);

    let (code, said, told) = self_check(&host);
    assert_eq!(code, Some(2_i32), "{said}{told}");
    assert!(
        told.contains(&format!(
            "binary of steamship {}'s release: it was built from source, as `cargo install` \
             builds it, or elsewhere, or altered since. Install the release's own binary, from \
             https://github.com/Aureliolo/steamship/releases/latest or with \
             `cargo binstall steamship`",
            env!("CARGO_PKG_VERSION")
        )),
        "{told}"
    );
    assert!(said.contains(&"0".repeat(64)), "{said}");
}

#[test]
fn a_release_without_an_sbom_fails_the_self_check_saying_so() {
    let (missing, said, told) = self_check(&web_api(vec![("404 Not Found", "{}")]).0);
    assert_eq!(missing, Some(1_i32), "{said}{told}");
    assert!(said.contains("404"), "{said}{told}");

    let (unreadable, read, refusal) = self_check(&web_api(vec![("200 OK", "{}")]).0);
    assert_eq!(unreadable, Some(1_i32), "{read}{refusal}");
    assert!(refusal.contains(".spdx.json lists no files"), "{refusal}");
}
