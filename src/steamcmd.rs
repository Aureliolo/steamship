//! How steamcmd is started: which file, with what around it, and with which commands.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::account::Account;
use crate::drm;
use crate::install;
use crate::platform::Platform;

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
pub fn check_login(account: &Account) -> Vec<OsString> {
    [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        account.name(),
        "+quit",
    ]
    .map(OsString::from)
    .into()
}

/// The commands that log `account` in with the login steamcmd saved, print `app`'s settings as
/// Steam shows them to that account, and quit.
///
/// The settings are asked for afresh rather than read from steamcmd's cache. Like [`upload`], it
/// never waits at a prompt.
#[must_use]
pub fn app_info(account: &Account, app: u32) -> Vec<OsString> {
    let app = app.to_string();
    [
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
    .into()
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
) -> Vec<OsString> {
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
    args
}

/// The commands that log `account` in with the login steamcmd saved, upload the Workshop item
/// `script` describes and quit. Like [`upload`], it never waits at a prompt.
#[must_use]
pub fn workshop(account: &Account, script: &Path) -> Vec<OsString> {
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
    args
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
) -> Vec<OsString> {
    let mut args = fetch_console(account, system, folder);
    args.push("+app_update".into());
    args.push(app.to_string().into());
    args.extend(["-beta", branch].map(OsString::from));
    if let Some(password) = password {
        args.extend(["-betapassword", password].map(OsString::from));
    }
    args.extend(["validate", "+quit"].map(OsString::from));
    args
}

/// The commands that log `account` in as [`fetch`] does and leave steamcmd at its console, for
/// the fetch to be typed there.
///
/// The `system` and the `folder` are set before logging in, which Valve's documentation asks of
/// the folder.
#[must_use]
pub fn fetch_console(account: &Account, system: &str, folder: &Path) -> Vec<OsString> {
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
    args
}

/// The longest a fetch may run: a large build over a slow line takes hours.
pub const FETCH_LIMIT: Duration = Duration::from_hours(6);

/// The longest a check of the saved login may run: logging in takes seconds.
pub const CHECK_LIMIT: Duration = Duration::from_mins(2);

/// The longest a DRM wrap may run: the executable goes to Valve's servers and back, which for a
/// large one over a slow line takes many minutes.
pub const WRAP_LIMIT: Duration = Duration::from_hours(1);

/// The longest an upload may run before it is taken to have hung and is stopped. A large first
/// upload over a slow line takes hours; steamcmd waiting at a prompt would wait forever.
pub const UPLOAD_LIMIT: Duration = Duration::from_hours(6);

/// What steamcmd says when it will never reach Steam, only wait.
///
/// It is stopped then rather than at its limit. With no certificates to check Steam's servers
/// against (on Linux, with the system's certificate store missing), it says this at once and then
/// waits to connect for good.
pub const HOPELESS: &[&str] = &[NO_CERTIFICATES];

/// See [`HOPELESS`].
pub const NO_CERTIFICATES: &str = "unable to load trusted SSL root certificates";

/// The commands that log `account` in with the login steamcmd saved, print the info of the app
/// `info_of` names, as [`app_info`] does, build `script` and quit.
///
/// The info is asked in the same run because each login to Steam takes seconds. With no saved
/// login, or an expired one, steamcmd fails rather than asking for a password nobody is there to
/// type.
#[must_use]
pub fn upload(account: &Account, script: &Path, info_of: Option<u32>) -> Vec<OsString> {
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
    args
}

#[cfg(test)]
mod tests {
    use super::*;

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
            upload(&account, script, None),
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
            upload(&account, script, Some(1)),
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
                ),
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
            workshop(&account, script),
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
            fetch(&account, 480, "testing", None, "linux", folder),
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
        let default = fetch(&account, 480, "public", None, "windows", folder);
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
        );
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
        let console = fetch_console(&account, "linux", folder);
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
        assert!(fetch(&account, 480, "friends", None, "linux", folder).starts_with(&console));
    }

    #[test]
    fn checks_the_saved_login_and_never_waits_at_a_prompt() {
        let account = Account::parse("build_bot").unwrap();
        assert_eq!(
            check_login(&account),
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
}
