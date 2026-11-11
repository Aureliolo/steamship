//! The steamcmd `steamship install` put into `STEAMSHIP_HOME`, started for real. `cargo test`
//! leaves this out. CI runs it with `--ignored` once it has installed the pinned steamcmd.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use steamship::install;
use steamship::platform::{self, Platform};
use steamship::run::{self, Finished, Startup};
use steamship::steamcmd;

/// How long steamcmd is held at its prompt before it is stopped.
const HOLD: Duration = Duration::from_secs(15);

/// The longest steamcmd may take here to add to its connection log: a sixth of the minute
/// steamship gives it, and eight times the slowest of twelve starts timed on CI's four systems.
const START: Duration = Duration::from_secs(10);

/// The commands that log steamcmd in anonymously and leave it at its prompt.
const LOGIN: [&str; 6] = [
    "+@ShutdownOnFailedCommand",
    "1",
    "+@NoPromptForPassword",
    "1",
    "+login",
    "anonymous",
];

/// The program and arguments that hold `file`, steamcmd, at its prompt after [`LOGIN`]. On Unix
/// its input is the pipe at `input`, made anew, which a `sleep` outlasting every hold keeps open
/// and never writes to. The shell becomes steamcmd, so what ends is steamcmd itself.
#[cfg(unix)]
fn held(file: &Path, input: &Path) -> (PathBuf, Vec<OsString>) {
    let line = "fifo=$1; shift; rm -f \"$fifo\"; mkfifo \"$fifo\"; sleep 60 > \"$fifo\" & exec \"$0\" \"$@\" < \"$fifo\"";
    let args = [OsString::from("-c"), OsString::from(line)]
        .into_iter()
        .chain([file.as_os_str().to_owned(), input.as_os_str().to_owned()])
        .chain(LOGIN.map(OsString::from))
        .collect();
    (PathBuf::from("/bin/sh"), args)
}

/// The program and arguments that hold `file`, steamcmd, at its prompt after [`LOGIN`]. On
/// Windows steamcmd waits there with no input at all.
#[cfg(windows)]
fn held(file: &Path, _input: &Path) -> (PathBuf, Vec<OsString>) {
    (file.to_path_buf(), LOGIN.map(OsString::from).into())
}

/// The home CI installed steamcmd into. Never `Platform::home`, whose fallback is the user's
/// own home and its saved login.
fn home() -> Option<PathBuf> {
    env::var_os(platform::HOME).map(PathBuf::from)
}

/// One steamcmd in `home`, held for `hold` at most and stopped sooner when it has not added to
/// its connection log within `start`, and how long it ran.
fn round(
    home: &Path,
    input: &Path,
    hold: Duration,
    start: Option<Duration>,
) -> io::Result<(Finished, Duration)> {
    let root = home.join(install::FOLDER);
    let log = steamcmd::connection_log(home, Platform::THIS);
    let (program, args) = held(&steamcmd::program(&root, Platform::THIS), input);
    let began = Instant::now();
    let finished = run::run(
        &program,
        &args,
        &steamcmd::environment(home, Platform::THIS),
        &root,
        hold,
        &[],
        start.map(|limit| Startup::new(limit, &log)),
    )?;
    Ok((finished, began.elapsed()))
}

/// What a round that did not last its hold shows of itself.
fn shown((finished, took): &(Finished, Duration)) -> String {
    format!(
        "It ended after {took:?} with exit code {:?}, having said:\n{}",
        finished.code,
        String::from_utf8_lossy(&finished.output)
    )
}

/// steamship stops a steamcmd that has added nothing to its connection log within a minute, as
/// one that never started. A steamcmd that wrote that log only as it ended would so be stopped
/// in every run past a minute. Here the start-up limit stops such a steamcmd before the hold is
/// over.
#[test]
#[ignore = "starts the steamcmd in STEAMSHIP_HOME, which CI installs from Valve"]
fn the_pinned_steamcmd_adds_to_its_connection_log_while_it_still_runs() {
    let home = home().expect("STEAMSHIP_HOME names the home");
    let folder = tempfile::tempdir().unwrap();
    let mut rounds = Vec::new();
    // On macOS one start in 30 hangs before steamcmd's API has loaded, so a round may fail.
    for _ in 0_u8..3_u8 {
        let ended = round(&home, &folder.path().join("input"), HOLD, Some(START)).unwrap();
        if ended.0.code.is_none() && ended.1 >= HOLD {
            return;
        }
        rounds.push(shown(&ended));
    }
    panic!(
        "steamcmd was not held at its prompt for {HOLD:?} in any of three rounds. steamship \
         stops one that adds nothing to {} within {START:?}. One that ends by itself was not \
         held.\n{}",
        steamcmd::connection_log(&home, Platform::THIS).display(),
        rounds.join("\n")
    );
}
