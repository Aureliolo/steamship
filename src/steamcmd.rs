//! How steamcmd is started: which file, with what around it, and with which commands.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::account::Account;
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

/// The longest an upload may run before it is taken to have hung and is stopped. A large first
/// upload over a slow line takes hours; steamcmd waiting at a prompt would wait forever.
pub const UPLOAD_LIMIT: Duration = Duration::from_hours(6);

/// The commands that log `account` in with the login steamcmd saved, build `script` and quit.
///
/// With no saved login, or an expired one, steamcmd fails rather than asking for a password
/// nobody is there to type.
#[must_use]
pub fn upload(account: &Account, script: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        account.name(),
        "+run_app_build",
    ]
    .map(OsString::from)
    .into();
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
        assert!(environment(home, Platform::Windows).is_empty());
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
            upload(&account, script),
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
