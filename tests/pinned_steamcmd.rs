//! The steamcmd `steamship install` put into `STEAMSHIP_HOME`, started for real. `cargo test`
//! leaves these out. CI runs them with `--ignored` once it has installed the pinned steamcmd.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::thread;
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

/// How long the first of two steamcmd in one home is held: past the ten seconds its log may take
/// to come to rest and the hold of the second.
const FIRST: Duration = Duration::from_secs(30);

/// The commands that log steamcmd in anonymously and leave it at its prompt.
const LOGIN: [&str; 6] = [
    "+@ShutdownOnFailedCommand",
    "1",
    "+@NoPromptForPassword",
    "1",
    "+login",
    "anonymous",
];

/// Both tests start steamcmd in the one home, so they take turns.
static HOME: Mutex<()> = Mutex::new(());

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
    let _turn = HOME.lock().unwrap_or_else(PoisonError::into_inner);
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

/// A steamcmd still running in a home keeps its logs to itself on Windows, and a second one there
/// works on without a line in them, which steamship would take for one that never started. So
/// either steamship turns the second away before it starts, or the second adds to the log too.
#[test]
#[ignore = "starts the steamcmd in STEAMSHIP_HOME, which CI installs from Valve"]
fn a_second_steamcmd_in_a_home_is_turned_away_or_adds_to_the_connection_log_too() {
    let _turn = HOME.lock().unwrap_or_else(PoisonError::into_inner);
    let home = home().expect("STEAMSHIP_HOME names the home");
    let folder = tempfile::tempdir().unwrap();
    let log = steamcmd::connection_log(&home, Platform::THIS);
    let length = || fs::metadata(&log).map_or(0, |found| found.len());
    let mut tries = Vec::new();
    // On macOS one start in 30 hangs before steamcmd's API has loaded, so a try may fail.
    for _ in 0_u8..3_u8 {
        let before = length();
        let first = {
            let (home, input) = (home.clone(), folder.path().join("first"));
            thread::spawn(move || round(&home, &input, FIRST, None))
        };
        // The first has settled once its log has grown and then stood still for two seconds.
        let began = Instant::now();
        let (mut last, mut still) = (before, Instant::now());
        while began.elapsed() < Duration::from_secs(10)
            && (last == before || still.elapsed() < Duration::from_secs(2))
        {
            thread::sleep(Duration::from_millis(100));
            if length() != last {
                (last, still) = (length(), Instant::now());
            }
        }
        let free = steamcmd::log_is_free(&home, Platform::THIS);
        let second = (last != before && free.is_ok())
            .then(|| round(&home, &folder.path().join("second"), HOLD, Some(START)).unwrap());
        let first = first.join().unwrap().unwrap();
        let stayed = last != before && first.0.code.is_none() && first.1 >= FIRST;
        let turned_away = free.as_ref().is_err_and(|refusal| {
            refusal
                .to_string()
                .contains("End any steamcmd left running")
        });
        let added = second
            .as_ref()
            .is_some_and(|second| second.0.code.is_none() && second.1 >= HOLD);
        if stayed && (turned_away || added) {
            return;
        }
        tries.push(format!(
            "The first added {} bytes to its log. {}\nsteamship's answer for a second: {free:?}\n{}",
            last.saturating_sub(before),
            shown(&first),
            second
                .as_ref()
                .map_or_else(|| "No second was started.".to_owned(), shown)
        ));
    }
    panic!(
        "In none of three tries did a first steamcmd stay at its prompt for {FIRST:?} while a \
         second was turned away, or was held for {HOLD:?} and added to the connection log within \
         {START:?}.\n{}",
        tries.join("\n")
    );
}
