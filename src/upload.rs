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
use std::time::Duration;

use crate::conversation;
use crate::run;
use crate::scripts;
use crate::steamcmd;
use crate::vdf::{self, Block, Pair, Value};
use crate::webapi;

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
/// When `path`'s folder is not there, Git cannot be run, or `path` is not in a Git repository
/// with a commit.
pub fn commit(path: &Path) -> Result<String, Error> {
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    };
    // Looked at first: a folder that is not there and a Git that is not installed fail to start
    // with the same error, and only one of them is about the folder.
    let _: fs::Metadata = fs::metadata(&folder).map_err(at(&folder))?;
    let output = run::command("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&folder)
        .output()
        .map_err(|error| {
            Error::Description(if error.kind() == io::ErrorKind::NotFound {
                "Git is not installed, or not on the PATH, and the build description names the \
                 commit the scripts are at"
                    .to_owned()
            } else {
                format!("Git could not be run ({error}), and the build description names the commit the scripts are at")
            })
        })?;
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
    clear_log(&prepared)?;
    Ok(prepared)
}

/// Clears the last run's app log, so that only the next run's is read.
///
/// # Errors
///
/// When the log is there and cannot be removed.
pub fn clear_log(prepared: &Prepared) -> Result<(), Error> {
    match fs::remove_file(prepared.log()) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(Error::Io {
            path: prepared.log(),
            error,
        }),
        _ => Ok(()),
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
    let finished = log.and_then(|log| log.lines().find_map(|line| finished(app_id, line)));
    match (code, finished) {
        (Some(0_i32), Some(Finished::Preview)) if preview => Outcome::Previewed,
        (Some(0_i32), Some(Finished::Build(build_id))) if build_id > 0 && !preview => {
            Outcome::Built { build_id }
        }
        (_, Some(Finished::Build(build_id))) if build_id > 0 && !preview => {
            Outcome::BuiltThenFailed {
                build_id,
                reasons: reasons(code, console, log),
            }
        }
        _ => Outcome::Failed(reasons(code, console, log)),
    }
}

/// How the build log says Valve finished a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finished {
    /// `Successfully finished AppID 1000010 build preview.`, as steamcmd 1788292693 ends a
    /// preview.
    Preview,
    /// `Successfully finished AppID 1000010 build (BuildID 18273645).`, with the build's ID.
    Build(u64),
}

/// What `line` says of a finished build for `app_id`, if it is that line.
fn finished(app_id: u32, line: &str) -> Option<Finished> {
    let (_, after) = line.split_once(&format!("Successfully finished AppID {app_id} build "))?;
    if after.starts_with("preview.") {
        return Some(Finished::Preview);
    }
    let digits: String = after
        .strip_prefix("(BuildID ")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().map(Finished::Build)
}

/// Why Steam may have refused to set a build live on the branch the script names, if it did.
///
/// Valve names no reason, and keeps the build: steamcmd 1788292693 said only "Failed to commit
/// build" for a branch the app did not have, and each such build was listed on Steam.
#[must_use]
pub fn commit_refused(reasons: &[String], set_live: Option<&str>) -> Option<String> {
    let branch = set_live?;
    reasons
        .iter()
        .any(|reason| reason.starts_with("Failed to commit build"))
        .then(|| {
            format!(
                "\"SetLive\" names \"{branch}\": Steam sets no build live on a branch the app \
                 does not have, or for an account that may not set builds live; a new app gets \
                 other branches only once its default branch has a build live"
            )
        })
}

/// Why a `SetLive` `branch` is refused when `app`'s `branches` do not include it, and what to do:
/// Steamworks offers no other branch until the default one has a build live.
#[must_use]
pub fn no_branch(app: u32, branch: &str, branches: &[webapi::Branch]) -> String {
    format!(
        "\"SetLive\" names \"{branch}\", but app {app} has no branch by that name; {}",
        create_first(branches)
    )
}

