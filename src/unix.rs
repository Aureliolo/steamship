//! What only Unix does. Kept in a module of its own so that everything here is compiled, tested
//! and mutation-tested on the systems where it runs.
#![expect(
    unsafe_code,
    reason = "stopping a process group, catching the signals that end steamship, opening a pseudo terminal and changing terminal modes are libc calls"
)]

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::fs::{PermissionsExt as _, symlink as make_symlink};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::panic;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::{ptr, slice, str};

use crate::run::Finished;
use crate::{elf, magic};

/// How often a running program is looked at to see whether it has ended.
const POLL: Duration = Duration::from_millis(50);

/// The process group of the program steamship is running, or 0 while none is. A program in a
/// group of its own is out of reach of the terminal's Ctrl+C, so a signal that ends steamship
/// ends this group first.
static RUNNING: AtomicI32 = AtomicI32::new(0);

/// The signals that end steamship from outside: Ctrl+C, the terminal closing, and `kill`.
const ENDING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGHUP, libc::SIGTERM];

/// Marks a process group as the one running, until dropped.
#[derive(Debug)]
struct Running(i32);

impl Running {
    fn mark(group: i32) -> Self {
        static CATCHING: Once = Once::new();
        CATCHING.call_once(catch_ending_signals);
        RUNNING.store(group, Ordering::SeqCst);
        Self(group)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Cleared, so that a signal after the group has ended cannot reach another that has
        // since been given its number.
        let _: Result<i32, i32> =
            RUNNING.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

#[expect(
    clippy::as_conversions,
    clippy::fn_to_numeric_cast_any,
    reason = "signal takes its handler as the address libc defines sighandler_t to be"
)]
fn catch_ending_signals() {
    for signal in ENDING {
        // SAFETY: the handler does only what a signal handler may: an atomic load, killpg,
        // signal and raise.
        let before = unsafe { libc::signal(signal, on_ending as libc::sighandler_t) };
        if before == libc::SIG_IGN {
            // A signal steamship was started to ignore, as under nohup, stays ignored.
            // SAFETY: puts back the disposition it had.
            let _: libc::sighandler_t = unsafe { libc::signal(signal, libc::SIG_IGN) };
        }
    }
}

extern "C" fn on_ending(signal: libc::c_int) {
    let group = RUNNING.load(Ordering::SeqCst);
    if group != 0 {
        stop_group(group);
    }
    // SAFETY: puts back the default disposition, which a signal handler may do.
    let _: libc::sighandler_t = unsafe { libc::signal(signal, libc::SIG_DFL) };
    // SAFETY: with the default disposition back, the signal ends steamship as it would have.
    let _: libc::c_int = unsafe { libc::raise(signal) };
}

/// [`crate::run::run`], for Unix: the program leads a process group of its own, so that stopping
/// it stops whatever it started too.
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
) -> io::Result<Finished> {
    let (mut reader, writer) = io::pipe()?;
    let mut child = Command::new(program)
        .args(args)
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .process_group(0)
        .spawn()?;
    let group = i32::try_from(child.id()).map_err(io::Error::other)?;
    let _running = Running::mark(group);
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

/// Only Windows puts up a dialogue of its own when a program crashes.
pub const fn silence_error_dialogues() {}

/// A program running on a pseudo terminal.
///
/// The program reads and writes it as the terminal it takes it for: one that shows its prompts
/// at once and hides what is typed at a password. It leads a process group of its own, ended as
/// one when it ends.
///
/// Dropped before it is waited for, it ends the program and everything the program started.
#[derive(Debug)]
pub struct Terminal {
    group: i32,
    /// Waits for the program, then ends its process group: whatever the program left running
    /// would otherwise hold the terminal, and so its output, open.
    watching: Option<JoinHandle<io::Result<ExitStatus>>>,
}

/// What a program writes to its pseudo terminal. Once the program and everything it started have
/// closed the terminal, reading fails with `EIO`: that is the end of the output, and read as such.
#[derive(Debug)]
pub struct Output(File);

impl io::Read for Output {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
            read => read,
        }
    }
}

