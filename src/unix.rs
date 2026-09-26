//! What only Unix does. Kept in a module of its own so that everything here is compiled, tested
//! and mutation-tested on the systems where it runs.
#![expect(unsafe_code, reason = "stopping a whole process group is a libc call")]

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read as _};
use std::os::unix::fs::{PermissionsExt as _, symlink as make_symlink};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::panic;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::run::Finished;
use crate::{elf, magic};

/// How often a running program is looked at to see whether it has ended.
const POLL: Duration = Duration::from_millis(50);

/// [`crate::run::run`], for Unix: the program leads a process group of its own, so that stopping
/// it stops whatever it started too.
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
    let (mut reader, writer) = io::pipe()?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .process_group(0)
        .spawn()?;
    let group = i32::try_from(child.id()).map_err(io::Error::other)?;
    let reading = thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).map(|_| output)
    });
    let deadline = Instant::now().checked_add(limit);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break None;
        }
        thread::sleep(POLL);
    };
    // Whatever the program left running goes too, which is also what lets the reading end: a
    // straggler would hold the output open.
    stop_group(group);
    if status.is_none() {
        let _: ExitStatus = child.wait()?;
    }
    let output = reading
        .join()
        .unwrap_or_else(|panic| panic::resume_unwind(panic))?;
    // A program ended by a signal has no exit code; the shell's convention stands in for one.
    let code = status.and_then(|status| {
        status
            .code()
            .or_else(|| status.signal().map(|signal| signal.saturating_add(128)))
    });
    Ok(Finished { code, output })
}

fn stop_group(group: i32) {
    // SAFETY: `killpg` only sends a signal; a group that has already gone is an error it
    // reports and that nothing here depends on.
    let _: i32 = unsafe { libc::killpg(group, libc::SIGKILL) };
}

/// Says what is wrong when `file` is a Linux program that nobody may execute.
#[must_use]
pub fn missing_executable_bit(file: &Path) -> Option<String> {
    let mut opened = match fs::File::open(file) {
        Ok(opened) => opened,
        Err(error) => return Some(format!("cannot be read: {error}")),
    };
    match elf::is_program(&mut opened) {
        Ok(false) => None,
        Ok(true) => match opened.metadata() {
            Ok(metadata) if metadata.permissions().mode() & 0o111 == 0 => {
                Some("is a Linux program without its executable bit; run chmod +x on it".to_owned())
            }
            Ok(_) => None,
            Err(error) => Some(format!("cannot be read: {error}")),
        },
        Err(error) => Some(format!("cannot be read: {error}")),
    }
}

/// Valve's zips carry no permissions, so a program is made executable here, as steamcmd's own
/// installer does, and everything else readable only.
///
/// # Errors
///
/// When `path` cannot be read, or the file system refuses the change.
pub fn mark_if_program(path: &Path) -> io::Result<()> {
    let mode = if magic::looks_like_program(path)? {
        0o755
    } else {
        0o644
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Makes `link` a symbolic link to `target`, which is written as given.
///
/// # Errors
///
/// When the file system refuses, or `link` exists.
pub fn make_link(target: &str, link: &Path) -> io::Result<()> {
    make_symlink(target, link)
}
