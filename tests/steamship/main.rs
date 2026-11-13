//! The `steamship` command itself: what it prints and the code it exits with.
//!
//! These run after the library's own tests on purpose: a change that broke the installer would
//! send `install` here to Valve's CDN, and the unit tests that fail at once on such a change are
//! the ones that should say so.
#[path = "../common/mod.rs"]
pub mod common;

mod commands {
    mod achievements;
    mod assets;
    mod branch;
    mod builds;
    mod check;
    mod ci;
    mod drm;
    mod fetch;
    mod init;
    mod install;
    mod itself;
    mod languages;
    mod leaderboards;
    mod login;
    mod logout;
    mod presence;
    mod promote;
    mod ready;
    mod rollback;
    mod settings;
    mod status;
    mod text;
    mod upload;
    mod workshop;
}

#[cfg(windows)]
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read as _, Write};
use std::iter;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::common::git::{commit, git, with_origin};
use crate::common::pe::windows_library;
use steamship::account::Account;
use steamship::install;
use steamship::manifest::Manifest;
use steamship::platform::Platform;
use steamship::run;
use steamship::terminal::{Event, Reader};
#[cfg(unix)]
use steamship::unix::Terminal;
use steamship::webapi::STAND_IN;
#[cfg(windows)]
use steamship::windows::Terminal;
use steamship::{ci, steamcmd};

/// The `steamship` built for these tests, which on Linux finds this process's own Secret Service
/// and never the session's.
fn steamship_command() -> Command {
    let mut command = run::command(env!("CARGO_BIN_EXE_steamship"));
    let _: &mut Command = command.envs(common::store::environment());
    command
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
    outcome(&run::output(&mut set_up(folder, args, home, variables)).unwrap())
}