impl Terminal {
    /// Starts `program`, and answers it with what it writes, escapes and all, and what it reads
    /// as typed. The output ends once the program has.
    ///
    /// # Errors
    ///
    /// When the terminal or the process cannot be made.
    pub fn start(
        program: &Path,
        args: &[OsString],
        environment: &[(OsString, OsString)],
        directory: &Path,
    ) -> io::Result<(Self, Output, File)> {
        let mut main = -1_i32;
        let mut replica = -1_i32;
        // SAFETY: both are valid places for a descriptor; the name, the settings and the size
        // are left to the system.
        let opened = unsafe {
            libc::openpty(
                &raw mut main,
                &raw mut replica,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if opened != 0_i32 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openpty succeeded, so both descriptors are new and owned by nobody else.
        let main = unsafe { OwnedFd::from_raw_fd(main) };
        // SAFETY: as above.
        let replica = unsafe { OwnedFd::from_raw_fd(replica) };
        // What is typed must never come back in the output, where it would be read, so the
        // terminal does not echo it, whatever the program itself asks for.
        quiet(replica.as_raw_fd())?;
        let mut child = Command::new(program)
            .args(args)
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .current_dir(directory)
            .stdin(Stdio::from(replica.try_clone()?))
            .stdout(Stdio::from(replica.try_clone()?))
            .stderr(Stdio::from(replica))
            .process_group(0)
            .spawn()?;
        let output = Output(File::from(main.try_clone()?));
        let group = i32::try_from(child.id()).map_err(io::Error::other)?;
        let running = Running::mark(group);
        let watching = thread::spawn(move || {
            let status = child.wait();
            stop_group(group);
            drop(running);
            status
        });
        let terminal = Self {
            group,
            watching: Some(watching),
        };
        Ok((terminal, output, File::from(main)))
    }

    /// Waits for the program to end and says its exit code, or the shell's stand-in for one
    /// when a signal ended it.
    ///
    /// # Errors
    ///
    /// When waiting fails.
    pub fn wait(mut self) -> io::Result<Option<i32>> {
        let status = self
            .watching
            .take()
            .map(|watching| {
                watching
                    .join()
                    .unwrap_or_else(|panic| panic::resume_unwind(panic))
            })
            .transpose()?;
        Ok(status.and_then(|status| {
            status
                .code()
                .or_else(|| status.signal().map(|signal| signal.saturating_add(128)))
        }))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(watching) = self.watching.take() {
            stop_group(self.group);
            // Joined, so that the program is reaped rather than left a zombie while steamship runs.
            drop(watching.join());
        }
    }
}

/// The keyboard, one character at a time as it is typed and without the terminal showing it.
///
/// The terminal's echo, line editing and signal keys are off until this is dropped, which puts
/// them back.
#[derive(Debug)]
pub struct Keys {
    saved: libc::termios,
}

impl Keys {
    /// The keyboard, or none when input does not come from a terminal.
    ///
    /// # Errors
    ///
    /// When the terminal will not change modes.
    pub fn open() -> io::Result<Option<Self>> {
        // SAFETY: asks about a descriptor; nothing is changed.
        if unsafe { libc::isatty(libc::STDIN_FILENO) } == 0_i32 {
            return Ok(None);
        }
        let mut saved = MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `saved` is a valid place for the settings.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, saved.as_mut_ptr()) } != 0_i32 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: tcgetattr succeeded, and on success it writes all of them.
        let saved = unsafe { saved.assume_init() };
        let mut raw = saved;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
        if let Some(least) = raw.c_cc.get_mut(libc::VMIN) {
            *least = 1;
        }
        if let Some(wait) = raw.c_cc.get_mut(libc::VTIME) {
            *wait = 0;
        }
        // SAFETY: the settings are the terminal's own with three flags and two counts changed.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const raw) } != 0_i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self { saved }))
    }

    /// The next character typed, or none when input has ended.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read.
    pub fn read_key(&mut self) -> io::Result<Option<char>> {
        let mut bytes = [0_u8; 4];
        let Some((first, rest)) = bytes.split_first_mut() else {
            return Ok(None);
        };
        let length = {
            let mut stdin = io::stdin().lock();
            if stdin.read(slice::from_mut(first))? == 0 {
                return Ok(None);
            }
            let length: usize = match *first {
                0xc0..=0xdf => 1,
                0xe0..=0xef => 2,
                0xf0..=0xf7 => 3,
                _ => 0,
            };
            stdin.read_exact(rest.get_mut(..length).unwrap_or_default())?;
            length
        };
        let decoded = str::from_utf8(bytes.get(..=length).unwrap_or_default())
            .ok()
            .and_then(|text| text.chars().next());
        bytes.fill(0);
        Ok(Some(decoded.unwrap_or(char::REPLACEMENT_CHARACTER)))
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        // SAFETY: puts back the settings the terminal had when this was opened.
        let _: i32 =
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const self.saved) };
    }
}

/// Turns off the echo of the terminal at `descriptor`.
fn quiet(descriptor: RawFd) -> io::Result<()> {
    let mut settings = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `settings` is a valid place for the settings.
    if unsafe { libc::tcgetattr(descriptor, settings.as_mut_ptr()) } != 0_i32 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: tcgetattr succeeded, and on success it writes all of them.
    let mut settings = unsafe { settings.assume_init() };
    settings.c_lflag &= !(libc::ECHO | libc::ECHONL);
    // SAFETY: the settings are the terminal's own with its echo off.
    if unsafe { libc::tcsetattr(descriptor, libc::TCSANOW, &raw const settings) } != 0_i32 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn stop_group(group: i32) {
    // SAFETY: `killpg` only sends a signal; a group that has already gone is an error it
    // reports and that nothing here depends on.
    let _: i32 = unsafe { libc::killpg(group, libc::SIGKILL) };
}

/// Says what is wrong when `file` is a Linux program that nobody may execute.
#[must_use]
pub fn missing_executable_bit(file: &Path) -> Option<String> {
    let mut opened = match File::open(file) {
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

/// Leaves `folder` to the current user alone, and says whether it had to change anything.
///
/// # Errors
///
/// When the folder's permissions cannot be read or set.
pub fn restrict(folder: &Path) -> io::Result<bool> {
    let mode = fs::metadata(folder)?.permissions().mode() & 0o7777;
    if mode == 0o700 {
        return Ok(false);
    }
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700))?;
    Ok(true)
}

/// Makes `link` a symbolic link to `target`, which is written as given.
///
/// # Errors
///
/// When the file system refuses, or `link` exists.
pub fn make_link(target: &str, link: &Path) -> io::Result<()> {
    make_symlink(target, link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restricted_folder_is_its_users_alone() {
        let folder = tempfile::tempdir().unwrap();
        fs::set_permissions(folder.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(restrict(folder.path()).unwrap());
        let mode = fs::metadata(folder.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o700);
        assert!(!restrict(folder.path()).unwrap(), "already so");
    }

    #[test]
    fn a_folder_that_is_not_there_cannot_be_restricted() {
        let folder = tempfile::tempdir().unwrap();
        let error = restrict(&folder.path().join("missing")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
