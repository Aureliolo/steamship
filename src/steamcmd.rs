//! How steamcmd is started: which file, with what around it, with which commands and for how
//! long.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::account::Account;
use crate::conversation;
use crate::drm;
use crate::fetch;
use crate::install;
use crate::platform::Platform;
use crate::run::{self, Finished};
#[cfg(unix)]
use crate::unix::{Terminal, held_elsewhere};
#[cfg(windows)]
use crate::windows::{Terminal, held_elsewhere};

/// The file that starts steamcmd in the install at `root`. On Linux and macOS that is Valve's
/// script, which sets up the library path steamcmd needs.
#[must_use]
pub fn program(root: &Path, platform: Platform) -> PathBuf {
    root.join(match platform {
        Platform::Windows => "steamcmd.exe",
        Platform::MacOs | Platform::Linux => "steamcmd.sh",
    })
}

/// What steamcmd's environment differs by from steamship's.
///
/// On Linux, steamcmd keeps its login, logs and cache in `$HOME/Steam`, which would be the same
/// folder as the user's own Steam's and outside the home steamship keeps to its user, so `HOME`
/// is the steamship home instead.
#[must_use]
pub fn environment(home: &Path, platform: Platform) -> Vec<(OsString, OsString)> {
    match platform {
        Platform::Windows => Vec::new(),
        Platform::MacOs | Platform::Linux => {
            vec![(OsString::from("HOME"), home.as_os_str().to_owned())]
        }
    }
}

/// Where steamcmd keeps its state under `home` on `platform`: its own folder on Windows,
/// `$HOME/Steam` on Linux, and the folder macOS keeps application data in.
#[must_use]
pub fn state_folder(home: &Path, platform: Platform) -> PathBuf {
    match platform {
        Platform::Windows => home.join(install::FOLDER),
        Platform::Linux => home.join("Steam"),
        Platform::MacOs => home
            .join("Library")
            .join("Application Support")
            .join("Steam"),
    }
}

/// The file in which steamcmd keeps the token that logs the account in with no password.
#[must_use]
pub fn saved_login(home: &Path, platform: Platform) -> PathBuf {
    state_folder(home, platform)
        .join("config")
        .join("config.vdf")
}

/// The log steamcmd adds to once its API has loaded, before it runs the first command it was
/// given.
#[must_use]
pub fn connection_log(home: &Path, platform: Platform) -> PathBuf {
    state_folder(home, platform)
        .join("logs")
        .join("connection_log.txt")
}

/// Whether steamcmd in `home` can add to its connection log now. A steamcmd still running from
/// that home keeps the log to itself on Windows, and a second one then works on without a line
/// in it.
///
/// # Errors
///
/// When the log is there and cannot be opened to add to. The error names the log and says what
/// to do: to end the program that holds it, or to let this user write to it.
pub fn log_is_free(home: &Path, platform: Platform) -> io::Result<()> {
    let log = connection_log(home, platform);
    match fs::File::options().append(true).open(&log) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            let step = if held_elsewhere(&error) {
                HELD
            } else {
                UNWRITABLE
            };
            Err(io::Error::new(
                error.kind(),
                format!(
                    "cannot add to its log {}: {error}. {step}, then run steamship again",
                    log.display()
                ),
            ))
        }
    }
}

/// What to do about a connection log another program holds open.
const HELD: &str =
    "End any steamcmd left running from the same folder, or whatever else holds that file";

/// What to do about a connection log that cannot be written for any other reason.
const UNWRITABLE: &str = "Let this user write to that file";

/// Every place steamcmd could have kept its state under `home`, whichever system wrote it, so that
/// nothing it saved is overlooked.
fn state_folders(home: &Path) -> [PathBuf; 3] {
    [Platform::Windows, Platform::Linux, Platform::MacOs]
        .map(|platform| state_folder(home, platform))
}