/// How to create a branch that an app with these `branches` lacks.
#[must_use]
pub fn create_first(branches: &[webapi::Branch]) -> &'static str {
    let fresh = branches
        .iter()
        .any(|found| webapi::is_default(&found.name) && found.build_id == 0);
    if fresh {
        "set a build live on its default branch, then create it, both in Steamworks under \
         SteamPipe, Builds"
    } else {
        "create it in Steamworks under SteamPipe, Builds first"
    }
}

/// What Steam shows live on a branch an upload set live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Live {
    /// The build just uploaded.
    Confirmed,
    /// Another build.
    Other(u64),
    /// The app has no branch by that name.
    Missing,
}

/// What `branches` show live on `branch`, against the `build_id` just uploaded.
#[must_use]
pub fn live_now(branches: &[webapi::Branch], branch: &str, build_id: u64) -> Live {
    match branches
        .iter()
        .find(|found| found.name.eq_ignore_ascii_case(branch))
    {
        Some(found) if found.build_id == build_id => Live::Confirmed,
        Some(found) => Live::Other(found.build_id),
        None => Live::Missing,
    }
}

/// Where a build Steam kept but set live nowhere is, when its ID is not to be had.
pub const NOT_LIVE: &str =
    "the build is on Steam but not live: it is listed in Steamworks under SteamPipe, Builds";

/// The ID of the newest of `builds`, given newest first, with this upload's `description`.
#[must_use]
pub fn uploaded_as(builds: &[webapi::Build], description: &str) -> Option<u64> {
    builds
        .iter()
        .find(|build| build.description == description)
        .map(|build| build.build_id)
}

/// How long an upload waits in all for Steam's CDN to serve a manifest steamcmd builds against.
///
/// The CDN was seen to answer 404 for a branch's last manifest 16 minutes after its build, and to
/// serve it a minute later.
pub const CDN_PATIENCE: Duration = Duration::from_mins(30);

/// The most times steamcmd runs for one upload. Each run logs in again, so a request code that
/// keeps expiring does not have steamcmd log in without end.
pub const RUNS: u32 = 4;

/// What steamcmd says when Steam's build server is not reached as a build begins, after a login
/// that was. Seen once in CI, on a run between two that built minutes before and after.
const NO_CONNECTION_AT_BUILD: &str = "ERROR! Failed to initialize build on server (No Connection)";

/// What steamcmd says when Steam drops it after it connected. Seen once in CI, as steamcmd
/// 1791918946 waited for the user info of its anonymous login, on a run between others that
/// logged in.
const DISCONNECTED: &str =
    "steamcmd has been disconnected from steam with result 3 (No Connection)";

/// Why a build failed that may pass when steamcmd runs again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retry {
    /// For a manifest the CDN does not serve yet; it passes once the CDN does.
    Unserved(Unserved),
    /// For no connection to Steam's build server; it is tried once more.
    NoConnection,
    /// For Steam dropping steamcmd's connection; it is tried once more, as for no connection.
    Disconnected,
}

/// Why steamcmd's build failed, from `console` and `log`, if running it again may pass.
#[must_use]
pub fn retry(console: &str, log: Option<&str>) -> Option<Retry> {
    let says = |line: &str| {
        log.into_iter()
            .chain([console])
            .any(|text| text.contains(line))
    };
    unserved(console, log).map(Retry::Unserved).or_else(|| {
        if says(NO_CONNECTION_AT_BUILD) {
            Some(Retry::NoConnection)
        } else if says(DISCONNECTED) {
            Some(Retry::Disconnected)
        } else {
            None
        }
    })
}

/// A depot's manifest that steamcmd could not download, because Steam's CDN does not serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unserved {
    pub depot: u32,
    /// Where to ask for it, request code included.
    pub url: String,
}

