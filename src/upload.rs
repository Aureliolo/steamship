//! An upload: the app script rewritten with what steamship decides, and the result read back.
//!
//! steamcmd is given a copy of the app script in the home, never the original, with every path
//! in it absolute and three keys set by steamship: `BuildOutput` in the home, so that logs and
//! the chunk cache never land in the content; `Desc`, the version and commit; and `Preview` for
//! a preview. steamcmd reads `Preview` from its command line only when another option follows
//! it, as tried, so the file is the one place it is reliably set.

use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};
use std::process::Command;

use crate::conversation;
use crate::scripts;
use crate::vdf::{self, Block, Pair, Value};

/// The folder in the home that holds each app's copy of the script and build output.
pub const APPS: &str = "apps";

#[derive(Debug)]
pub enum Error {
    /// The description steamship would give the build cannot be used as it is.
    Description(String),
    /// The script cannot be read or rewritten.
    Script {
        path: PathBuf,
        reason: String,
    },
    Io {
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Description(reason) => formatter.write_str(reason),
            Self::Script { path, reason } => write!(formatter, "{}: {reason}", path.display()),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io { error, .. } => Some(error),
            Self::Description(_) | Self::Script { .. } => None,
        }
    }
}

fn at(path: &Path) -> impl FnOnce(io::Error) -> Error {
    let path = path.to_path_buf();
    move |error| Error::Io { path, error }
}

/// The build's description: the version and the commit its scripts are in.
///
/// # Errors
///
/// When the version is empty, too long, or holds a quote or a control character, none of which
/// a script value can carry.
pub fn description(version: &str, commit: &str) -> Result<String, Error> {
    let usable = !version.is_empty()
        && version.len() <= 64
        && !version
            .chars()
            .any(|character| character == '"' || character.is_control());
    if usable {
        Ok(format!("{version} {commit}"))
    } else {
        Err(Error::Description(format!(
            "the version \"{}\" is not 1 to 64 characters without quotes or control characters",
            version.escape_debug()
        )))
    }
}