/// The files that hold steamcmd's saved login in `home`, of those that are there.
///
/// `config.vdf` holds the token that logs the account in with no password, and each user's
/// `localconfig.vdf` more about the account.
///
/// # Errors
///
/// When a folder that would hold one is there but cannot be read.
pub fn login_files(home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for folder in state_folders(home) {
        // Whatever is there counts, a file that cannot be read included: skipping it would let
        // the secrets in it through.
        let config = folder.join("config").join("config.vdf");
        if config.try_exists()? {
            files.push(config);
        }
        let users = match fs::read_dir(folder.join("userdata")) {
            Ok(users) => users,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for user in users {
            let local = user?.path().join("config").join("localconfig.vdf");
            if local.try_exists()? {
                files.push(local);
            }
        }
    }
    Ok(files)
}

/// An error that names `file`, which starts steamcmd, since the system's reason alone does not.
fn named(file: &Path) -> impl FnOnce(io::Error) -> io::Error {
    move |error| io::Error::new(error.kind(), format!("{}: {error}", file.display()))
}

/// What steamship says of a steamcmd it stopped twice before it had started.
const NOT_STARTED: &str = "did not finish starting up and was stopped. Run steamship again";

/// Whether a run of steamcmd was stopped before it had started, with nothing telling steamship to
/// stop and steamcmd not having said that it never will finish.
fn stalled(finished: &Finished, startup: run::Startup<'_>) -> bool {
    let said = String::from_utf8_lossy(&finished.output);
    finished.code.is_none()
        && run::told().is_none()
        && !startup.begun()
        && !HOPELESS.iter().any(|words| said.contains(words))
}

/// What `run` gives, called once more when steamcmd was stopped before it had started: until then
/// it has asked Steam nothing, so a second run repeats nothing.
///
/// # Errors
///
/// When `run` fails, when both runs were stopped before steamcmd had started, or when a test
/// build cannot empty its ready file.
fn started<Run>(startup: run::Startup<'_>, mut run: Run) -> io::Result<Finished>
where
    Run: FnMut() -> io::Result<Finished>,
{
    #[cfg(feature = "fake-steamcmd")]
    startup.rearm()?;
    let first = run()?;
    if !stalled(&first, startup) {
        return Ok(first);
    }
    #[cfg(feature = "fake-steamcmd")]
    startup.rearm()?;
    let second = run()?;
    if stalled(&second, startup) {
        return Err(io::Error::new(io::ErrorKind::TimedOut, NOT_STARTED));
    }
    Ok(second)
}

/// The commands of one run of steamcmd with nobody at it, the longest it may take over them, and
/// the longest it may take to start.
#[derive(Debug)]
pub struct Commands {
    args: Vec<OsString>,
    limit: Duration,
    startup: Duration,
}

impl Commands {
    /// Runs the steamcmd in `home` with these commands, as [`run::run`] runs a program.
    ///
    /// It starts in its own folder with [`environment`] around it. It is stopped at the commands'
    /// limit, or once it says that it will never reach Steam. It is not started while its
    /// [`connection_log`] cannot be added to. It is stopped as well when it has added nothing to
    /// that log within the commands' time to start, and is then run once more.
    ///
    /// # Errors
    ///
    /// When steamcmd cannot be started, its output cannot be read, its log cannot be added to, or
    /// it was stopped twice before it had started. The error names the file that starts steamcmd.
    pub fn run(&self, home: &Path) -> io::Result<Finished> {
        let root = home.join(install::FOLDER);
        let file = program(&root, Platform::THIS);
        let log = connection_log(home, Platform::THIS);
        let startup = run::Startup::new(self.startup, &log);
        started(startup, || {
            log_is_free(home, Platform::THIS)?;
            run::run(
                &file,
                &self.args,
                &environment(home, Platform::THIS),
                &root,
                self.limit,
                HOPELESS,
                Some(startup),
            )
        })
        .map_err(named(&file))
    }

    /// Runs the steamcmd in `home` with these commands at a console of its own and types
    /// `command` at its prompt, as [`fetch::typed`] does with `secret` hidden.
    ///
    /// A password typed there is seen by no other program, as one on the command line is. The
    /// run starts and is stopped as [`Commands::run`] has it.
    ///
    /// # Errors
    ///
    /// When steamcmd cannot be started, its console cannot be read or typed into, or it cannot be
    /// waited for, or when its log cannot be added to, or when it was stopped twice before it had
    /// started. The error names the file that starts steamcmd.
    pub fn typed(
        &self,
        home: &Path,
        command: &Zeroizing<String>,
        secret: &Zeroizing<String>,
    ) -> io::Result<Finished> {
        let root = home.join(install::FOLDER);
        let file = program(&root, Platform::THIS);
        let log = connection_log(home, Platform::THIS);
        let startup = run::Startup::new(self.startup, &log);
        started(startup, || {
            log_is_free(home, Platform::THIS)?;
            self.typed_at(&file, home, startup, command.clone(), secret.clone())
        })
        .map_err(named(&file))
    }

    /// Runs `file` in its folder under `home` with these commands at a console of its own and
    /// types `command` at its prompt, as [`fetch::typed`] does with `secret` hidden. It is
    /// stopped at the commands' limit, or sooner when `startup` has not begun in its time.
    fn typed_at(
        &self,
        file: &Path,
        home: &Path,
        startup: run::Startup<'_>,
        command: Zeroizing<String>,
        secret: Zeroizing<String>,
    ) -> io::Result<Finished> {
        let (terminal, mut output, mut input) = Terminal::start(
            file,
            &self.args,
            &environment(home, Platform::THIS),
            &home.join(install::FOLDER),
        )?;
        let (ended, ending) = mpsc::channel();
        let reading = thread::spawn(move || {
            let read = fetch::typed(&mut output, &mut input, &command, &secret, HOPELESS);
            // The receiver has gone only once a limit passed, and then nothing waits to hear.
            let _told = ended.send(());
            read
        });
        let in_time = run::waited(
            self.limit,
            &AtomicBool::new(false),
            Some(startup),
            |pause| {
                // A closed channel means the reading is over as well.
                Ok(match ending.recv_timeout(pause) {
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => Some(()),
                })
            },
        )?
        .is_some();
        // Dropping the terminal unwaited ends steamcmd, and with it the output being read.
        let terminal = in_time.then_some(terminal);
        let lines = reading
            .join()
            .unwrap_or_else(|panic| panic::resume_unwind(panic))?;
        let given_up = lines
            .last()
            .is_some_and(|line| HOPELESS.iter().any(|words| line.contains(words)));
        let code = match terminal {
            Some(terminal) if !given_up => terminal.wait()?,
            _ => None,
        };
        Ok(Finished {
            code,
            output: lines.join("\n").into_bytes(),
        })
    }
}

/// The commands that log `account` in and quit: steamcmd asks for the password and a Steam
/// Guard code itself, and a failure ends the run rather than leaving it at its prompt.
#[must_use]
pub fn login(account: &Account) -> Vec<OsString> {
    [
        "+@ShutdownOnFailedCommand",
        "1",
        "+login",
        account.name(),
        "+quit",
    ]
    .map(OsString::from)
    .into()
}

/// The commands that log `account` in with the login steamcmd saved and quit, which is how the
/// saved login is checked with Steam. Like [`upload`], it never waits at a prompt.
#[must_use]
pub fn check_login(account: &Account) -> Commands {
    Commands {
        args: [
            "+@ShutdownOnFailedCommand",
            "1",
            "+@NoPromptForPassword",
            "1",
            "+login",
            account.name(),
            "+quit",
        ]
        .map(OsString::from)
        .into(),
        limit: CHECK_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The commands that log `account` in with the login steamcmd saved, print `app`'s settings as
/// Steam shows them to that account, and quit.
///
/// The settings are asked for afresh rather than read from steamcmd's cache. Like [`upload`], it
/// never waits at a prompt.
#[must_use]
pub fn app_info(account: &Account, app: u32) -> Commands {
    let app = app.to_string();
    Commands {
        args: [
            "+@ShutdownOnFailedCommand",
            "1",
            "+@NoPromptForPassword",
            "1",
            "+login",
            account.name(),
            "+app_info_update",
            "1",
            "+app_info_print",
            &app,
            "+quit",
        ]
        .map(OsString::from)
        .into(),
        limit: CHECK_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The commands that log `account` in with the login steamcmd saved, wrap `input` in Steam DRM
/// into `output`, and quit.
///
/// Valve's servers do the wrapping, for `app`, of the Windows executable. Like [`upload`], it
/// never waits at a prompt.
#[must_use]
pub fn drm_wrap(
    account: &Account,
    app: u32,
    input: &Path,
    output: &Path,
    mode: drm::Mode,
) -> Commands {
    let mut args: Vec<OsString> = [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        account.name(),
        "+drm_wrap",
    ]
    .map(OsString::from)
    .into();
    args.push(app.to_string().into());
    args.push(input.as_os_str().to_owned());
    args.push(output.as_os_str().to_owned());
    args.extend(["drmtoolp", mode.flags(), "+quit"].map(OsString::from));
    Commands {
        args,
        limit: WRAP_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The commands that log `account` in with the login steamcmd saved, upload the Workshop item
/// `script` describes and quit. Like [`upload`], it never waits at a prompt.
#[must_use]
pub fn workshop(account: &Account, script: &Path) -> Commands {
    let mut args: Vec<OsString> = [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        account.name(),
        "+workshop_build_item",
    ]
    .map(OsString::from)
    .into();
    args.push(script.as_os_str().to_owned());
    args.push(OsString::from("+quit"));
    Commands {
        args,
        limit: UPLOAD_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The commands that log `account` in with the login steamcmd saved, fetch `app` into `folder`,
/// and quit. Like [`upload`], it never waits at a prompt.
///
/// The build is the one live on `branch`, `public` for the default, as Steam serves it to
/// `system`, and every file of it already in the folder is checked. The branch is always named:
/// steamcmd keeps the build account's last choice of branch for each app, and without `-beta`
/// fetches that one again. A branch's `password` goes on the command line, where any program on
/// the machine can read it while steamcmd runs; [`fetch_console`] keeps it off.
#[must_use]
pub fn fetch(
    account: &Account,
    app: u32,
    branch: &str,
    password: Option<&str>,
    system: &str,
    folder: &Path,
) -> Commands {
    let mut args = fetch_console(account, system, folder).args;
    args.push("+app_update".into());
    args.push(app.to_string().into());
    args.extend(["-beta", branch].map(OsString::from));
    if let Some(password) = password {
        args.extend(["-betapassword", password].map(OsString::from));
    }
    args.extend(["validate", "+quit"].map(OsString::from));
    Commands {
        args,
        limit: FETCH_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The commands that log `account` in as [`fetch()`] does and leave steamcmd at its console, for
/// the fetch to be typed there.
///
/// The `system` and the `folder` are set before logging in, which Valve's documentation asks of
/// the folder.
#[must_use]
pub fn fetch_console(account: &Account, system: &str, folder: &Path) -> Commands {
    let mut args: Vec<OsString> = [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+@sSteamCmdForcePlatformType",
        system,
        "+force_install_dir",
    ]
    .map(OsString::from)
    .into();
    args.push(folder.as_os_str().to_owned());
    args.extend(["+login", account.name()].map(OsString::from));
    Commands {
        args,
        limit: FETCH_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

/// The longest a fetch may run: a large build over a slow line takes hours.
const FETCH_LIMIT: Duration = Duration::from_hours(6);

/// The longest a check of the saved login may run: logging in takes seconds.
const CHECK_LIMIT: Duration = Duration::from_mins(2);

/// The longest a DRM wrap may run: the executable goes to Valve's servers and back, which for a
/// large one over a slow line takes many minutes.
const WRAP_LIMIT: Duration = Duration::from_hours(1);

/// The longest an upload may run before it is taken to have hung and is stopped. A large first
/// upload over a slow line takes hours. steamcmd waiting at a prompt would wait forever.
const UPLOAD_LIMIT: Duration = Duration::from_hours(6);

/// What steamcmd says when it will never reach Steam, only wait.
///
/// It is stopped then rather than at its limit. With no certificates to check Steam's servers
/// against (on Linux, with the system's certificate store missing), it says this at once and then
/// waits to connect for good.
const HOPELESS: &[&str] = &[conversation::NO_CERTIFICATES];

/// The longest steamcmd may take to add to its connection log. In 224 starts on CI's four systems
/// it took 3 s at most. On macOS one start in 30 CI runs hung before it, having asked Steam
/// nothing.
const STARTUP_LIMIT: Duration = Duration::from_mins(1);

/// The commands that log `account` in with the login steamcmd saved, print the info of the app
/// `info_of` names, as [`app_info`] does, build `script` and quit.
///
/// The info is asked in the same run because each login to Steam takes seconds. With no saved
/// login, or an expired one, steamcmd fails rather than asking for a password nobody is there to
/// type.
#[must_use]
pub fn upload(account: &Account, script: &Path, info_of: Option<u32>) -> Commands {
    let mut args: Vec<OsString> = [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        account.name(),
    ]
    .map(OsString::from)
    .into();
    if let Some(app) = info_of {
        args.extend(["+app_info_update", "1", "+app_info_print"].map(OsString::from));
        args.push(OsString::from(app.to_string()));
    }
    args.push(OsString::from("+run_app_build"));
    args.push(script.as_os_str().to_owned());
    args.push(OsString::from("+quit"));
    Commands {
        args,
        limit: UPLOAD_LIMIT,
        startup: STARTUP_LIMIT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::unix;
    use std::time::Instant;

    #[test]
    fn starts_valves_program_on_windows_and_valves_script_elsewhere() {
        let root = Path::new("steamcmd");
        assert_eq!(
            [Platform::Windows, Platform::MacOs, Platform::Linux]
                .map(|platform| program(root, platform)),
            [
                root.join("steamcmd.exe"),
                root.join("steamcmd.sh"),
                root.join("steamcmd.sh")
            ]
        );
    }

    #[test]
    fn keeps_steamcmds_state_in_the_home_where_it_would_follow_home() {
        let home = Path::new("home");
        assert_eq!(
            environment(home, Platform::Windows),
            Vec::<(OsString, OsString)>::new()
        );
        for platform in [Platform::MacOs, Platform::Linux] {
            assert_eq!(
                environment(home, platform),
                [(OsString::from("HOME"), OsString::from("home"))]
            );
        }
    }

    #[test]
    fn uploads_with_the_saved_login_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        let script = Path::new("home (x86)/apps/1/app_build.vdf");
        assert_eq!(
            upload(&account, script, None).args,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+login",
                "build_bot",
                "+run_app_build",
                "home (x86)/apps/1/app_build.vdf",
                "+quit"
            ]
        );
        assert_eq!(
            upload(&account, script, Some(1)).args,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+login",
                "build_bot",
                "+app_info_update",
                "1",
                "+app_info_print",
                "1",
                "+run_app_build",
                "home (x86)/apps/1/app_build.vdf",
                "+quit"
            ]
        );
    }

    #[test]
    fn wraps_in_drm_with_the_saved_login_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        for (mode, flags) in [(drm::Mode::Default, "0"), (drm::Mode::Compatibility, "6")] {
            assert_eq!(
                drm_wrap(
                    &account,
                    480,
                    Path::new("my game/game.exe"),
                    Path::new("my game/.game.exe.steamship-drm"),
                    mode
                )
                .args,
                [
                    "+@ShutdownOnFailedCommand",
                    "1",
                    "+@NoPromptForPassword",
                    "1",
                    "+login",
                    "build_bot",
                    "+drm_wrap",
                    "480",
                    "my game/game.exe",
                    "my game/.game.exe.steamship-drm",
                    "drmtoolp",
                    flags,
                    "+quit"
                ]
            );
        }
    }

    #[test]
    fn uploads_a_workshop_item_with_the_saved_login_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        let script = Path::new("home (x86)/workshop/480/workshop_item.vdf");
        assert_eq!(
            workshop(&account, script).args,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+login",
                "build_bot",
                "+workshop_build_item",
                "home (x86)/workshop/480/workshop_item.vdf",
                "+quit"
            ]
        );
    }

    #[test]
    fn fetches_a_branch_for_a_system_into_its_folder_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        let folder = Path::new("home (x86)/fetched/480/testing-linux");
        assert_eq!(
            fetch(&account, 480, "testing", None, "linux", folder).args,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+@sSteamCmdForcePlatformType",
                "linux",
                "+force_install_dir",
                "home (x86)/fetched/480/testing-linux",
                "+login",
                "build_bot",
                "+app_update",
                "480",
                "-beta",
                "testing",
                "validate",
                "+quit"
            ]
        );
        let default = fetch(&account, 480, "public", None, "windows", folder).args;
        assert!(
            default.ends_with(
                &["+app_update", "480", "-beta", "public", "validate", "+quit"].map(OsString::from)
            ),
            "{default:?}"
        );
    }

    #[test]
    fn a_password_given_for_the_command_line_follows_its_branch_there() {
        let account = Account::parse("build_bot").unwrap();
        let args = fetch(
            &account,
            480,
            "friends",
            Some("open sesame"),
            "linux",
            Path::new("f"),
        )
        .args;
        assert!(
            args.ends_with(
                &[
                    "+app_update",
                    "480",
                    "-beta",
                    "friends",
                    "-betapassword",
                    "open sesame",
                    "validate",
                    "+quit"
                ]
                .map(OsString::from)
            ),
            "{args:?}"
        );
    }

    #[test]
    fn a_fetch_typed_at_the_console_logs_in_where_the_fetch_would_and_waits_there() {
        let account = Account::parse("build_bot").unwrap();
        let folder = Path::new("home (x86)/fetched/480/friends-linux");
        let console = fetch_console(&account, "linux", folder).args;
        assert_eq!(
            console,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+@sSteamCmdForcePlatformType",
                "linux",
                "+force_install_dir",
                "home (x86)/fetched/480/friends-linux",
                "+login",
                "build_bot"
            ]
        );
        assert!(
            fetch(&account, 480, "friends", None, "linux", folder)
                .args
                .starts_with(&console)
        );
    }

    #[test]
    fn checks_the_saved_login_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        assert_eq!(
            check_login(&account).args,
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+@NoPromptForPassword",
                "1",
                "+login",
                "build_bot",
                "+quit"
            ]
        );
    }

    #[test]
    fn logs_in_as_the_account_and_stops_at_a_failure() {
        let account = Account::parse("build_bot").unwrap();
        assert_eq!(
            login(&account),
            [
                "+@ShutdownOnFailedCommand",
                "1",
                "+login",
                "build_bot",
                "+quit"
            ]
        );
    }

    #[test]
    fn the_connection_log_is_in_the_logs_folder_steamcmd_names_on_each_system() {
        let home = Path::new("home");
        assert_eq!(
            [Platform::Windows, Platform::Linux, Platform::MacOs]
                .map(|platform| connection_log(home, platform)),
            [
                home.join("steamcmd/logs/connection_log.txt"),
                home.join("Steam/logs/connection_log.txt"),
                home.join("Library/Application Support/Steam/logs/connection_log.txt")
            ]
        );
    }

    #[test]
    fn a_connection_log_that_is_not_there_yet_is_free_and_left_unmade() {
        let home = tempfile::tempdir().unwrap();
        let log = connection_log(home.path(), Platform::THIS);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        log_is_free(home.path(), Platform::THIS).unwrap();
        assert!(!log.exists(), "{} was made", log.display());
    }

    /// A home whose connection log holds one line.
    fn logged() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let log = connection_log(home.path(), Platform::THIS);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, "an earlier run\n").unwrap();
        (home, log)
    }

    /// Makes `log` a file nobody may write to, as a home another user owns leaves it.
    fn read_only(log: &Path) {
        let mut permissions = fs::metadata(log).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(log, permissions).unwrap();
    }

    /// The end of a refusal for a log that cannot be written.
    const WRITE: &str = ". Let this user write to that file, then run steamship again";

    #[test]
    fn a_connection_log_nobody_may_write_to_is_not_free_and_the_error_says_to_let_this_user_write()
    {
        let (home, log) = logged();
        log_is_free(home.path(), Platform::THIS).unwrap();
        read_only(&log);
        let error = log_is_free(home.path(), Platform::THIS)
            .unwrap_err()
            .to_string();
        let start = format!("cannot add to its log {}: ", log.display());
        assert!(
            error.starts_with(&start) && error.ends_with(WRITE),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), "an earlier run\n");
    }

    #[cfg(windows)]
    #[test]
    fn a_connection_log_another_program_holds_is_not_free_and_the_error_says_to_end_it() {
        use std::os::windows::fs::OpenOptionsExt as _;
        let (home, log) = logged();
        let holding = fs::File::options()
            .read(true)
            .share_mode(0)
            .open(&log)
            .unwrap();
        let error = log_is_free(home.path(), Platform::THIS)
            .unwrap_err()
            .to_string();
        drop(holding);
        let start = format!("cannot add to its log {}: ", log.display());
        let end = ". End any steamcmd left running from the same folder, or whatever else holds \
                   that file, then run steamship again";
        assert!(error.starts_with(&start) && error.ends_with(end), "{error}");
        log_is_free(home.path(), Platform::THIS).unwrap();
    }

    /// The line that leaves the file `ran` in the folder steamcmd runs in.
    const RAN: &str = "echo ran > ran";

    /// What a refused run's error must read for the stand-in in `home`, and that nothing ran.
    fn refused(home: &Path, error: &io::Error) {
        let root = home.join(install::FOLDER);
        let start = format!(
            "{}: cannot add to its log {}: ",
            program(&root, Platform::THIS).display(),
            connection_log(home, Platform::THIS).display()
        );
        let error = error.to_string();
        assert!(
            error.starts_with(&start) && error.ends_with(WRITE),
            "{error}"
        );
        assert!(!root.join("ran").exists());
    }

    #[test]
    fn a_run_whose_connection_log_cannot_be_added_to_is_refused_and_steamcmd_never_starts() {
        let (home, args) = stand_in(RAN);
        read_only(&connection_log(home.path(), Platform::THIS));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_mins(1),
        };
        let error = commands.run(home.path()).unwrap_err();
        refused(home.path(), &error);
    }

    #[test]
    fn a_run_typed_into_whose_connection_log_cannot_be_added_to_is_refused_and_steamcmd_never_starts()
     {
        let (home, args) = stand_in(RAN);
        read_only(&connection_log(home.path(), Platform::THIS));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_mins(1),
        };
        let error = commands
            .typed(home.path(), &Zeroizing::default(), &Zeroizing::default())
            .unwrap_err();
        refused(home.path(), &error);
    }

    #[test]
    fn every_command_may_run_as_long_as_its_work_takes_and_no_longer() {
        let account = Account::parse("build_bot").unwrap();
        let file = Path::new("f");
        assert_eq!(
            [check_login(&account).limit, app_info(&account, 480).limit],
            [Duration::from_mins(2); 2]
        );
        assert_eq!(
            drm_wrap(&account, 480, file, file, drm::Mode::Default).limit,
            Duration::from_hours(1)
        );
        assert_eq!(
            [
                upload(&account, file, None).limit,
                workshop(&account, file).limit,
                fetch(&account, 480, "public", None, "linux", file).limit,
                fetch_console(&account, "linux", file).limit
            ],
            [Duration::from_hours(6); 4]
        );
        assert_eq!(
            [
                check_login(&account).startup,
                app_info(&account, 480).startup,
                drm_wrap(&account, 480, file, file, drm::Mode::Default).startup,
                upload(&account, file, None).startup,
                workshop(&account, file).startup,
                fetch(&account, 480, "public", None, "linux", file).startup,
                fetch_console(&account, "linux", file).startup
            ],
            [Duration::from_mins(1); 7]
        );
    }

    /// A home whose steamcmd is the system's shell and whose connection log holds one line, and
    /// the arguments that have the shell run `line`. On Unix the stand-in is a link, since a file
    /// this process writes can be held open by a program another test starts meanwhile, and then
    /// refuses to run. Windows hands no file on to a program unasked, so a copy is safe there.
    /// Its ready file is there and empty.
    fn stand_in(line: &str) -> (tempfile::TempDir, Vec<OsString>) {
        let (home, log) = logged();
        fs::write(log.with_file_name(run::READY), "").unwrap();
        let root = home.path().join(install::FOLDER);
        fs::create_dir_all(&root).unwrap();
        let stand_in = program(&root, Platform::THIS);
        #[cfg(unix)]
        let args = {
            unix::make_link("/bin/sh", &stand_in).unwrap();
            vec!["-c".to_owned(), line.to_owned()]
        };
        #[cfg(windows)]
        let args = {
            let _: u64 = fs::copy(r"C:\Windows\System32\cmd.exe", &stand_in).unwrap();
            // cmd words its own messages, a prompt's among them, from the message file that
            // carries its program's name.
            let messages = root.join("en-US");
            fs::create_dir_all(&messages).unwrap();
            let _: u64 = fs::copy(
                r"C:\Windows\System32\en-US\cmd.exe.mui",
                messages.join("steamcmd.exe.mui"),
            )
            .unwrap();
            ["/d", "/s", "/c"]
                .map(str::to_owned)
                .into_iter()
                .chain([line.to_owned()])
                .collect::<Vec<String>>()
        };
        (home, args.into_iter().map(OsString::from).collect())
    }

    /// The line that waits half a minute: longer than the limit a test gives it, and over by
    /// itself when nothing stops it.
    const WAIT: &str = if cfg!(windows) {
        "ping -n 30 127.0.0.1 > nul"
    } else {
        "sleep 30"
    };

    /// The line that adds to steamcmd's connection log from the folder steamcmd runs in, as a
    /// steamcmd that has started does.
    fn starts() -> String {
        if cfg!(windows) {
            r"echo this run>> logs\connection_log.txt".to_owned()
        } else {
            let log = connection_log(Path::new(".."), Platform::THIS);
            format!("echo this run >> '{}'", log.display())
        }
    }

    /// The line that writes to the ready file from the folder steamcmd runs in, as a stand-in does
    /// once it has run its first commands.
    fn readies() -> String {
        if cfg!(windows) {
            format!(r"echo ready>> logs\{}", run::READY)
        } else {
            let log = connection_log(Path::new(".."), Platform::THIS);
            format!(
                "echo ready >> '{}'",
                log.with_file_name(run::READY).display()
            )
        }
    }

    /// `first`, then `second`, as the system's shell joins two commands.
    fn then(first: &str, second: &str) -> String {
        let joined = if cfg!(windows) { "& " } else { "; " };
        format!("{first}{joined}{second}")
    }

    /// A home whose stand-in steamcmd says `saying`, when given, then starts, is ready and waits.
    fn waiting(saying: Option<&str>) -> (tempfile::TempDir, Vec<OsString>) {
        let started = then(&starts(), &then(&readies(), WAIT));
        stand_in(&saying.map_or_else(
            || started.clone(),
            |words| then(&format!("echo {words}"), &started),
        ))
    }

    /// What steamship says of the stand-in in `home` when it never started.
    fn not_started(home: &Path) -> String {
        let file = program(&home.join(install::FOLDER), Platform::THIS);
        format!("{}: {NOT_STARTED}", file.display())
    }

    /// The line that adds one line to the file `runs` in the folder steamcmd runs in, once for
    /// each time steamcmd is run.
    const COUNT: &str = "echo run >> runs";

    /// How many times the stand-in in `home` was run.
    fn runs(home: &Path) -> usize {
        fs::read_to_string(home.join(install::FOLDER).join("runs"))
            .unwrap()
            .lines()
            .count()
    }

    /// The line that makes steamcmd's connection log a file nobody may write to, from the folder
    /// steamcmd runs in, as a steamcmd of another user would leave it as `starts` adds to it.
    fn keeps() -> String {
        if cfg!(windows) {
            r"attrib +R logs\connection_log.txt".to_owned()
        } else {
            let log = connection_log(Path::new(".."), Platform::THIS);
            format!("chmod 444 '{}'", log.display())
        }
    }

    /// The line that waits at its first run, as a steamcmd that hangs as it loads does, and runs
    /// `second` at its second.
    fn starts_the_second_time(second: &str) -> String {
        let ready = readies();
        if cfg!(windows) {
            format!("if exist again ({second}) else (type nul > again & {ready}& {WAIT})")
        } else {
            format!("if [ -e again ]; then {second}; else touch again; {ready}; {WAIT}; fi")
        }
    }

    /// The second run of a steamcmd that starts, is ready and ends with code 3.
    fn starts_and_ends() -> String {
        then(&starts(), &then(&readies(), "exit 3"))
    }

    /// The second run of a steamcmd that starts, shows its prompt, records the line typed at it in
    /// the file `typed`, echoes that line as steamcmd's console does and ends with code 3.
    fn starts_and_is_typed_at() -> String {
        if cfg!(windows) {
            format!(
                "{} & {}& set /p typed=Steam^> & call echo %typed%> typed& call echo %typed%& exit 3",
                starts(),
                readies()
            )
        } else {
            format!(
                "{}; {}; printf 'Steam>'; read -r typed; printf '%s' \"$typed\" > typed; \
                 echo \"$typed\"; exit 3",
                starts(),
                readies()
            )
        }
    }

    /// Input for a run typed into.
    fn quit() -> (Zeroizing<String>, Zeroizing<String>) {
        (Zeroizing::new("quit".to_owned()), Zeroizing::default())
    }

    /// `commands` run under `home` with `command` typed, which must end within 20 seconds: a run
    /// stopped at its limit whose output never ends would otherwise hold the test up for good.
    fn typed_in_time(
        commands: Commands,
        home: &Path,
        command: Zeroizing<String>,
        secret: Zeroizing<String>,
    ) -> io::Result<Finished> {
        let home = home.to_owned();
        run::within(Duration::from_secs(20), move || {
            commands.typed(&home, &command, &secret)
        })
    }

    #[test]
    fn a_second_run_starts_with_its_ready_file_empty() {
        let (_home, log) = logged();
        let ready = log.with_file_name(run::READY);
        let startup = run::Startup::new(Duration::from_secs(1), &log);
        fs::write(&ready, "an earlier run\n").unwrap();
        let ended = |code| Finished {
            code,
            output: Vec::new(),
        };
        let mut calls = 0_u32;
        let mut seen = Vec::new();
        let finished = started(startup, || {
            calls += 1;
            seen.push(fs::metadata(&ready)?.len());
            if calls == 1 {
                fs::write(&ready, "ready\n")?;
                Ok(ended(None))
            } else {
                Ok(ended(Some(3_i32)))
            }
        })
        .unwrap();
        assert_eq!(finished.code, Some(3_i32));
        assert_eq!(seen, [0, 0]);
        fs::remove_file(&ready).unwrap();
        let _unused: Finished = started(startup, || Ok(ended(Some(0_i32)))).unwrap();
        assert!(!ready.exists(), "a ready file was made");
    }

    #[test]
    fn a_run_is_stopped_at_the_limit_of_its_commands() {
        let (home, args) = waiting(None);
        let commands = Commands {
            args,
            limit: Duration::from_secs(1),
            startup: Duration::from_mins(1),
        };
        let started = Instant::now();
        let finished = commands.run(home.path()).unwrap();
        assert_eq!(finished.code, None);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_run_that_does_not_start_is_stopped_and_run_once_more() {
        let (home, args) = stand_in(&then(COUNT, &starts_the_second_time(&starts_and_ends())));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let started = Instant::now();
        let finished = commands.run(home.path()).unwrap();
        assert_eq!(finished.code, Some(3_i32));
        assert_eq!(runs(home.path()), 2);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_run_whose_log_is_taken_as_it_starts_is_not_run_again() {
        let (home, args) = stand_in(&then(COUNT, &then(&keeps(), &then(&readies(), WAIT))));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let started = Instant::now();
        let error = commands.run(home.path()).unwrap_err();
        refused(home.path(), &error);
        assert_eq!(runs(home.path()), 1);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_run_typed_into_that_does_not_start_is_stopped_and_run_once_more() {
        let second = starts_the_second_time(&starts_and_is_typed_at());
        let (home, args) = stand_in(&then(COUNT, &second));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let command = Zeroizing::new("login hunter2".to_owned());
        let secret = Zeroizing::new("hunter2".to_owned());
        let finished = typed_in_time(commands, home.path(), command, secret).unwrap();
        assert_eq!(finished.code, Some(3_i32));
        assert_eq!(runs(home.path()), 2);
        let typed = fs::read_to_string(home.path().join(install::FOLDER).join("typed")).unwrap();
        assert_eq!(typed.trim_end(), "login hunter2");
        let said = String::from_utf8_lossy(&finished.output);
        assert!(
            said.contains("<password>") && !said.contains("hunter2"),
            "{said}"
        );
    }

    #[test]
    fn a_run_typed_into_whose_log_is_taken_as_it_starts_is_not_run_again() {
        let (home, args) = stand_in(&then(COUNT, &then(&keeps(), &then(&readies(), WAIT))));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let (command, secret) = quit();
        let error = typed_in_time(commands, home.path(), command, secret).unwrap_err();
        refused(home.path(), &error);
        assert_eq!(runs(home.path()), 1);
    }

    #[test]
    fn a_run_that_does_not_start_in_time_is_stopped_and_is_an_error_that_names_the_file() {
        let (home, args) = stand_in(&then(COUNT, &then(&readies(), WAIT)));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let started = Instant::now();
        let error = commands.run(home.path()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), not_started(home.path()));
        assert_eq!(runs(home.path()), 2);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_run_typed_into_that_does_not_start_in_time_is_an_error_that_names_the_file() {
        let (home, args) = stand_in(&then(COUNT, &then(&readies(), WAIT)));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let (command, secret) = quit();
        let error = typed_in_time(commands, home.path(), command, secret).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), not_started(home.path()));
        assert_eq!(runs(home.path()), 2);
    }

    #[test]
    fn a_run_stopped_at_its_limit_after_it_started_is_no_error() {
        let (home, args) = stand_in(&then(COUNT, &then(&starts(), &then(&readies(), WAIT))));
        let commands = Commands {
            args,
            limit: Duration::from_secs(2),
            startup: Duration::from_secs(1),
        };
        let finished = commands.run(home.path()).unwrap();
        assert_eq!(finished.code, None);
        assert_eq!(runs(home.path()), 1);
    }

    #[test]
    fn a_run_that_says_it_never_will_finish_before_it_starts_is_no_error() {
        let said = format!("echo {}", conversation::NO_CERTIFICATES);
        let (home, args) = stand_in(&then(COUNT, &then(&said, &then(&readies(), WAIT))));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(20),
        };
        let started = Instant::now();
        let finished = commands.run(home.path()).unwrap();
        assert_eq!(finished.code, None);
        assert_eq!(runs(home.path()), 1);
        assert!(
            String::from_utf8_lossy(&finished.output).contains(conversation::NO_CERTIFICATES),
            "{:?}",
            String::from_utf8_lossy(&finished.output)
        );
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_run_typed_into_that_started_runs_on_past_the_start_up_limit() {
        let ends = if cfg!(windows) {
            "ping -n 4 127.0.0.1 > nul& exit 3"
        } else {
            "sleep 3; exit 3"
        };
        let (home, args) = stand_in(&then(&starts(), &then(&readies(), ends)));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_secs(1),
        };
        let (command, secret) = quit();
        let finished = typed_in_time(commands, home.path(), command, secret).unwrap();
        assert_eq!(finished.code, Some(3_i32));
    }

    #[test]
    fn a_run_typed_into_that_started_is_stopped_at_its_limit_counted_from_its_start() {
        let ends = if cfg!(windows) {
            "ping -n 7 127.0.0.1 > nul& exit 3"
        } else {
            "sleep 6; exit 3"
        };
        let (home, args) = stand_in(&then(&starts(), &then(&readies(), ends)));
        let commands = Commands {
            args,
            limit: Duration::from_secs(4),
            startup: Duration::from_secs(3),
        };
        let (command, secret) = quit();
        let finished = typed_in_time(commands, home.path(), command, secret).unwrap();
        assert_eq!(finished.code, None);
    }

    #[test]
    fn a_run_typed_into_is_stopped_at_the_limit_of_its_commands() {
        let (home, args) = waiting(None);
        let commands = Commands {
            args,
            limit: Duration::from_secs(1),
            startup: Duration::from_mins(1),
        };
        let (command, secret) = quit();
        let finished = typed_in_time(commands, home.path(), command, secret).unwrap();
        assert_eq!(finished.code, None);
    }

    #[test]
    fn a_run_typed_into_that_says_it_never_will_finish_is_stopped_well_inside_its_limit() {
        let (home, args) = waiting(Some(conversation::NO_CERTIFICATES));
        let commands = Commands {
            args,
            limit: Duration::from_mins(1),
            startup: Duration::from_mins(1),
        };
        let (command, secret) = quit();
        let finished = typed_in_time(commands, home.path(), command, secret).unwrap();
        assert_eq!(finished.code, None);
        assert!(
            String::from_utf8_lossy(&finished.output).contains(conversation::NO_CERTIFICATES),
            "{:?}",
            String::from_utf8_lossy(&finished.output)
        );
    }
}