/// The manifest steamcmd's build failed for want of, if that is why, from `console` and `log`.
///
/// steamcmd builds each depot against the manifest of the build last on the branch, and Steam's
/// CDN serves that only some minutes after the build: until then it answers 404.
fn unserved(console: &str, log: Option<&str>) -> Option<Unserved> {
    log.into_iter()
        .flat_map(str::lines)
        .chain(console.lines())
        .find_map(|line| {
            let (_, quoted) = line.split_once("ERROR! Failed to download manifest \"")?;
            let (printed, after) = quoted.split_once('"')?;
            if after.trim() == "(HTTP 404)" {
                manifest_at(printed)
            } else {
                None
            }
        })
}

/// The manifest at `printed`, which steamcmd gives as a host and a path with no scheme. Asked
/// over HTTPS; a debug build, as the tests run, asks a stand-in on this machine over HTTP.
fn manifest_at(printed: &str) -> Option<Unserved> {
    let mut parts = printed.split('/');
    let host = parts.next()?;
    let path: Vec<&str> = parts.collect();
    let ["depot", depot, "manifest", numbers @ ..] = path.as_slice() else {
        return None;
    };
    let numeric = |part: &&str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if numbers.len() != 3 || !numbers.iter().all(numeric) {
        return None;
    }
    let loopback = host
        .strip_prefix("127.0.0.1:")
        .is_some_and(|port| numeric(&port));
    let named = host.contains('.')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
    let scheme = if cfg!(debug_assertions) && loopback {
        "http"
    } else if named {
        "https"
    } else {
        return None;
    };
    Some(Unserved {
        depot: depot.parse().ok()?,
        url: format!("{scheme}://{printed}"),
    })
}

/// Adds to a failed `outcome` that the CDN never served the manifest of `depot`, and what to do.
pub fn still_unserved(outcome: &mut Outcome, depot: u32) {
    if let Outcome::Failed(reasons) = outcome {
        reasons.push(format!(
            "Steam's CDN still does not serve depot {depot}'s last manifest, which steamcmd \
             builds the upload against; upload again later"
        ));
    }
}

/// What steamcmd says in place of a login when it never reached Steam, which refused nothing.
const NO_CONNECTION: &str = "No Connection";

