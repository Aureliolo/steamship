//! Running steamcmd: out of sight of whoever is at the machine, fed nothing on its input, heard in
//! full, and stopped, together with everything it started, when it runs past its time.
//!
//! On Windows a hidden window is not enough: it still takes the keyboard. So the program runs on
//! a desktop of its own, which Windows keeps focus from leaving, inside a job that ends every
//! process it starts. Elsewhere it runs in a process group of its own, for the same ending.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use crate::unix as native;
#[cfg(windows)]
use crate::windows as native;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// The exit code, or none when it ran past its time and was stopped.
    pub code: Option<i32>,
    /// Everything it wrote to either stream, in the order it wrote it.
    pub output: Vec<u8>,
}

/// Runs `program` with `args` in `directory`, and waits at most `limit` for it.
///
/// # Errors
///
/// When the program cannot be started or its output cannot be read.
pub fn run(
    program: &Path,
    args: &[OsString],
    directory: &Path,
    limit: Duration,
) -> io::Result<Finished> {
    native::run(program, args, directory, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::Instant;

    /// A shell, what makes it run the one line that follows, and the line that, in it, starts a
    /// grandchild that holds the output open for a minute. `cmd` keeps the quotes around a line
    /// holding `&` or `>` unless told `/s`, `/d` keeps it from running anything set up to run
    /// whenever it starts, and its own path is written with backslashes because it reads any
    /// `/` on its command line, its own path included, as the start of a switch.
    #[cfg(windows)]
    const SHELL: (&str, &[&str], &str) = (
        r"C:\Windows\System32\cmd.exe",
        &["/d", "/s", "/c"],
        "start /b ping -n 60 127.0.0.1 > nul & ping -n 60 127.0.0.1",
    );
    #[cfg(unix)]
    const SHELL: (&str, &[&str], &str) = ("/bin/sh", &["-c"], "sleep 60 & sleep 60");

    fn shell(line: &str, limit: Duration) -> Finished {
        let args: Vec<OsString> = SHELL
            .1
            .iter()
            .copied()
            .chain([line])
            .map(OsString::from)
            .collect();
        run(Path::new(SHELL.0), &args, &env::temp_dir(), limit).unwrap()
    }

    #[test]
    fn hears_both_streams_and_the_exit_code() {
        let finished = shell("echo out&& echo err 1>&2&& exit 3", Duration::from_secs(30));
        let output = String::from_utf8_lossy(&finished.output);
        assert_eq!(finished.code, Some(3_i32), "{output:?}");
        assert!(
            output.contains("out") && output.contains("err"),
            "{output:?}"
        );
    }

    #[test]
    fn stops_everything_it_started_once_its_time_is_up() {
        let started = Instant::now();
        let finished = shell(SHELL.2, Duration::from_secs(2));
        // The grandchild holds the output open: reading it to the end only finishes once the
        // grandchild has been stopped as well.
        assert_eq!(finished.code, None);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        let missing = env::temp_dir().join("steamship-no-such-program");
        let error = run(&missing, &[], &env::temp_dir(), Duration::from_secs(5)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
