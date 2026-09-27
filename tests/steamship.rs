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

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read as _, Write};
use std::iter;
use std::net::TcpListener;
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use steamship::account::Account;
use steamship::install;
use steamship::keychain;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::terminal::{Event, Reader};
#[cfg(unix)]
use steamship::unix::Terminal;
use steamship::webapi::STAND_IN;
#[cfg(windows)]
use steamship::windows::Terminal;
#[cfg(unix)]
use steamship::{ci, steamcmd};

/// The `steamship` built for these tests, which on Linux finds this process's own Secret Service
/// and never the session's.
fn steamship_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_steamship"));
    let _: &mut Command = command.envs(common::store_environment());
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
    let _: &mut Command = command.args(args).env_remove("STEAMSHIP_ACCOUNT");
    let _: &mut Command = match home {
        Some(home) => command.env("STEAMSHIP_HOME", home),
        None => command.env_clear(),
    };
    // After the environment may have been cleared, so that nothing reaches the session's store.
    let _: &mut Command = command.envs(common::store_environment());
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
        "  login     Log in to Steam, once, for uploads\n",
        "  status    Show the login and steamcmd, checking the login with Steam\n",
        "  check     Check the build scripts, without logging in\n",
        "  upload    Check, build and upload, then print the build ID\n",
        "  workshop  Upload a Workshop item, then print its ID\n",
        "  builds    Show an app's branches and last builds\n",
        "  promote   Set an uploaded build live on a branch\n",
        "  ci        Set up uploads from CI, the login kept as a secret\n",
        "  logout    Forget the saved login\n",
        "  install   Install or verify the pinned steamcmd\n",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
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
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("PROGRAM", env!("CARGO_BIN_EXE_steamship"))
        .output()
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

/// steamship.exe as its headers describe it, read the way the loader reads them.
#[cfg(windows)]
struct Program(Vec<u8>);

#[cfg(windows)]
impl Program {
    fn bytes(&self, at: usize, length: usize) -> &[u8] {
        self.0
            .get(at..at.checked_add(length).unwrap())
            .ok_or("past the end of the program")
            .unwrap()
    }

    fn u16(&self, at: usize) -> u16 {
        u16::from_le_bytes(self.bytes(at, 2).try_into().unwrap())
    }

    fn u32(&self, at: usize) -> usize {
        usize::try_from(u32::from_le_bytes(self.bytes(at, 4).try_into().unwrap())).unwrap()
    }

    /// Where the 64-bit optional header starts.
    fn optional(&self) -> usize {
        let pe = self.u32(0x3C);
        assert_eq!(self.bytes(pe, 4), b"PE\0\0", "a Windows program");
        let optional = pe.checked_add(24).unwrap();
        assert_eq!(self.u16(optional), 0x20B, "a 64-bit program");
        optional
    }

    /// The file offset of a data directory's contents, and their size.
    fn directory(&self, index: usize) -> (usize, usize) {
        let entry = self
            .optional()
            .checked_add(112)
            .and_then(|table| table.checked_add(index.checked_mul(8)?))
            .unwrap();
        (
            self.offset(self.u32(entry)),
            self.u32(entry.checked_add(4).unwrap()),
        )
    }

    /// The file offset of an address as loaded, found through the section it falls in.
    fn offset(&self, address: usize) -> usize {
        let pe = self.u32(0x3C);
        let sections = usize::from(self.u16(pe.checked_add(6).unwrap()));
        let first = self
            .optional()
            .checked_add(usize::from(self.u16(pe.checked_add(20).unwrap())))
            .unwrap();
        (0..sections)
            .map(|index| first.checked_add(index.checked_mul(40).unwrap()).unwrap())
            .find_map(|header| {
                let (size, start, raw) = (
                    self.u32(header.checked_add(8).unwrap()),
                    self.u32(header.checked_add(12).unwrap()),
                    self.u32(header.checked_add(20).unwrap()),
                );
                let inside = address.checked_sub(start).filter(|within| *within < size)?;
                raw.checked_add(inside)
            })
            .unwrap()
    }
}