/// Why steamcmd could not log in with the login it saved, when that is what its `console` says.
/// The reason is taken from the line and not the line itself, which names the account.
#[must_use]
pub fn refused_login(console: &str) -> Option<String> {
    console.lines().find_map(|line| {
        if line.contains("Cached credentials not found") {
            Some(line.trim().to_owned())
        } else if conversation::refusal(line) == Some(NO_CONNECTION) {
            None
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
        let error = if let Some((_, error)) = line.split_once("ERROR!") {
            error.trim().to_owned()
        } else if line.contains(steamcmd::NO_CERTIFICATES) {
            "steamcmd found no certificates to check Steam's servers against, and would have \
             waited for good; install the system's certificate store (ca-certificates)"
                .to_owned()
        } else if let Some((_, socket)) = line.split_once("CreateBoundSocket: ") {
            no_socket(socket.trim())
        } else if conversation::refusal(line) == Some(NO_CONNECTION) {
            format!("steamcmd could not connect to Steam ({NO_CONNECTION})")
        } else {
            continue;
        };
        if !found.contains(&error) {
            found.push(error);
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

/// Why steamcmd could not open a network socket, from what it `said`. Error 38 is ENOSYS, which
/// Docker's default seccomp filter answers the socket call 32-bit steamcmd makes with: seen in a
/// container, and gone when the same container runs without the filter.
fn no_socket(said: &str) -> String {
    if said.ends_with("(38)") {
        "steamcmd could not open a network socket: in a Docker container, the default seccomp \
         filter refuses the socket call 32-bit steamcmd makes; run the container with \
         --security-opt seccomp=unconfined"
            .to_owned()
    } else {
        format!("steamcmd could not open a network socket: {said}")
    }
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
    fn a_folder_that_is_not_there_is_named_rather_than_git() {
        let missing = tempfile::tempdir().unwrap().path().join("steam");
        let failed = commit(&missing.join("app_build.vdf")).unwrap_err();
        assert!(
            matches!(&failed, Error::Io { path, error } if *path == missing && error.kind() == io::ErrorKind::NotFound),
            "{failed:?}"
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

    const LOG: &str = "[2026-09-26 17:27:28]: Starting AppID 1000030 build (flags 0x0).\n\
                       [2026-09-26 17:29:02]: Successfully finished AppID 1000030 build (BuildID 18273645).\n";

    /// What steamcmd prints on logging in with the token it keeps.
    const LOGGED_IN: &str = "Logging in user 'build_bot' [U:1:0] to Steam Public...OK\r\n\
                             Waiting for client config...OK\r\n";

    /// A preview's build log as steamcmd 1788292693 wrote it for Falcon Ridge Overseer.
    const PREVIEW_LOG: &str = "[2026-09-28 20:24:13]: Starting AppID 1000010 build (flags 0x2).\n\
         [2026-09-28 20:24:13]: Building depot 1000011...\n\
         \n\
         [2026-09-28 20:24:21]: Building depot 1000012...\n\
         \n\
         [2026-09-28 20:24:30]: Successfully finished AppID 1000010 build preview.\n";

    #[test]
    fn takes_the_build_id_from_the_log_when_steamcmd_exits_0() {
        assert_eq!(
            judge(1_000_030, Some(0_i32), LOGGED_IN, Some(LOG), false),
            Outcome::Built {
                build_id: 18_273_645
            }
        );
    }

    #[test]
    fn a_preview_is_finished_only_by_the_line_valve_ends_one_with() {
        assert_eq!(
            judge(1_000_010, Some(0_i32), LOGGED_IN, Some(PREVIEW_LOG), true),
            Outcome::Previewed
        );
        for (app, code, log, preview) in [
            (1_000_011, Some(0_i32), PREVIEW_LOG, true),
            (1_000_010, Some(6_i32), PREVIEW_LOG, true),
            (1_000_010, Some(0_i32), PREVIEW_LOG, false),
            (1_000_030, Some(0_i32), LOG, true),
        ] {
            assert!(
                matches!(
                    judge(app, code, LOGGED_IN, Some(log), preview),
                    Outcome::Failed(_)
                ),
                "{app} {code:?} {preview}"
            );
        }
    }

    /// The console of a run steamcmd 1791918946 made for Tidal Mill Inn 16 minutes after the
    /// testing branch's last build, before the CDN served that build's manifest.
    const UNSERVED: &str = "[2026-10-15 18:05:09]: Building depot 1000052...\r\n\
         \r\n\
         Building file mapping...\r\n\
         [2026-10-15 18:06:11]: ERROR! Failed to download manifest \"steampipe-partner.akamaized.net/depot/1000052/manifest/6076939079705630407/5/7060534466868254360\" (HTTP 404)\r\n\
         \r\n\
         [2026-10-15 18:06:11]: ERROR! Build for depot 1000052 failed : Failure\r\n";

    #[test]
    fn a_build_that_failed_for_a_manifest_the_cdn_does_not_serve_yet_says_where_it_is() {
        let expected = Some(Unserved {
            depot: 1_000_052,
            url: "https://steampipe-partner.akamaized.net/depot/1000052/manifest/\
                  6076939079705630407/5/7060534466868254360"
                .to_owned(),
        });
        assert_eq!(unserved(UNSERVED, None), expected);
        assert_eq!(unserved("", Some(UNSERVED)), expected);
        assert_eq!(
            unserved(&UNSERVED.replace("(HTTP 404)", "(HTTP 401)"), None),
            None
        );
        assert_eq!(unserved(LOGGED_IN, Some(LOG)), None);
    }

    /// The console of CI's preview of app 480 that steamcmd 1788292693 could not begin on Steam's
    /// build server, between runs minutes before and after that could.
    const NO_CONNECTION_AT_BUILD_CONSOLE: &str = "Connecting anonymously to Steam Public...OK\n\
         Waiting for client config...OK\n\
         Waiting for user info...OK\n\
         [2026-10-12 15:35:56]: Starting AppID 480 build (flags 0x2).\n\
         [2026-10-12 15:35:56]: Building depot 481...\n\
         \n\
         Building file mapping...\n\
         Scanning content\n\
         [2026-10-12 15:35:56]: ERROR! Failed to initialize build on server (No Connection) \n\
         \n\
         [2026-10-12 15:35:56]: ERROR! Build for depot 481 failed : Failure\n";

    /// The console of CI's anonymous upload of app 480 that Steam dropped steamcmd 1791918946
    /// from before it logged in, its colours left out.
    const DISCONNECTED_CONSOLE: &str = "Connecting anonymously to Steam Public...OK\n\
         Waiting for client config...OK\n\
         Waiting for user info...\n\
         steamcmd has been disconnected from steam with result 3 (No Connection)\n\
         ERROR! (Timed out)\n\
         Unloading Steam API...OK\n";

    #[test]
    fn a_build_that_may_pass_when_steamcmd_runs_again_says_why() {
        let no_connection = Some(Retry::NoConnection);
        assert_eq!(retry(NO_CONNECTION_AT_BUILD_CONSOLE, None), no_connection);
        assert_eq!(retry(DISCONNECTED_CONSOLE, None), Some(Retry::Disconnected));
        assert_eq!(
            retry("", Some(DISCONNECTED_CONSOLE)),
            Some(Retry::Disconnected)
        );
        assert_eq!(
            judge(480, Some(6_i32), DISCONNECTED_CONSOLE, None, true),
            Outcome::Failed(vec!["(Timed out)".to_owned()]),
            "a failure, so the build is tried again"
        );
        assert_eq!(
            retry("", Some(NO_CONNECTION_AT_BUILD_CONSOLE)),
            no_connection
        );
        assert!(matches!(
            retry(UNSERVED, Some(NO_CONNECTION_AT_BUILD_CONSOLE)),
            Some(Retry::Unserved(Unserved {
                depot: 1_000_052,
                ..
            }))
        ));
        for (console, log) in [
            (LOGGED_IN, Some(LOG)),
            (
                "Logging in user 'build_bot' to Steam Public...FAILED (No Connection)\r\n",
                None,
            ),
            (
                "",
                Some("[..]: ERROR! Failed to initialize build on server (Access Denied)\n"),
            ),
        ] {
            assert_eq!(retry(console, log), None, "{console}");
        }
    }

    #[test]
    fn only_a_manifest_address_of_steams_shape_is_asked_for() {
        let at = |printed: &str| manifest_at(printed).map(|found| found.url);
        assert_eq!(
            at("cdn.example/depot/7/manifest/1/5/2").as_deref(),
            Some("https://cdn.example/depot/7/manifest/1/5/2")
        );
        assert_eq!(
            at("127.0.0.1:4321/depot/7/manifest/1/5/2").as_deref(),
            Some("http://127.0.0.1:4321/depot/7/manifest/1/5/2")
        );
        for printed in [
            "localhost/depot/7/manifest/1/5/2",
            "cdn.example:443/depot/7/manifest/1/5/2",
            "user@cdn.example/depot/7/manifest/1/5/2",
            "127.0.0.1:/depot/7/manifest/1/5/2",
            "cdn.example/depot/7/manifest/1/5",
            "cdn.example/depot/7/manifest/1/5/2/3",
            "cdn.example/depot/7/manifest/1/x/2",
            "cdn.example/depot/7/manifest/1//2",
            "cdn.example/depot/x/manifest/1/5/2",
            "cdn.example/depots/7/manifest/1/5/2",
            "cdn.example/depot/7/chunk/1/5/2",
        ] {
            assert_eq!(at(printed), None, "{printed}");
        }
    }

    #[test]
    fn an_upload_that_never_got_the_manifest_says_to_upload_later() {
        let mut outcome = Outcome::Failed(vec!["Build for depot 5 failed : Failure".to_owned()]);
        still_unserved(&mut outcome, 5);
        assert_eq!(
            outcome,
            Outcome::Failed(vec![
                "Build for depot 5 failed : Failure".to_owned(),
                "Steam's CDN still does not serve depot 5's last manifest, which steamcmd builds \
                 the upload against; upload again later"
                    .to_owned()
            ])
        );
    }

    #[test]
    fn a_refused_commit_names_the_branch_it_would_have_set_live() {
        // As steamcmd 1788292693 reported an upload whose SetLive branch did not exist yet.
        let refused = vec!["Failed to commit build for AppID 1000010 : Failure".to_owned()];
        let why = commit_refused(&refused, Some("testing")).unwrap();
        assert!(why.starts_with("\"SetLive\" names \"testing\": "), "{why}");
        assert!(
            why.ends_with("once its default branch has a build live"),
            "{why}"
        );
        assert_eq!(commit_refused(&refused, None), None);
        let other = vec!["Failed to initialize build on server (Access Denied)".to_owned()];
        assert_eq!(commit_refused(&other, Some("testing")), None);
    }

    #[test]
    fn a_missing_branch_on_a_new_app_says_the_default_branch_needs_a_build_first() {
        let branch = |name: &str, build_id| webapi::Branch {
            name: name.to_owned(),
            build_id,
            description: String::new(),
            locked: false,
        };
        assert_eq!(
            no_branch(5, "testing", &[branch("public", 0)]),
            "\"SetLive\" names \"testing\", but app 5 has no branch by that name; set a build \
             live on its default branch, then create it, both in Steamworks under SteamPipe, \
             Builds"
        );
        for live in [
            vec![branch("public", 9)],
            vec![branch("public", 9), branch("beta", 0)],
        ] {
            assert_eq!(
                no_branch(5, "testing", &live),
                "\"SetLive\" names \"testing\", but app 5 has no branch by that name; create it \
                 in Steamworks under SteamPipe, Builds first",
                "{live:?}"
            );
        }
    }

    #[test]
    fn a_branch_is_confirmed_live_only_with_the_build_just_uploaded() {
        let branches = [
            webapi::Branch {
                name: "public".to_owned(),
                build_id: 20_000_001,
                description: String::new(),
                locked: false,
            },
            webapi::Branch {
                name: "testing".to_owned(),
                build_id: 20_000_740,
                description: String::new(),
                locked: true,
            },
        ];
        assert_eq!(live_now(&branches, "Testing", 20_000_740), Live::Confirmed);
        assert_eq!(
            live_now(&branches, "testing", 20_000_773),
            Live::Other(20_000_740)
        );
        assert_eq!(live_now(&branches, "beta", 20_000_740), Live::Missing);
    }

    #[test]
    fn the_build_an_upload_made_is_the_newest_with_its_description() {
        let build = |build_id, description: &str| webapi::Build {
            build_id,
            description: description.to_owned(),
            created: 0,
        };
        // As Steam listed Falcon Ridge Overseer's: two uploads of one commit, newest first.
        let builds = [
            build(20_000_001, "0.1.0-test 0a1b2c3d4e5f"),
            build(19_999_999, "0.1.0-test 0a1b2c3d4e5f"),
            build(19_999_998, "0.1.0-test 5f4e3d2c1b0a"),
        ];
        assert_eq!(
            uploaded_as(&builds, "0.1.0-test 0a1b2c3d4e5f"),
            Some(20_000_001)
        );
        assert_eq!(
            uploaded_as(&builds, "0.1.0-test 5f4e3d2c1b0a"),
            Some(19_999_998)
        );
        assert_eq!(uploaded_as(&builds, "0.1.0-test"), None);
    }

    #[test]
    fn a_finished_line_is_read_for_its_app_and_nothing_else() {
        assert_eq!(
            finished(5, "[..]: Successfully finished AppID 5 build (BuildID 12)."),
            Some(Finished::Build(12))
        );
        assert_eq!(
            finished(5, "Successfully finished AppID 5 build preview."),
            Some(Finished::Preview)
        );
        for line in [
            "Successfully finished AppID 55 build preview.",
            "Successfully finished AppID 5 build previewing",
            "Successfully finished AppID 5 build (BuildID x).",
            "Successfully finished AppID 5 build",
        ] {
            assert_eq!(finished(5, line), None, "{line}");
        }
    }

    #[test]
    fn a_failure_after_logging_in_is_not_a_failure_to_log_in() {
        let console = format!("{LOGGED_IN}ERROR! FAILED to upload a chunk\r\n");
        assert!(matches!(
            judge(1_000_030, Some(6_i32), &console, None, false),
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
                1_000_030,
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
                judge(1_000_030, code, &console, Some(LOG), false),
                Outcome::BuiltThenFailed {
                    build_id: 18_273_645,
                    reasons: vec!["Failed to set build live on branch testing".to_owned()],
                },
                "{code:?}"
            );
        }
        assert!(
            matches!(
                judge(1_000_030, Some(6_i32), &console, Some(LOG), true),
                Outcome::Failed(_)
            ),
            "a preview builds nothing to keep"
        );
        assert!(matches!(
            judge(
                1_000_030,
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
                   [..]: ERROR! Build for depot 1000031 failed : Failure\n";
        let console = "[..]: ERROR! Build for depot 1000031 failed : Failure\n";
        assert_eq!(
            judge(1_000_030, Some(6_i32), console, Some(log), false),
            Outcome::Failed(vec![
                "Failed to initialize build on server (Access Denied)".to_owned(),
                "Build for depot 1000031 failed : Failure".to_owned(),
            ])
        );
    }

    /// steamcmd's console in a Docker container with the default seccomp filter, as it was.
    const NO_SOCKET: &str = "Loading Steam API...CreateBoundSocket: failed to create socket, \
        error [no name available] (38)\r\nOK\r\n\
        Connecting anonymously to Steam Public...Retrying... \r\n\
        CreateBoundSocket: failed to create socket, error [no name available] (38)\r\n\
        FAILED (No Connection)\r\n";

    #[test]
    fn a_socket_steamcmd_cannot_open_is_named_with_what_to_do_in_a_container() {
        assert_eq!(
            judge(480, Some(5_i32), NO_SOCKET, None, true),
            Outcome::Failed(vec![
                "steamcmd could not open a network socket: in a Docker container, the default \
                 seccomp filter refuses the socket call 32-bit steamcmd makes; run the container \
                 with --security-opt seccomp=unconfined"
                    .to_owned(),
                "steamcmd could not connect to Steam (No Connection)".to_owned(),
            ])
        );
        assert_eq!(
            reasons(
                Some(5_i32),
                "CreateBoundSocket: failed to create socket, error [refused] (13)\r\n",
                None
            ),
            [
                "steamcmd could not open a network socket: failed to create socket, error [refused] (13)"
            ]
        );
    }

    #[test]
    fn a_login_that_never_reached_steam_is_not_one_steam_refused() {
        let unreachable =
            "Logging in user 'build_bot' to Steam Public...FAILED (No Connection)\r\n";
        assert_eq!(refused_login(unreachable), None);
        assert_eq!(
            judge_login(Some(5_i32), unreachable),
            Login::Failed(vec![
                "steamcmd could not connect to Steam (No Connection)".to_owned()
            ])
        );
        assert!(matches!(
            judge(1, Some(5_i32), unreachable, None, false),
            Outcome::Failed(_)
        ));
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
    fn a_run_stopped_for_want_of_certificates_says_so_rather_than_that_it_ran_out_of_time() {
        // As steamcmd printed it with the certificate store hidden, before it waited for good.
        let console = "src/common/opensslconnection.cpp (1706) : unable to load trusted SSL root \
                       certificates\nLoading Steam API...src/common/opensslconnection.cpp (1706) : \
                       unable to load trusted SSL root certificates\nOK\n\
                       Connecting anonymously to Steam Public...";
        assert_eq!(
            judge(480, None, console, None, true),
            Outcome::Failed(vec![
                "steamcmd found no certificates to check Steam's servers against, and would have \
                 waited for good; install the system's certificate store (ca-certificates)"
                    .to_owned()
            ])
        );
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