/// [`steamship`] in `home`, with `typed` piped to it as a script would type it.
fn steamship_typing(
    args: &[&str],
    home: &Path,
    variables: &[(&str, &str)],
    typed: &str,
) -> (Option<i32>, String, String) {
    let mut command = set_up(Path::new("."), args, Some(home), variables);
    let mut child = run::spawn(
        command
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
    outcome(&child.wait_with_output().unwrap())
}

/// Its exit code, and what it wrote to standard output and standard error.
fn outcome(output: &Output) -> (Option<i32>, String, String) {
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// steamship with `args` in `folder`, as [`steamship`] runs it.
fn set_up(
    folder: &Path,
    args: &[&str],
    home: Option<&Path>,
    variables: &[(&str, &str)],
) -> Command {
    let mut command = steamship_command();
    let _: &mut Command = command.current_dir(folder);
    // The runner's own CI variables would make every test a CI run, writing to the job's real
    // outputs. A test that wants them sets its own.
    let _: &mut Command = command
        .args(args)
        .env_remove("STEAMSHIP_ACCOUNT")
        .env_remove("STEAMSHIP_BRANCH_PASSWORD")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("GITHUB_OUTPUT");
    let _: &mut Command = match home {
        Some(home) => command.env("STEAMSHIP_HOME", home),
        None => command.env_clear(),
    };
    // After the environment may have been cleared, so that nothing reaches the session's store.
    let _: &mut Command = command.envs(common::store::environment());
    #[cfg(windows)]
    if let Some(home) = home
        && home.join(install::FOLDER).join("steamcmd.sh").is_file()
    {
        let _: &mut Command = command
            .env("STEAMSHIP_FAKE_STEAMCMD", shell())
            .env("HOME", home);
    }
    // Last, so that a test can name a shell that does not exist.
    let _: &mut Command = command.envs(variables.iter().copied());
    command
}

/// Git's `sh`, which runs a fake steamcmd on Windows: the one on the PATH, or else the one beside
/// the `git` on it.
#[cfg(windows)]
fn shell() -> PathBuf {
    let path = env::var_os("PATH").unwrap_or_default();
    let folders: Vec<PathBuf> = env::split_paths(&path).collect();
    folders
        .iter()
        .map(|folder| folder.join("sh.exe"))
        .chain(folders.iter().filter_map(|folder| {
            folder
                .join("git.exe")
                .is_file()
                .then(|| folder.parent().map(|git| git.join("bin").join("sh.exe")))
                .flatten()
        }))
        .find(|shell| shell.is_file())
        .unwrap()
}

/// The first failure steamship reported, without the cross before it. When it reported none,
/// all it wrote, for the assertion to show.
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
fn help_lists_every_command_with_one_short_line() {
    let (code, stdout, _) = steamship(&["--help"], Some(Path::new(".")), &[]);
    assert_eq!(code, Some(0_i32));
    assert!(
        !stdout.contains("uploads your build to Steam"),
        "no banner in a log"
    );
    assert!(
        stdout.contains("Usage: steamship [OPTIONS] <COMMAND>\n"),
        "named as it is typed, on Windows too: {stdout}"
    );
    assert!(
        stdout.contains("      --times    Show how long each step took, at the end of its line\n"),
        "{stdout}"
    );
    for line in [
        "  login          Log in to Steam, once, for uploads\n",
        "  status         Show the login, steamcmd and whether Steam accepts the login\n",
        "  check          Check the build scripts, without logging in\n",
        "  upload         Check, build and upload, then print the build ID\n",
        "  workshop       Upload a Workshop item, then print its ID\n",
        "  builds         Show an app's branches and last builds\n",
        "  fetch          Download a branch's live build into its own folder\n",
        "  achievements   Show, save or check an app's achievements and stats, or write their \
         import\n",
        "  assets         Check artwork and icons against Valve's sizes, before they are \
         uploaded\n",
        "  drm-wrap       Wrap a Windows executable in Steam DRM, before it is uploaded\n",
        "  settings       Show an app's settings in Steamworks, or check them against a snapshot\n",
        "  leaderboards   Show, save, check, create or delete an app's leaderboards\n",
        "  rich-presence  Upload an app's rich presence localisation\n",
        "  text           Write a game's store, Workshop and rich presence text in every language \
         it speaks\n",
        "  promote        Set an uploaded build live on a branch\n",
        "  rollback       Set a branch back to its previous build\n",
        "  branch         Set the description players see for a beta branch\n",
        "  ci             Set up uploads from CI, with the login as a secret\n",
        "  logout         Forget the saved login\n",
        "  init           Write starter build scripts for an app\n",
        "  install        Install or verify the pinned steamcmd\n",
        "  languages      List Steam's languages with their locale codes\n",
        "  self           Check this steamship binary\n",
        "  completions    Print tab completion for bash, zsh, fish, PowerShell or elvish\n",
    ] {
        assert!(stdout.contains(line), "{line:?} in {stdout}");
    }
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
    // ExtractIconExW is declared through reflection: Add-Type would compile C#, which takes
    // seconds on a runner that has not compiled any yet, and half a minute on Windows on Arm.
    let script = "$ErrorActionPreference = 'Stop'
        $type = [AppDomain]::CurrentDomain.DefineDynamicAssembly(
          [Reflection.AssemblyName]::new('Shell'), 'Run').DefineDynamicModule('Shell').DefineType(
          'Icons', 'Public')
        $type.DefinePInvokeMethod('ExtractIconExW', 'shell32.dll', 'Public, Static', 'Standard',
          [uint32], [Type[]] @([string], [int], [IntPtr], [IntPtr], [uint32]), 'Winapi',
          'Unicode').SetImplementationFlags('PreserveSig')
        $icons = $type.CreateType()
        $v = (Get-Item -LiteralPath $env:PROGRAM).VersionInfo
        $v.CompanyName, $v.FileDescription, $v.ProductName, $v.ProductVersion,
          $v.FileVersionRaw.ToString(), $v.LegalCopyright, $v.OriginalFilename,
          $icons::ExtractIconExW($env:PROGRAM, -1, [IntPtr]::Zero, [IntPtr]::Zero, 0)";
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

/// A home with a login saved where steamcmd keeps it on this system, for `build_bot`, remembered.
fn saved_login(home: &Path) {
    let config = steamcmd::saved_login(home, Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    fs::write(home.join("account"), "build_bot\n").unwrap();
}

/// A home with the login `saved_login` makes, whose steamcmd keeps what it was started with in
/// `args`, says `said` and exits `code`.
fn checking(said: &str, code: u8) -> tempfile::TempDir {
    let home = answering(said, code);
    saved_login(home.path());
    home
}

/// [`checking`] with no login saved.
fn answering(said: &str, code: u8) -> tempfile::TempDir {
    faked_with(&format!(
        "#!/bin/sh\necho \"$*\" > \"$HOME/args\"\nprintf '%s\\r\\n' '{said}'\nexit {code}\n"
    ))
}

/// The login `saved_login` makes, packed as `steamship ci` packs it, on 2026-09-28.
fn packed_login() -> String {
    let home = tempfile::tempdir().unwrap();
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(config, "\"token\" \"a_saved_login_token_0123456789\"").unwrap();
    let account = Account::parse("build_bot").unwrap();
    let login = ci::Login::saved(home.path(), Platform::THIS, account).unwrap();
    login.packed("2026-09-28").to_string()
}

/// A steamcmd that, asked for app 1000's info, prints it as [`app_info`] does with the
/// `ReleaseState` `state`, and asked to build, in the same run or alone, has Valve build the upload
/// as `BuildID` 4242, as [`BUILT`] does, and adds a line to `builds` in the home. Each run adds its
/// arguments to `runs` there.
fn released_as(state: &str) -> String {
    format!(
        "#!/bin/sh\n\
         echo \"$*\" >> \"$HOME/runs\"\n\
         case \" $* \" in *' +app_info_print '*)\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
         printf '\"1000\"\\n{{\\n\\t\"common\"\\n\\t{{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"ReleaseState\"\\t\\t\"{state}\"\\n\\t}}\\n}}\\n'\n\
         echo 'Unloading Steam API...OK'\n\
         case \" $* \" in *' +run_app_build '*) ;; *) exit 0;; esac;;\n\
         esac\n\
         echo built >> \"$HOME/builds\"\n\
         output=\"$HOME/apps/1000/output\"\n\
         mkdir -p \"$output\"\n\
         echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\"\n\
         echo 'Logging in user build_bot to Steam Public...OK'\n"
    )
}

/// A steamcmd that prints app 1000's info as [`released_as`] does, cut off after the app's name,
/// and asked to build has Valve build the upload as `BuildID` 4242.
const CUT_OFF: &str = "#!/bin/sh\n\
    case \" $* \" in *' +run_app_build '*)\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\";;\n\
    esac\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
    echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
    printf '\"1000\"\\n{\\n\\t\"common\"\\n\\t{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n";

/// What steamship says of the info [`CUT_OFF`] prints.
const CUT_OFF_SAID: &str = "steamcmd printed app info for app 1000 that cannot be read (line 6, \
                            column 1: a block is missing its closing })";

/// The one line of JSON a `--json` command printed on standard output, which must be all there
/// is.
fn summary_of(stdout: &str) -> serde_json::Value {
    let mut lines = stdout.lines();
    let line = lines.next().unwrap_or_default();
    assert_eq!(lines.next(), None, "one line only: {stdout}");
    serde_json::from_str(line).unwrap()
}

/// The value at the JSON pointer `path` in `line`, or null when there is none.
fn at(line: &serde_json::Value, path: &str) -> serde_json::Value {
    line.pointer(path).cloned().unwrap_or_default()
}

/// The text at the JSON pointer `path` in `line`, or nothing when it is not text.
fn text_at(line: &serde_json::Value, path: &str) -> String {
    line.pointer(path)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A home whose recorded steamcmd is a script that prints what it was started with and exits
/// with `STEAMSHIP_FAKE_EXIT`, or 0. With `STEAMSHIP_FAKE_CHANGE` set it first changes itself, as
/// an update would. With `STEAMSHIP_FAKE_SAVED` set it logs in with the login it saved.
fn faked() -> tempfile::TempDir {
    faked_with(
        "#!/bin/sh\n\
         [ -n \"$STEAMSHIP_FAKE_SAVED\" ] && echo 'Logging in using cached credentials.'\n\
         shown() { if command -v cygpath > /dev/null; then cygpath -w \"$1\"; else echo \"$1\"; fi; }\n\
         echo \"args: $*\"\n\
         echo \"home: $(shown \"$HOME\")\"\n\
         echo \"folder: $(shown \"$PWD\")\"\n\
         [ -n \"$STEAMSHIP_FAKE_CHANGE\" ] && echo >> \"$0\"\n\
         exit \"$((STEAMSHIP_FAKE_EXIT + 0))\"\n",
    )
}

/// A home whose recorded steamcmd is `script`.
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
/// every handle until it runs. Running `program` in those moments fails with "Text file busy".
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

/// Writes `program`, which Windows runs through Git's `sh` and so needs no executable bit.
#[cfg(windows)]
fn write_program(program: &Path, script: &str) {
    fs::write(program, script).unwrap();
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
    /// On Windows, a home with a fake steamcmd takes its line in Git's `sh`, as on Unix, and any
    /// other in `cmd`.
    fn start_with(line: &str, home: &Path, variables: &[(&str, &str)]) -> Self {
        #[cfg(windows)]
        let fake = home.join(install::FOLDER).join("steamcmd.sh").is_file();
        #[cfg(windows)]
        let (shell, args) = if fake {
            (shell(), &["-c"][..])
        } else {
            (
                PathBuf::from(r"C:\Windows\System32\cmd.exe"),
                &["/d", "/s", "/c"][..],
            )
        };
        #[cfg(unix)]
        let (shell, args) = (PathBuf::from("/bin/sh"), &["-c"][..]);
        let args: Vec<OsString> = args
            .iter()
            .copied()
            .chain([line])
            .map(OsString::from)
            .collect();
        #[cfg(windows)]
        let faked = if fake {
            vec![
                ("STEAMSHIP_FAKE_STEAMCMD", shell.clone().into_os_string()),
                ("HOME", home.as_os_str().to_owned()),
            ]
        } else {
            Vec::new()
        };
        #[cfg(unix)]
        let faked = Vec::<(&str, OsString)>::new();
        let environment: Vec<(OsString, OsString)> =
            iter::once(("STEAMSHIP_HOME", home.as_os_str()))
                .chain(
                    variables
                        .iter()
                        .chain(&common::store::environment())
                        .map(|&(name, value)| (name, OsStr::new(value))),
                )
                .map(|(name, value)| (OsString::from(name), value.to_owned()))
                .chain(
                    faked
                        .into_iter()
                        .map(|(name, value)| (OsString::from(name), value)),
                )
                .collect();
        let (terminal, mut output, input) =
            Terminal::start(&shell, &args, &environment, home).unwrap();
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
    assert!(
        stdout.contains("  login     none saved\n  account   none remembered\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("99.0.0"), "{stdout}");
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

/// A token that logs the account in without a password, and a value from its local settings.
const SECRETS: [&str; 2] = [
    "eyJhbGciOiJFZERTQSJ9.token.that.logs.in.without.a.password",
    "localconfig_secret_value_0123456789",
];

/// Every file under `folder` but those in `except`, relative to `folder` with `/` between its
/// folders on every system, with its contents.
fn written(folder: &Path, except: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(folder)
                .unwrap()
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
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

const KEY: &str = "0123456789abcdef0123456789abcdef";

/// The variables that give a test a Web API key, no update check and a stand-in at `host`.
const fn web_api_variables(host: &str) -> [(&'static str, &str); 3] {
    [
        ("STEAMSHIP_WEB_API_KEY", KEY),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        (STAND_IN, host),
    ]
}

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
    web_api_answering(|_| answers)
}

/// `web_api`, with answers that `script` writes knowing the stand-in's own address.
fn web_api_answering<Body, Script>(script: Script) -> (String, mpsc::Receiver<String>)
where
    Body: AsRef<[u8]> + Send + 'static,
    Script: FnOnce(&str) -> Vec<(&'static str, Body)>,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let answers = script(&format!("http://{address}"));
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

/// `steamship login --web-api-key-only` with the Web API at `host`, `typed` piped to its prompt, and
/// `variables` set besides.
fn login_with_key(
    home: &Path,
    host: &str,
    typed: &str,
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    steamship_typing(
        &["login", "--web-api-key-only"],
        home,
        &[&[(STAND_IN, host)], variables].concat(),
        &format!("{typed}\n"),
    )
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

/// `GetAppBetas` as Steam answered for Falcon Ridge Overseer.
const DESCRIBED: &str = r#"{"response": {"betas": {
    "public": {"BuildID": 20000001, "Description": "Public default branch", "ReqPassword": 0},
    "testing": {"BuildID": 20000099, "Description": "test1", "ReqPassword": 1}
}, "result": 1, "message": ""}}"#;

/// `UpdateAppBranchDescription` as Steam answered it.
const UPDATED: &str =
    r#"{"response": {"result": 1, "message": "Successfully updated beta branch description"}}"#;

/// `GetAppBetas` showing build 4242 live on the default branch and testing, and 4100 on a branch
/// that takes a password.
const FETCHABLE: &str = r#"{"response": {"result": 1, "betas": {
    "public": {"BuildID": 4242},
    "testing": {"BuildID": 4242, "Description": "0.3.0"},
    "friends": {"BuildID": 4100, "ReqPassword": 1}
}}}"#;

/// `steamship fetch` with `args` from a home whose steamcmd is `home`'s, with the Web API
/// answering [`FETCHABLE`] once, and `variables` set besides.
fn fetching(
    home: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    fetching_typed(home, args, variables, None)
}

/// [`fetching`], with `typed`, when given, piped to it as a script would type it.
fn fetching_typed(
    home: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
    typed: Option<&str>,
) -> (Option<i32>, String, String) {
    let (host, _requests) = web_api(vec![("200 OK", FETCHABLE)]);
    let mut all = vec![
        ("STEAMSHIP_WEB_API_KEY", KEY),
        (STAND_IN, host.as_str()),
        ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
    ];
    all.extend_from_slice(variables);
    let mut given = vec!["fetch"];
    given.extend_from_slice(args);
    typed.map_or_else(
        || steamship(&given, Some(home), &all),
        |typed| steamship_typing(&given, home, &all, typed),
    )
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

/// A game laid out as games-kit lays one out, with `toml` as its `steamship.toml` when given,
/// committed and pushed to an origin of its own beside it, and the game's folder.
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
    with_origin(temp.path(), &root);
    pushed(&root, "The game");
    (temp, root)
}

/// Commits everything in the game at `root` and pushes it to its origin.
fn pushed(root: &Path, message: &str) {
    git(root, &["add", "--all"]);
    commit(root, message);
    git(
        root,
        &[
            "push",
            "--quiet",
            "origin",
            "main",
            "+main:refs/heads/green",
        ],
    );
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
fn the_help_names_allow_ungreen_and_says_players_update_to_a_rolled_back_build() {
    for (command, said) in [
        ("upload", "--allow-ungreen"),
        (
            "rollback",
            "Players on the branch then update to that build.",
        ),
    ] {
        let (code, stdout, stderr) = steamship(&[command, "--help"], Some(Path::new(".")), &[]);
        assert_eq!(code, Some(0_i32), "{command}: {stderr}");
        assert!(stdout.contains(said), "{command}: {stdout}");
    }
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