/// The exploit mitigations the loader enforces, and the manifest it reads, on the program as
/// built: a flag lost from the build configuration fails here rather than going unnoticed.
#[cfg(windows)]
#[test]
fn the_windows_program_has_its_mitigations_and_manifest() {
    let program = Program(fs::read(env!("CARGO_BIN_EXE_steamship")).unwrap());
    let characteristics = program.u16(program.optional().checked_add(70).unwrap());
    for (flag, name) in [
        (0x0020, "high-entropy ASLR"),
        (0x0040, "ASLR"),
        (0x0100, "DEP"),
        (0x4000, "Control Flow Guard"),
    ] {
        assert_ne!(characteristics & flag, 0, "{name}");
    }

    let (debug, size) = program.directory(6);
    let shadow_stack = (0..size.checked_div(28).unwrap())
        .map(|index| debug.checked_add(index.checked_mul(28).unwrap()).unwrap())
        .filter(|entry| program.u32(entry.checked_add(12).unwrap()) == 20)
        .any(|entry| program.u32(program.u32(entry.checked_add(24).unwrap())) & 1 != 0);
    assert!(shadow_stack, "compatible with the hardware shadow stack");

    let (load_config, _) = program.directory(10);
    assert_eq!(
        program.u16(load_config.checked_add(78).unwrap()),
        0x800,
        "imports looked up in System32 alone"
    );

    let text = String::from_utf8_lossy(&program.0);
    for setting in [
        r#"<requestedExecutionLevel level="asInvoker" uiAccess="false"/>"#,
        ">UTF-8</activeCodePage>",
        ">true</longPathAware>",
    ] {
        assert!(text.contains(setting), "the manifest says {setting}");
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
    use std::os::unix::ffi::OsStrExt as _;

    let home = tempfile::tempdir().unwrap();
    let output = steamship_command()
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
/// `STEAMSHIP_FAKE_GH_REFUSE` is set; `gh api` answers a commit, `STEAMSHIP_FAKE_GH_COMMIT` if
/// set, or fails when `STEAMSHIP_FAKE_GH_OFFLINE` is set.
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
         echo \"${STEAMSHIP_FAKE_GH_COMMIT:-0123456789abcdef0123456789abcdef01234567}\" ;;\n\
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
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
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
    let mut child = steamship_command()
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
                        .chain(&common::store_environment())
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
    let console = fs::read_to_string(home.path().join("workshop/480/steamcmd.log")).unwrap();
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

#[test]
fn builds_with_no_key_set_or_kept_and_no_one_to_ask_says_how_to_keep_one_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship(
        &["builds", "5335950"],
        Some(home.path()),
        &[("STEAMSHIP_WEB_API_KEY", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}");
    assert!(stdout.contains("  app       5335950\n"), "{stdout}");
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
    let mut child = command.spawn().unwrap();
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
    {"appid": 5335950, "app_type": "game", "app_name": "Fantasy Guild Manager"},
    {"appid": 5335970, "app_type": "game", "app_name": "Ostinato"}
]}}}"#;

/// `GetAppBetas` and `GetAppBuilds` answering with one branch and one build.
const BETAS: &str = r#"{"response": {"result": 1, "betas": {"testing": {"BuildID": 7}}}}"#;
const BUILDS: &str = r#"{"response": {"builds": {"7": {"Description": "0.1.0"}}}}"#;

/// `SetAppBuildLive` answering that the build is live.
const SET_LIVE: &str = r#"{"response": {"result": 1}}"#;

/// A stand-in for Steam's partner Web API on this machine, which a debug build of steamship is
/// sent to through `STEAMSHIP_WEB_API_STAND_IN`. It answers one request with each status and body
/// in `answers`, in turn, and hands over each request it read.
fn web_api(answers: Vec<(&'static str, &'static str)>) -> (String, mpsc::Receiver<String>) {
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
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            drop(sender.send(request));
        }
    });
    (format!("http://{address}"), requests)
}