/// The commit the file or folder at `path` is in, abbreviated as Git does for display.
///
/// # Errors
///
/// When Git cannot be run, or `path` is not in a Git repository with a commit.
pub fn commit(path: &Path) -> Result<String, Error> {
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    };
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&folder)
        .output()
        .map_err(at(&folder))?;
    if !output.status.success() {
        return Err(Error::Description(format!(
            "{} is not in a Git repository with a commit, which the build description names",
            folder.display()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub app_id: u32,
    /// The copy of the app script steamcmd is given.
    pub script: PathBuf,
    /// Where steamcmd writes its build logs and keeps its chunk cache.
    pub output: PathBuf,
    /// The branch the script sets the build live on, if any.
    pub set_live: Option<String>,
}

impl Prepared {
    /// The log steamcmd writes for the app, in [`Prepared::output`].
    #[must_use]
    pub fn log(&self) -> PathBuf {
        self.output.join(format!("app_build_{}.log", self.app_id))
    }
}

/// Writes the copy of the app script at `original` that steamcmd is given, into the app's folder
/// in `home`, and clears the last run's app log so that only this run's is read.
///
/// # Errors
///
/// When the script cannot be read, has no app ID, or cannot be written back; or the folders in
/// the home cannot be made.
pub fn prepare(
    home: &Path,
    original: &Path,
    description: &str,
    preview: bool,
) -> Result<Prepared, Error> {
    let problem = |reason: &str| Error::Script {
        path: original.to_path_buf(),
        reason: reason.to_owned(),
    };
    let text = fs::read_to_string(original).map_err(at(original))?;
    let document = vdf::parse(&text).map_err(|error| problem(&error.to_string()))?;
    let Some(app) = document.block("AppBuild") else {
        return Err(problem("there is no \"AppBuild\" block"));
    };
    let Some(app_id) = app.text("AppID").and_then(|id| id.parse::<u32>().ok()) else {
        return Err(problem("there is no numeric \"AppID\""));
    };
    // steamcmd reads a relative path in the copy against the copy's folder, so every path is
    // made absolute from where the original is.
    let absolute = path::absolute(original).map_err(at(original))?;
    let folder = absolute
        .parent()
        .map_or_else(|| absolute.clone(), Path::to_path_buf);
    let content_root = app
        .text("ContentRoot")
        .map_or_else(|| folder.clone(), |root| scripts::resolve(&folder, root));
    let apps = home.join(APPS).join(app_id.to_string());
    let output = apps.join("output");
    fs::create_dir_all(&output).map_err(at(&output))?;
    let mut rewritten = Block::default();
    for pair in &app.pairs {
        let decided = ["ContentRoot", "BuildOutput", "Desc", "Preview", "Local"]
            .iter()
            .any(|key| pair.key.eq_ignore_ascii_case(key));
        if decided {
            continue;
        }
        let value = if pair.key.eq_ignore_ascii_case("Depots") {
            depots(&pair.value, &folder).map_err(|reason| problem(&reason))?
        } else {
            pair.value.clone()
        };
        rewritten.pairs.push(Pair {
            key: pair.key.clone(),
            value,
        });
    }
    let mut set = |key: &str, value: String| {
        rewritten.pairs.push(Pair {
            key: key.to_owned(),
            value: Value::Text(value),
        });
    };
    set(
        "ContentRoot",
        written(&content_root).map_err(|reason| problem(&reason))?,
    );
    set(
        "BuildOutput",
        written(&output).map_err(|reason| problem(&reason))?,
    );
    set("Desc", description.to_owned());
    if preview {
        set("Preview", "1".to_owned());
    }
    let whole = Block {
        pairs: vec![Pair {
            key: "AppBuild".to_owned(),
            value: Value::Block(rewritten),
        }],
    };
    let script = apps.join("app_build.vdf");
    let rewritten_text = vdf::write(&whole).map_err(|error| problem(&error.to_string()))?;
    fs::write(&script, rewritten_text).map_err(at(&script))?;
    let prepared = Prepared {
        app_id,
        script,
        output,
        set_live: app.text("SetLive").map(str::to_owned),
    };
    match fs::remove_file(prepared.log()) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(Error::Io {
            path: prepared.log(),
            error,
        }),
        _ => Ok(prepared),
    }
}

/// The `Depots` block with each depot script named by its absolute path. A depot written inline
/// is kept as it is: its own paths resolve against the app's content root, which stays the same.
fn depots(listed: &Value, folder: &Path) -> Result<Value, String> {
    let Value::Block(block) = listed else {
        return Err("\"Depots\" is not a block".to_owned());
    };
    let mut rewritten = Block::default();
    for pair in &block.pairs {
        let value = match &pair.value {
            Value::Text(name) => Value::Text(written(&scripts::resolve(folder, name))?),
            Value::Block(inline) => Value::Block(inline.clone()),
        };
        rewritten.pairs.push(Pair {
            key: pair.key.clone(),
            value,
        });
    }
    Ok(Value::Block(rewritten))
}

/// `path` as a script value. Forward slashes work for steamcmd on every system, and avoid a
/// backslash before the closing quote, which some `KeyValues` readers take as an escape.
///
/// # Errors
///
/// When `path` is not valid Unicode, which a script cannot carry.
pub fn written(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|text| text.replace('\\', "/"))
        .ok_or_else(|| format!("{} is not valid Unicode", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Uploaded, as the build with this ID.
    Built { build_id: u64 },
    /// A preview that finished; nothing was uploaded.
    Previewed,
    /// steamcmd could not log in with what it has saved; the line it said so on.
    NotLoggedIn(String),
    /// Valve built the upload as this build, and steamcmd failed after, as when setting it live;
    /// with every reason it gave. The build is there to be set live by hand.
    BuiltThenFailed { build_id: u64, reasons: Vec<String> },
    /// Anything else, with every reason steamcmd gave.
    Failed(Vec<String>),
}

/// Reads the result of a run for `app_id` from steamcmd's exit code, console and build log.
///
/// Success is taken only from the log, since console output captured from steamcmd is known to
/// lose lines, and only with an exit code of 0.
#[must_use]
pub fn judge(
    app_id: u32,
    code: Option<i32>,
    console: &str,
    log: Option<&str>,
    preview: bool,
) -> Outcome {
    if let Some(reason) = refused_login(console) {
        return Outcome::NotLoggedIn(reason);
    }
    let finished = format!("Successfully finished AppID {app_id} build (BuildID ");
    let build_id = log.and_then(|log| {
        log.lines().find_map(|line| {
            let (_, after) = line.split_once(&finished)?;
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
    });
    match (code, build_id) {
        (Some(0_i32), Some(_)) if preview => Outcome::Previewed,
        (Some(0_i32), Some(build_id)) if build_id > 0 => Outcome::Built { build_id },
        (_, Some(build_id)) if build_id > 0 && !preview => Outcome::BuiltThenFailed {
            build_id,
            reasons: reasons(code, console, log),
        },
        _ => Outcome::Failed(reasons(code, console, log)),
    }
}

/// Why steamcmd could not log in with the login it saved, when that is what its `console` says.
/// The reason is taken from the line and not the line itself, which names the account.
#[must_use]
pub fn refused_login(console: &str) -> Option<String> {
    console.lines().find_map(|line| {
        if line.contains("Cached credentials not found") {
            Some(line.trim().to_owned())
        } else if line.contains("FAILED (No cached credentials")
            || (line.contains("Logging in") && line.contains("FAILED"))
        {
            Some(conversation::refusal(line).map_or_else(
                || "Steam refused the saved login".to_owned(),
                ToOwned::to_owned,
            ))
        } else {
            None
        }
    })
}

/// What checking the saved login with Steam came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    /// Steam took it: an upload now would be logged in.
    Taken,
    /// Steam refused it, for this reason.
    Refused(String),
    /// The check itself failed, for every reason steamcmd gave.
    Failed(Vec<String>),
}

/// Reads steamcmd's exit `code` and `console` from a run that only logged in and quit.
#[must_use]
pub fn judge_login(code: Option<i32>, console: &str) -> Login {
    refused_login(console).map_or_else(
        || {
            if code == Some(0_i32) {
                Login::Taken
            } else {
                Login::Failed(reasons(code, console, None))
            }
        },
        Login::Refused,
    )
}

/// Every error steamcmd reported, from the log and the console, once each, and what its exit
/// said when nothing else did.
#[must_use]
pub fn reasons(code: Option<i32>, console: &str, log: Option<&str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in log.into_iter().flat_map(str::lines).chain(console.lines()) {
        if let Some((_, error)) = line.split_once("ERROR!") {
            let error = error.trim().to_owned();
            if !found.contains(&error) {
                found.push(error);
            }
        }
    }
    if found.is_empty() {
        found.push(match code {
            Some(0_i32) => {
                "steamcmd exited 0, but its build log does not say the build finished".to_owned()
            }
            Some(code) => format!("steamcmd exited {code} without saying why"),
            None => "steamcmd did not finish in time and was stopped".to_owned(),
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_the_build_as_its_version_and_commit() {
        assert_eq!(
            description("1.4.0", "abc123def456").unwrap(),
            "1.4.0 abc123def456"
        );
    }

    #[test]
    fn refuses_a_version_a_script_value_cannot_carry() {
        for version in ["", "1.0\"", "1.0\n", &"9".repeat(65)] {
            assert!(
                matches!(description(version, "abc"), Err(Error::Description(_))),
                "{version:?}"
            );
        }
        assert_eq!(
            description(&"9".repeat(64), "abc").unwrap(),
            format!("{} abc", "9".repeat(64))
        );
    }

    const LOG: &str = "[2026-09-26 17:27:28]: Starting AppID 5335970 build (flags 0x0).\n\
                       [2026-09-26 17:29:02]: Successfully finished AppID 5335970 build (BuildID 18273645).\n";

    /// What steamcmd prints on logging in with the token it keeps.
    const LOGGED_IN: &str = "Logging in user 'build_bot' [U:1:0] to Steam Public...OK\r\n\
                             Waiting for client config...OK\r\n";

    #[test]
    fn takes_the_build_id_from_the_log_when_steamcmd_exits_0() {
        assert_eq!(
            judge(5_335_970, Some(0_i32), LOGGED_IN, Some(LOG), false),
            Outcome::Built {
                build_id: 18_273_645
            }
        );
        assert_eq!(
            judge(5_335_970, Some(0_i32), LOGGED_IN, Some(LOG), true),
            Outcome::Previewed
        );
    }

    #[test]
    fn a_failure_after_logging_in_is_not_a_failure_to_log_in() {
        let console = format!("{LOGGED_IN}ERROR! FAILED to upload a chunk\r\n");
        assert!(matches!(
            judge(5_335_970, Some(6_i32), &console, None, false),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn a_finished_line_for_another_app_or_a_nonzero_exit_is_no_success() {
        assert!(matches!(
            judge(1, Some(0_i32), "", Some(LOG), false),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            judge(
                5_335_970,
                Some(0_i32),
                "",
                Some(&LOG.replace("18273645", "0")),
                false
            ),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn a_build_valve_finished_is_kept_when_steamcmd_fails_after() {
        let console = format!("{LOGGED_IN}ERROR! Failed to set build live on branch testing\r\n");
        for code in [Some(6_i32), None] {
            assert_eq!(
                judge(5_335_970, code, &console, Some(LOG), false),
                Outcome::BuiltThenFailed {
                    build_id: 18_273_645,
                    reasons: vec!["Failed to set build live on branch testing".to_owned()],
                },
                "{code:?}"
            );
        }
        assert!(
            matches!(
                judge(5_335_970, Some(6_i32), &console, Some(LOG), true),
                Outcome::Failed(_)
            ),
            "a preview builds nothing to keep"
        );
        assert!(matches!(
            judge(
                5_335_970,
                Some(6_i32),
                &console,
                Some(&LOG.replace("18273645", "0")),
                false
            ),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn a_missing_or_refused_login_is_named_by_its_reason_and_never_the_account() {
        let console = "Loading Steam API...OK\r\nCached credentials not found.\r\n\
                       FAILED (No cached credentials and @NoPromptForPassword is set)\r\n";
        assert_eq!(
            judge(1, Some(5_i32), console, None, false),
            Outcome::NotLoggedIn("Cached credentials not found.".to_owned())
        );
        assert_eq!(
            refused_login("FAILED (No cached credentials and @NoPromptForPassword is set)"),
            Some("No cached credentials and @NoPromptForPassword is set".to_owned())
        );
        let expired =
            "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)";
        assert_eq!(
            judge(1, Some(5_i32), expired, None, false),
            Outcome::NotLoggedIn("Expired Login Auth Code".to_owned())
        );
        assert_eq!(
            refused_login("Logging in user 'build_bot' to Steam Public...FAILED"),
            Some("Steam refused the saved login".to_owned())
        );
        assert_eq!(refused_login(LOGGED_IN), None);
    }

    #[test]
    fn a_login_check_is_taken_refused_or_failed() {
        assert_eq!(judge_login(Some(0_i32), LOGGED_IN), Login::Taken);
        assert_eq!(
            judge_login(
                Some(5_i32),
                "Logging in user 'build_bot' to Steam Public...FAILED (Expired Login Auth Code)"
            ),
            Login::Refused("Expired Login Auth Code".to_owned())
        );
        assert_eq!(
            judge_login(Some(0_i32), "Cached credentials not found.\r\n"),
            Login::Refused("Cached credentials not found.".to_owned())
        );
        assert!(matches!(
            judge_login(Some(6_i32), "ERROR! Timed out waiting for Steam\r\n"),
            Login::Failed(reasons) if reasons == ["Timed out waiting for Steam"]
        ));
        assert!(matches!(judge_login(None, ""), Login::Failed(reasons) if !reasons.is_empty()));
    }

    #[test]
    fn a_failure_lists_every_error_once_from_the_log_and_the_console() {
        let log = "[..]: ERROR! Failed to initialize build on server (Access Denied)\n\
                   [..]: ERROR! Build for depot 5335971 failed : Failure\n";
        let console = "[..]: ERROR! Build for depot 5335971 failed : Failure\n";
        assert_eq!(
            judge(5_335_970, Some(6_i32), console, Some(log), false),
            Outcome::Failed(vec![
                "Failed to initialize build on server (Access Denied)".to_owned(),
                "Build for depot 5335971 failed : Failure".to_owned(),
            ])
        );
    }

    #[test]
    fn says_what_went_wrong_and_keeps_the_cause() {
        let io = Error::Io {
            path: PathBuf::from("home"),
            error: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(io.to_string(), "home: denied");
        assert_eq!(
            error::Error::source(&io).map(ToString::to_string),
            Some("denied".to_owned())
        );
        let script = Error::Script {
            path: PathBuf::from("app.vdf"),
            reason: "no".to_owned(),
        };
        assert_eq!(script.to_string(), "app.vdf: no");
        assert!(error::Error::source(&script).is_none());
        let description = Error::Description("bad".to_owned());
        assert_eq!(description.to_string(), "bad");
        assert!(error::Error::source(&description).is_none());
    }

    #[test]
    fn a_failure_that_says_nothing_is_described_by_how_steamcmd_ended() {
        let failed = |code| judge(1, code, "", None, false);
        let reason = |text: &str| Outcome::Failed(vec![text.to_owned()]);
        assert_eq!(
            failed(Some(3_i32)),
            reason("steamcmd exited 3 without saying why")
        );
        assert_eq!(
            failed(None),
            reason("steamcmd did not finish in time and was stopped")
        );
        assert_eq!(
            failed(Some(0_i32)),
            reason("steamcmd exited 0, but its build log does not say the build finished")
        );
    }
}
