//! Running steamcmd: out of sight of whoever is at the machine, fed nothing on its input, heard in
//! full, and stopped, together with everything it started, when it runs past its time.
//!
//! On Windows a hidden window is not enough: it still takes the keyboard. So the program runs on
//! a desktop of its own, which Windows keeps focus from leaving, inside a job that ends every
//! process it starts. Elsewhere it runs in a process group of its own, for the same ending.

use std::ffi::OsString;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Runs `program` with `args` and `environment` added to steamship's, in `directory`.
///
/// It is waited for at most `limit`, or until it writes one of `hopeless`: words after which it
/// never succeeds, only waits. Stopped either way, it has no exit code.
///
/// # Errors
///
/// When the program cannot be started or its output cannot be read.
pub fn run(
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    directory: &Path,
    limit: Duration,
    hopeless: &'static [&'static str],
) -> io::Result<Finished> {
    native::run(program, args, environment, directory, limit, hopeless)
}

/// Reads `output` to its end, and sets `seen` once it has held one of `hopeless`.
///
/// # Errors
///
/// When the output cannot be read.
pub fn watch<Output>(output: Output, hopeless: &[&str], seen: &AtomicBool) -> io::Result<Vec<u8>>
where
    Output: Read,
{
    let mut written = Vec::new();
    // Byte by byte, so that words split across reads need no looking back, and so that the end
    // of the output and an interrupted read are std's to tell, not a loop of this crate's.
    for byte in BufReader::new(output).bytes() {
        written.push(byte?);
        if hopeless
            .iter()
            .any(|words| written.ends_with(words.as_bytes()))
        {
            seen.store(true, Ordering::Relaxed);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    /// A shell, what makes it run the one line that follows, a line that starts a grandchild
    /// holding the output open for a minute and then waits a minute itself, and one that starts
    /// such a grandchild and exits 4 at once. `cmd` keeps the quotes around a line holding `&` or
    /// `>` unless told `/s`, `/d` keeps it from running anything set up to run whenever it
    /// starts, and its own path is written with backslashes because it reads any `/` on its
    /// command line, its own path included, as the start of a switch.
    #[cfg(windows)]
    const SHELL: (&str, &[&str], &str, &str) = (
        r"C:\Windows\System32\cmd.exe",
        &["/d", "/s", "/c"],
        "start /b ping -n 60 127.0.0.1 > nul & ping -n 60 127.0.0.1",
        "start /b ping -n 60 127.0.0.1 > nul & exit 4",
    );
    #[cfg(unix)]
    const SHELL: (&str, &[&str], &str, &str) = (
        "/bin/sh",
        &["-c"],
        "sleep 60 & sleep 60",
        "sleep 60 & exit 4",
    );

    fn shell_args(line: &str) -> Vec<OsString> {
        SHELL
            .1
            .iter()
            .copied()
            .chain([line])
            .map(OsString::from)
            .collect()
    }

    fn shell(line: &str, limit: Duration) -> Finished {
        run(
            Path::new(SHELL.0),
            &shell_args(line),
            &[],
            &env::temp_dir(),
            limit,
            &[],
        )
        .unwrap()
    }

    /// A line that says it will never finish, then waits a minute anyway.
    #[cfg(windows)]
    const HOPELESS_LINE: &str = "echo no way through& ping -n 60 127.0.0.1 > nul";
    #[cfg(unix)]
    const HOPELESS_LINE: &str = "echo no way through; sleep 60";

    #[test]
    fn stops_a_program_once_it_says_it_never_will_finish() {
        let started = Instant::now();
        let finished = run(
            Path::new(SHELL.0),
            &shell_args(HOPELESS_LINE),
            &[],
            &env::temp_dir(),
            // Far longer than the line takes to say it, and short enough that a runner which
            // never hears it fails here soon.
            Duration::from_secs(15),
            &["no way through"],
        )
        .unwrap();
        assert_eq!(finished.code, None);
        assert!(String::from_utf8_lossy(&finished.output).contains("no way through"));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }

    /// Gives what it holds one byte at a time, as a pipe may.
    struct Trickle<'bytes>(&'bytes [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let (Some(first), Some(slot)) = (self.0.first(), buf.first_mut()) else {
                return Ok(0);
            };
            *slot = *first;
            self.0 = self.0.get(1..).unwrap_or_default();
            Ok(1)
        }
    }

    #[test]
    fn words_split_across_reads_are_seen_and_the_output_kept_whole() {
        let written = b"Loading...unable to load trusted SSL root certificates\nConnecting";
        let seen = AtomicBool::new(false);
        let output = watch(
            Trickle(written),
            &["unable to load trusted SSL root certificates"],
            &seen,
        )
        .unwrap();
        assert_eq!(output, written);
        assert!(seen.load(Ordering::Relaxed));
        let unseen = AtomicBool::new(false);
        drop(watch(Trickle(written), &["never said"], &unseen).unwrap());
        assert!(!unseen.load(Ordering::Relaxed));
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

    /// A line that reads its input to the end and exits 0 only when that input was empty.
    #[cfg(windows)]
    const READS_INPUT: &str = "findstr /r \"^\" > nul && exit 3 || exit 0";
    #[cfg(unix)]
    const READS_INPUT: &str = "test -z \"$(cat)\"";

    #[test]
    fn a_program_is_given_nothing_to_read() {
        let finished = shell(READS_INPUT, Duration::from_secs(20));
        assert_eq!(
            finished.code,
            Some(0_i32),
            "{}",
            String::from_utf8_lossy(&finished.output)
        );
    }

    #[test]
    fn stops_what_a_program_left_running_when_it_ends() {
        let started = Instant::now();
        let finished = shell(SHELL.3, Duration::from_secs(50));
        // Again the grandchild holds the output open, so the run ends before the grandchild's
        // minute is up only if the grandchild was stopped when its parent ended.
        assert_eq!(finished.code, Some(4_i32));
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "took {:?}",
            started.elapsed()
        );
    }

    /// A shell line that exits 0 only when `STEAMSHIP_PROBE` is `yes` and the folder it runs in
    /// holds `marker`.
    #[cfg(windows)]
    const PROBE: &str =
        r#"if "%STEAMSHIP_PROBE%"=="yes" (if exist marker (exit 0) else (exit 5)) else (exit 5)"#;
    #[cfg(unix)]
    const PROBE: &str = r#"test "$STEAMSHIP_PROBE" = yes && test -f marker || exit 5"#;

    fn probe() -> [(OsString, OsString); 1] {
        [(OsString::from("STEAMSHIP_PROBE"), OsString::from("yes"))]
    }

    /// Runs a shell `line` on a terminal, with `typed` typed into it, and gives its exit code and
    /// all it wrote.
    fn on_terminal(
        args: &[OsString],
        environment: &[(OsString, OsString)],
        directory: &Path,
        typed: &[u8],
    ) -> (Option<i32>, String) {
        let (terminal, output, mut input) =
            native::Terminal::start(Path::new(SHELL.0), args, environment, directory).unwrap();
        input.write_all(typed).unwrap();
        let written = to_the_end(output);
        drop(input);
        (
            terminal.wait().unwrap(),
            String::from_utf8_lossy(&written).into_owned(),
        )
    }

    /// All of `output`, which must end within half a minute: output that never ends fails the
    /// test rather than holding it up for good.
    fn to_the_end<Output>(mut output: Output) -> Vec<u8>
    where
        Output: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let _reading = thread::spawn(move || {
            let mut written = Vec::new();
            let read = output.read_to_end(&mut written).map(|_| written);
            drop(sender.send(read));
        });
        receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("the output never ended")
            .unwrap()
    }

    fn terminal_shell(environment: &[(OsString, OsString)], directory: &Path) -> Option<i32> {
        on_terminal(&shell_args(PROBE), environment, directory, b"").0
    }

    fn hidden_shell(environment: &[(OsString, OsString)], directory: &Path) -> Option<i32> {
        let limit = Duration::from_secs(30);
        run(
            Path::new(SHELL.0),
            &shell_args(PROBE),
            environment,
            directory,
            limit,
            &[],
        )
        .unwrap()
        .code
    }

    #[test]
    fn a_program_on_a_terminal_gets_its_environment_and_folder_and_is_waited_for() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("marker"), "").unwrap();
        assert_eq!(terminal_shell(&probe(), folder.path()), Some(0_i32));
        assert_eq!(terminal_shell(&[], folder.path()), Some(5_i32));
        assert_eq!(terminal_shell(&probe(), &env::temp_dir()), Some(5_i32));
    }

    /// A line that reads a line typed at its prompt and exits 0 only when that was `hunter2`.
    /// `cmd` expands `%typed%` as it reads its line, before anything has been typed, and leaves it
    /// as it is while there is no such variable; the `cmd` it starts then expands it as it reads
    /// its own line, after `set /p` has set it.
    #[cfg(windows)]
    const READS_TYPED: &str =
        "set /p typed=password: & cmd /d /c if \"%typed%\"==\"hunter2\" (exit 0) else (exit 5)";
    #[cfg(unix)]
    const READS_TYPED: &str =
        "printf 'password: '; read -r typed; test \"$typed\" = hunter2 || exit 5";

    #[test]
    fn what_is_typed_reaches_a_program_on_a_terminal() {
        let args = shell_args(READS_TYPED);
        let (code, written) = on_terminal(&args, &[], &env::temp_dir(), b"hunter2\r");
        assert_eq!(code, Some(0_i32), "{written:?}");
        assert!(written.contains("password:"), "{written:?}");
        let (wrong, _) = on_terminal(&args, &[], &env::temp_dir(), b"hunter3\r");
        assert_eq!(wrong, Some(5_i32));
    }

    /// Windows leaves echo to the program, which sets its own console's modes; steamcmd hides a
    /// password as it is typed.
    #[cfg(unix)]
    #[test]
    fn what_is_typed_on_a_terminal_is_not_echoed() {
        let (code, written) = on_terminal(
            &shell_args(READS_TYPED),
            &[],
            &env::temp_dir(),
            b"hunter2\r",
        );
        assert_eq!(code, Some(0_i32), "{written:?}");
        assert!(!written.contains("hunter2"), "{written:?}");
    }

    #[test]
    fn the_output_of_a_program_on_a_terminal_ends_when_it_does_whatever_it_left_running() {
        let (code, _) = on_terminal(&shell_args(SHELL.3), &[], &env::temp_dir(), b"");
        assert_eq!(code, Some(4_i32));
    }

    #[test]
    fn a_program_on_a_terminal_dropped_unwaited_is_ended() {
        // The line runs for a minute, and holds the output open, unless it is ended.
        let (terminal, output, _input) = native::Terminal::start(
            Path::new(SHELL.0),
            &shell_args(SHELL.2),
            &[],
            &env::temp_dir(),
        )
        .unwrap();
        drop(terminal);
        drop(to_the_end(output));
    }

    #[test]
    fn a_hidden_program_gets_its_environment_and_folder() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("marker"), "").unwrap();
        assert_eq!(hidden_shell(&probe(), folder.path()), Some(0_i32));
        assert_eq!(hidden_shell(&[], folder.path()), Some(5_i32));
        assert_eq!(hidden_shell(&probe(), &env::temp_dir()), Some(5_i32));
    }

    #[test]
    fn a_program_on_a_terminal_that_is_not_there_is_an_error() {
        let missing = env::temp_dir().join("steamship-no-such-program");
        let error = native::Terminal::start(&missing, &[], &[], &env::temp_dir()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        let missing = env::temp_dir().join("steamship-no-such-program");
        let limit = Duration::from_secs(5);
        let error = run(&missing, &[], &[], &env::temp_dir(), limit, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