/// `steamship login --web-api-key` with the Web API at `host`, `typed` piped to its prompt, and
/// `variables` set besides.
fn login_with_key(
    home: &Path,
    host: &str,
    typed: &str,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let mut child = steamship_command()
        .args(["login", "--web-api-key"])
        .env("STEAMSHIP_HOME", home)
        .env(STAND_IN, host)
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
    let _store = common::store_lock();
    let home = tempfile::tempdir().unwrap();
    let (host, requests) = web_api(vec![("200 OK", APPS)]);
    let (code, stdout, stderr) = login_with_key(home.path(), &host, KEY, &[]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("Steam takes the key, for 2 apps\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  5335950   Fantasy Guild Manager\n  5335970   Ostinato\n"),
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
        request.starts_with("GET /ISteamApps/GetPartnerAppListForWebAPIKey/v2/ "),
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
        &["builds", "5335950"],
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
    let home = tempfile::tempdir().unwrap();
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
    let _store = common::store_lock();
    let steamship_program = env!("CARGO_BIN_EXE_steamship");
    #[cfg(windows)]
    let line = format!("{steamship_program} builds 5335950");
    #[cfg(unix)]
    let line = format!("'{steamship_program}' builds 5335950");
    for (answer, shown) in [("\r", "\u{2713} kept in"), ("n\r", "not kept")] {
        let home = tempfile::tempdir().unwrap();
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
        drop(steamship(&["logout"], Some(home.path()), &[]));
    }
}

#[cfg(unix)]
#[test]
fn login_on_a_terminal_keeps_the_key_typed_at_its_offer_and_offers_no_more() {
    let _store = common::store_lock();
    let home = asking();
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
    drop(steamship(&["logout"], Some(home.path()), &[]));
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
    let (built, listed, said) = steamship(&["builds", "5335950"], Some(home.path()), &set);
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
        &["promote", "5335950", "--build", "7", "--branch", "testing"],
        Some(home.path()),
        &set,
    );
    assert_eq!((promoted, stderr.as_str()), (Some(0_i32), ""), "{live}");
    assert!(live.contains("  steam     \u{2713} set live\n"), "{live}");
    assert!(
        live.contains("app 5335950: BuildID 7 live on testing"),
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
            && setting.ends_with("appid=5335950&buildid=7&betakey=testing"),
        "{setting}"
    );
}

#[test]
fn builds_and_promote_that_steam_does_not_answer_say_so_and_exit_1() {
    let home = tempfile::tempdir().unwrap();
    let (host, _requests) = web_api(vec![
        ("500 Internal Server Error", ""),
        ("500 Internal Server Error", ""),
    ]);
    let set = [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, &host),
    ];
    for args in [
        &["builds", "5335950"][..],
        &["promote", "5335950", "--build", "7", "--branch", "testing"],
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
    let (code, builds, stderr) = steamship(&["builds", "5335950"], Some(home.path()), &broken);
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
        &["builds", "5335950"],
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

#[test]
fn promote_leaves_the_default_branch_to_steamworks_and_exits_2() {
    let (_project, script) = project(false);
    let home = tempfile::tempdir().unwrap();
    for default in ["default", "public"] {
        let (code, stdout, stderr) = steamship(
            &[
                "promote",
                script.to_str().unwrap(),
                "--build",
                "1234",
                "--branch",
                default,
            ],
            Some(home.path()),
            &[],
        );
        assert_eq!(code, Some(2_i32), "{stdout}");
        assert!(
            stdout.contains("  app       1000\n"),
            "the app from its script: {stdout}"
        );
        assert_eq!(
            failure(&stderr),
            "the default branch is set live in Steamworks, not by steamship"
        );
    }
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
    let mut steamship = steamship_command()
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
