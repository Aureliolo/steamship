//! What every command gets ready before it runs steamcmd.

#[cfg(unix)]
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use steamship::install;
use steamship::platform::Platform;
#[cfg(unix)]
use steamship::run;
use steamship::steamcmd;

#[cfg(unix)]
use crate::steamship_command;
use crate::{
    SECRETS, assert_no_secret_in, failure, faked, faked_with, fetching, project, saved_login,
    steamship, upload, workshop_item, written,
};

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
        "STEAMSHIP_LOGIN does not hold a login packed by steamship ci. Pack the login again \
         with steamship ci"
    );
}

/// The commands that start steamcmd, each with what it needs around it to get that far.
#[derive(Clone, Copy, Debug)]
enum Starter {
    Status,
    Upload,
    Fetch,
    /// A fetch that types the password of its branch at steamcmd's console.
    FetchTyped,
    Wrap,
    Settings,
    Workshop,
}

impl Starter {
    /// Whether its step says how long steamcmd took, which the others have no clock for.
    const fn timed(self) -> bool {
        !matches!(self, Self::Status | Self::Settings)
    }

    /// The command, run from `home` with `variables` set.
    fn run(self, home: &Path, variables: &[(&str, &str)]) -> (Option<i32>, String, String) {
        let mut all = vec![("STEAMSHIP_NO_UPDATE_CHECK", "1")];
        all.extend_from_slice(variables);
        match self {
            Self::Status => steamship(&["status"], Some(home), &all),
            Self::Upload => {
                let (_project, script) = project(true);
                upload(&script, home, &["--version", "1.4.0"], &all)
            }
            Self::Fetch => fetching(home, &["1000"], &all),
            Self::FetchTyped => {
                all.push(("STEAMSHIP_BRANCH_PASSWORD", "open-sesame"));
                fetching(home, &["1000", "--branch", "friends"], &all)
            }
            Self::Wrap => {
                let files = tempfile::tempdir().unwrap();
                let game = files.path().join("game.exe");
                fs::write(&game, "MZplain").unwrap();
                steamship(
                    &["drm-wrap", "1000", &game.to_string_lossy()],
                    Some(home),
                    &all,
                )
            }
            Self::Settings => steamship(&["settings", "1000"], Some(home), &all),
            Self::Workshop => {
                let (_item, script) = workshop_item("");
                steamship(&["workshop", &script.to_string_lossy()], Some(home), &all)
            }
        }
    }
}

/// A home with a saved login whose recorded steamcmd is `script`.
fn stubbed(script: &str) -> tempfile::TempDir {
    let home = faked_with(script);
    saved_login(home.path());
    home
}

/// `starter` in a home whose steamcmd matches the pinned hash but cannot be started: on Unix its
/// interpreter is missing, and on Windows the shell that runs the fake is.
fn assert_cannot_start(starter: Starter) {
    let home = stubbed("#!/no/such/interpreter\n");
    let shell = home.path().join("no-such-shell");
    let (code, stdout, stderr) = starter.run(
        home.path(),
        &[("STEAMSHIP_FAKE_STEAMCMD", &shell.to_string_lossy())],
    );
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let program = steamcmd::program(&home.path().join(install::FOLDER), Platform::THIS);
    let reason = io::Error::from_raw_os_error(2_i32);
    assert_eq!(
        failure(&stderr),
        format!("{}: {reason}", program.display()),
        "{stderr}"
    );
    let step = "  steam     \u{2717} could not start";
    assert!(stdout.contains(step), "{stdout}");
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(step))
        .unwrap();
    if starter.timed() {
        let seconds = line
            .strip_prefix(" after ")
            .and_then(|rest| rest.strip_suffix(" s"));
        assert!(
            seconds.is_some_and(|seconds| seconds.parse::<u64>().is_ok()),
            "{line:?}"
        );
    } else {
        assert_eq!(line, "", "this step has no clock: {stdout}");
    }
}

/// A steamcmd that logs in anew during the run: it saves a login holding the first of
/// [`SECRETS`], then fails with that file printed as an error.
///
/// steamcmd is not known to print a saved login. The `ERROR! ` prefix is what makes steamship
/// show a line at these commands, so the test stands for the worst case.
fn renewing() -> String {
    let [token, _] = SECRETS;
    format!(
        "#!/bin/sh\n\
         mkdir -p \"$HOME/Steam/config\"\n\
         printf '\"InstallConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \
         \"ConnectCache\" {{ \"3a1f0c2e\" \"{token}\" }} }} }} }} }}\\n' \
         > \"$HOME/Steam/config/config.vdf\"\n\
         sed 's/^/ERROR! /' \"$HOME/Steam/config/config.vdf\"\n\
         exit 8\n"
    )
}

/// `starter` against [`renewing`], whose new login was in no file before the run.
fn assert_renewed_login_kept_out(starter: Starter) {
    let home = stubbed(&renewing());
    let (code, stdout, stderr) = starter.run(home.path(), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_no_secret_in("stdout", stdout.as_bytes());
    assert_no_secret_in("stderr", stderr.as_bytes());
    assert!(
        stderr.contains("[redacted]"),
        "the error is shown with the login blanked: {stderr}"
    );
    let steamcmd = format!("{}/steamcmd.sh", install::FOLDER);
    let saved = format!("{}/config/config.vdf", install::FOLDER);
    let except = [steamcmd.as_str(), saved.as_str(), "Steam/config/config.vdf"];
    for (path, contents) in written(home.path(), &except) {
        assert_no_secret_in(&path, &contents);
    }
}

#[test]
fn status_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::Status);
}

#[test]
fn an_upload_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::Upload);
}

#[test]
fn a_fetch_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::Fetch);
}

#[test]
fn a_typed_fetch_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::FetchTyped);
}

#[test]
fn status_whose_steamcmd_will_never_reach_steam_is_stopped_at_once() {
    let home = stubbed(
        "#!/bin/sh\n\
         echo 'unable to load trusted SSL root certificates'\n\
         sleep 30\n",
    );
    let started = Instant::now();
    let (code, stdout, stderr) = Starter::Status.run(home.path(), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "steamcmd found no certificates to check Steam's servers against, and would have waited \
         forever. Install the system's certificate store (ca-certificates)",
        "{stderr}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "not waited for: {:?}",
        started.elapsed()
    );
}

#[test]
fn an_upload_starts_steamcmd_in_steamcmds_own_folder() {
    let home = faked();
    saved_login(home.path());
    let (code, stdout, stderr) = Starter::Upload.run(home.path(), &[]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    let console = fs::read_to_string(home.path().join("apps/1000/output/steamcmd.log")).unwrap();
    // macOS's temporary folder is reached through a link, which steamcmd's folder is not.
    #[cfg(unix)]
    let root = fs::canonicalize(home.path().join(install::FOLDER)).unwrap();
    #[cfg(windows)]
    let root = home.path().join(install::FOLDER);
    assert!(
        console.contains(&format!("folder: {}\n", root.display())),
        "{console}"
    );
}

#[test]
fn a_wrap_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::Wrap);
}

#[test]
fn settings_that_cannot_start_steamcmd_name_the_program_and_why_and_exit_1() {
    assert_cannot_start(Starter::Settings);
}

#[test]
fn a_workshop_upload_that_cannot_start_steamcmd_names_the_program_and_why_and_exits_1() {
    assert_cannot_start(Starter::Workshop);
}

#[test]
fn status_keeps_a_login_steamcmd_saves_during_the_run_out_of_what_it_prints() {
    assert_renewed_login_kept_out(Starter::Status);
}

#[test]
fn an_upload_keeps_a_login_steamcmd_saves_during_the_run_out_of_what_it_prints_and_saves() {
    assert_renewed_login_kept_out(Starter::Upload);
}

#[test]
fn a_fetch_keeps_a_login_steamcmd_saves_during_the_run_out_of_what_it_prints() {
    assert_renewed_login_kept_out(Starter::Fetch);
}

#[test]
fn a_wrap_keeps_a_login_steamcmd_saves_during_the_run_out_of_what_it_prints() {
    assert_renewed_login_kept_out(Starter::Wrap);
}

#[test]
fn settings_keep_a_login_steamcmd_saves_during_the_run_out_of_what_they_print() {
    assert_renewed_login_kept_out(Starter::Settings);
}

#[test]
fn a_workshop_upload_keeps_a_login_steamcmd_saves_during_the_run_out_of_what_it_prints_and_saves() {
    assert_renewed_login_kept_out(Starter::Workshop);
}
