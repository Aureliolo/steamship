//! What only Windows does. Kept in a module of its own so that everything here is compiled,
//! tested and mutation-tested on the system where it runs.
#![expect(
    unsafe_code,
    reason = "Windows' own APIs are the only way to ask Windows these questions"
)]

use std::env;
use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter, Read as _};
use std::iter;
use std::mem::MaybeUninit;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle};
use std::panic;
use std::path::Path;
use std::process;
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_NOT_FOUND, ERROR_SUCCESS, GENERIC_ALL, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL,
    INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorControl,
    GetTokenInformation, InitializeAcl, OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, COORD, ClosePseudoConsole, CreatePseudoConsole, ENABLE_ECHO_INPUT,
    ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, GetConsoleMode, GetStdHandle, HPCON, ReadConsoleW,
    STD_INPUT_HANDLE, SetConsoleMode,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX, SetErrorMode,
    THREAD_ERROR_MODE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, CreateDesktopW, HDESK};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    OpenProcessToken, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, STARTUPINFOW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use zeroize::{Zeroize as _, Zeroizing};

use crate::keychain;
use crate::run::Finished;
use windows_sys::Win32::Security::Cryptography::{
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW,
};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_WHOLECHAIN,
    WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows_sys::core::BOOL;

/// Numbers each desktop this process makes, so that no two runs share one: a desktop goes away
/// when its last handle closes, and a run opening one by the same name at that moment is refused.
static DESKTOPS: AtomicU64 = AtomicU64::new(0);

/// [`crate::run::run`], for Windows: on a desktop of its own, inside a job that ends every process
/// in it, with only its input and output handed down to it.
///
/// # Errors
///
/// When the desktop, the job or the process cannot be made, or the output cannot be read.
pub fn run(
    program: &Path,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    directory: &Path,
    limit: Duration,
) -> io::Result<Finished> {
    silence_error_dialogues();
    let desktop = Desktop::create()?;
    let job = Job::create()?;
    let (mut reader, writer) = io::pipe()?;
    let input = File::open("NUL")?;
    let process = spawn(
        program,
        args,
        environment_block(environment).as_deref(),
        directory,
        &desktop,
        &job,
        Streams::Handles([input.as_raw_handle(), writer.as_raw_handle()]),
    )?;
    // The child has its own copies now. Holding ours would keep the output open after it ends.
    drop(writer);
    drop(input);
    let reading = thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).map(|_| output)
    });
    let code = process.wait(limit, &job)?;
    // Whatever the program started and left running ends with the job, which is also what lets
    // the reading finish: a straggler would hold the output open.
    drop(job);
    let output = reading
        .join()
        .unwrap_or_else(|panic| panic::resume_unwind(panic))?;
    drop(desktop);
    Ok(Finished { code, output })
}

/// Keeps Windows from showing a dialogue when a program crashes or cannot find a disk or file.
///
/// The setting is per process and inherited, so set here it reaches steamcmd and everything
/// steamcmd starts: those failures are then only returned as errors.
pub fn silence_error_dialogues() {
    // SAFETY: changes one process-wide flag and returns the previous one; no memory is involved.
    let _: u32 = unsafe { SetErrorMode(NO_DIALOGUES) };
}

/// No dialogue for a crash, for a disk that is not there, or for a file that cannot be opened.
const NO_DIALOGUES: THREAD_ERROR_MODE =
    SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX;

/// A job that ends every process in it when steamship lets go of it, however steamship ends, and
/// ends a process that crashes rather than leaving it at Windows' crash handling.
const JOB_LIMITS: JOB_OBJECT_LIMIT =
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;

/// How a program is started: suspended until it is in the job, so that nothing it does happens
/// outside it; with an environment block of UTF-16; and with the attribute list that limits what
/// it inherits.
const CREATION: PROCESS_CREATION_FLAGS =
    CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;

/// A program given pipes has no console window either, which on a desktop nobody sees would
/// only be waste. One on a pseudo console must not be told so: it would get a console of its own
/// in place of the pseudo console, and nothing it wrote would arrive.
const PIPED: PROCESS_CREATION_FLAGS = CREATION | CREATE_NO_WINDOW;

/// The desktop a program runs on. Nobody switches to it, so nothing on it is ever seen, and
/// Windows keeps the keyboard focus from crossing from one desktop to another.
#[derive(Debug)]
struct Desktop {
    handle: HDESK,
    name: Vec<u16>,
}

impl Desktop {
    fn create() -> io::Result<Self> {
        let number = DESKTOPS.fetch_add(1, Ordering::Relaxed);
        let name = wide(OsStr::new(&format!("steamship-{}-{number}", process::id())));
        // SAFETY: `name` is NUL-terminated and outlives the call; the other arguments are null,
        // meaning the default display settings and security.
        let handle = unsafe {
            CreateDesktopW(
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                GENERIC_ALL,
                ptr::null(),
            )
        };
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self { handle, name })
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateDesktopW and is closed once, here.
        let _: i32 = unsafe { CloseDesktop(self.handle) };
    }
}

#[derive(Debug)]
struct Job(OwnedHandle);

impl Job {
    fn create() -> io::Result<Self> {
        // SAFETY: both arguments may be null: default security, no name.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `job` is a new handle that nothing else owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(job) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_LIMITS;
        // SAFETY: `limits` is initialised and its size is the one passed.
        let set = unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of_u32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
            )
        };
        if set == 0_i32 {
            Err(io::Error::last_os_error())
        } else {
            Ok(job)
        }
    }

    fn stop_all(&self) {
        // SAFETY: the job handle is open; ending its processes is what it is for.
        let _: i32 = unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }
}

#[derive(Debug)]
struct Process(OwnedHandle);

impl Process {
    /// Returns once the process has ended.
    fn ended(&self) {
        // SAFETY: the process handle is open for as long as `self`.
        let _: u32 = unsafe { WaitForSingleObject(self.0.as_raw_handle(), INFINITE) };
    }

    /// The exit code, or none when `limit` passed first and the job was ended.
    fn wait(&self, limit: Duration, job: &Job) -> io::Result<Option<i32>> {
        // INFINITE itself is reserved, so the longest finite wait is one less.
        let milliseconds = u32::try_from(limit.as_millis()).unwrap_or(INFINITE.saturating_sub(1));
        let handle = self.0.as_raw_handle();
        // SAFETY: the process handle is open for as long as `self`.
        if unsafe { WaitForSingleObject(handle, milliseconds) } == WAIT_TIMEOUT {
            job.stop_all();
            // SAFETY: as above; the job has ended the process, so this returns at once.
            let _: u32 = unsafe { WaitForSingleObject(handle, INFINITE) };
            return Ok(None);
        }
        let mut code = 0_u32;
        // SAFETY: the process has ended and `code` is a valid place for its exit code.
        if unsafe { GetExitCodeProcess(handle, &raw mut code) } == 0_i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(code.cast_signed()))
    }
}

/// A program running on a pseudo console.
///
/// The program reads and writes it as the terminal it takes it for: one that shows its prompts
/// at once and hides what is typed at a password. It runs, like any other, on a desktop nobody
/// sees and in a job that ends everything it starts.
///
/// Dropped before it is waited for, it ends the program and everything the program started.
#[derive(Debug)]
pub struct Terminal {
    process: Process,
    /// Closes the console once the program ends, which is what ends its output: the console
    /// keeps it open for as long as the console is open.
    closing: Option<JoinHandle<()>>,
    job: Job,
    _desktop: Desktop,
}

impl Terminal {
    /// Starts `program`, and answers it with what it writes, escapes and all, and what it reads
    /// as typed. The output ends once the program has.
    ///
    /// # Errors
    ///
    /// When the console, the desktop, the job or the process cannot be made.
    pub fn start(
        program: &Path,
        args: &[OsString],
        environment: &[(OsString, OsString)],
        directory: &Path,
    ) -> io::Result<(Self, PipeReader, PipeWriter)> {
        silence_error_dialogues();
        let desktop = Desktop::create()?;
        let job = Job::create()?;
        let (typed, input) = io::pipe()?;
        let (output, written) = io::pipe()?;
        // Wide enough that steamcmd's longest line is never broken across two.
        let console = PseudoConsole::create(&typed, &written, COORD { X: 240, Y: 50 })?;
        // The console holds its own copies now; ours would keep the output open after it closes.
        drop(typed);
        drop(written);
        let process = spawn(
            program,
            args,
            environment_block(environment).as_deref(),
            directory,
            &desktop,
            &job,
            Streams::Console(console.0),
        )?;
        let watched = Process(process.0.try_clone()?);
        let closing = thread::spawn(move || {
            watched.ended();
            drop(console);
        });
        let terminal = Self {
            process,
            closing: Some(closing),
            job,
            _desktop: desktop,
        };
        Ok((terminal, output, input))
    }

    /// Waits for the program to end and says its exit code.
    ///
    /// # Errors
    ///
    /// When the exit code cannot be read.
    pub fn wait(mut self) -> io::Result<Option<i32>> {
        if let Some(closing) = self.closing.take() {
            closing
                .join()
                .unwrap_or_else(|panic| panic::resume_unwind(panic));
        }
        self.process.wait(Duration::MAX, &self.job)
    }
}

#[derive(Debug)]
struct PseudoConsole(HPCON);

impl PseudoConsole {
    fn create(input: &PipeReader, output: &PipeWriter, size: COORD) -> io::Result<Self> {
        let mut console: HPCON = 0;
        // SAFETY: both pipe ends are open for the length of the call, and the console takes its
        // own copies of them.
        let result = unsafe {
            CreatePseudoConsole(
                size,
                input.as_raw_handle(),
                output.as_raw_handle(),
                0,
                &raw mut console,
            )
        };
        if result < 0_i32 {
            Err(io::Error::from_raw_os_error(result))
        } else {
            Ok(Self(console))
        }
    }
}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: the console came from CreatePseudoConsole and is closed once, here.
        unsafe {
            ClosePseudoConsole(self.0);
        }
    }
}

/// The keyboard, one character at a time as it is typed and without the console showing it.
///
/// The console's own echo, line editing and handling of Ctrl+C are off until this is dropped,
/// which puts them back.
#[derive(Debug)]
pub struct Keys {
    input: HANDLE,
    mode: CONSOLE_MODE,
}

impl Keys {
    /// The keyboard, or none when input does not come from a console.
    ///
    /// # Errors
    ///
    /// When the console will not change modes.
    pub fn open() -> io::Result<Option<Self>> {
        // SAFETY: asks for this process's own standard input; nothing is freed.
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        // SAFETY: `mode` is a valid place for the answer; a handle that is no console fails.
        if unsafe { GetConsoleMode(input, &raw mut mode) } == 0_i32 {
            return Ok(None);
        }
        let raw = mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
        // SAFETY: as above, with a mode made from the console's own.
        if unsafe { SetConsoleMode(input, raw) } == 0_i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self { input, mode }))
    }

    /// The next character typed, or none when input has ended.
    ///
    /// # Errors
    ///
    /// When the console cannot be read.
    pub fn read_key(&mut self) -> io::Result<Option<char>> {
        let mut units = Vec::with_capacity(2);
        loop {
            let mut unit = 0_u16;
            let mut read = 0_u32;
            // SAFETY: `unit` has room for the one unit asked for, and `read` for the count.
            let done = unsafe {
                ReadConsoleW(
                    self.input,
                    (&raw mut unit).cast(),
                    1,
                    &raw mut read,
                    ptr::null(),
                )
            };
            if done == 0_i32 {
                return Err(io::Error::last_os_error());
            }
            if read == 0 {
                return Ok(None);
            }
            units.push(unit);
            // A high surrogate is half a character; the other half is the next unit.
            if !(0xd800..=0xdbff).contains(&unit) {
                let decoded = char::decode_utf16(units.iter().copied()).next();
                units.fill(0);
                return Ok(Some(
                    decoded
                        .and_then(Result::ok)
                        .unwrap_or(char::REPLACEMENT_CHARACTER),
                ));
            }
        }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        // SAFETY: puts back the mode the console had when this was opened.
        let _: BOOL = unsafe { SetConsoleMode(self.input, self.mode) };
    }
}

/// Where a program's input and output go.
#[derive(Clone, Copy)]
enum Streams {
    /// These handles, handed down: its input first, then the one its output and errors both go
    /// to.
    Handles([RawHandle; 2]),
    /// A pseudo console, which it reads and writes as a terminal.
    Console(HPCON),
}

/// Starts `program` suspended, on `desktop`, in `job`, with `streams` and nothing else handed
/// down, and only then lets it run, so that it never does anything outside the job.
fn spawn(
    program: &Path,
    args: &[OsString],
    environment: Option<&[u16]>,
    directory: &Path,
    desktop: &Desktop,
    job: &Job,
    streams: Streams,
) -> io::Result<Process> {
    let (mut list, [input, output], inherit, creation) = match streams {
        Streams::Handles(handles) => {
            for handle in handles {
                // SAFETY: both handles are open for the length of this call.
                if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
                    == 0_i32
                {
                    return Err(io::Error::last_os_error());
                }
            }
            (
                AttributeList::handing_down(&handles)?,
                handles,
                1_i32,
                PIPED,
            )
        }
        // No standard handles at all: a program that inherits none but is told to use them
        // talks to the pseudo console, where one left to default could take steamship's.
        Streams::Console(console) => (
            AttributeList::on_console(console)?,
            [INVALID_HANDLE_VALUE, INVALID_HANDLE_VALUE],
            0_i32,
            CREATION,
        ),
    };
    // STARTUPINFOW asks for a writable name, though CreateProcessW only reads it.
    let mut desktop_name = desktop.name.clone();
    let info = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: size_of_u32::<STARTUPINFOEXW>(),
            lpDesktop: desktop_name.as_mut_ptr(),
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: input,
            hStdOutput: output,
            hStdError: output,
            ..STARTUPINFOW::default()
        },
        lpAttributeList: list.pointer(),
    };
    let application = wide(program.as_os_str());
    let mut line = command_line(program.as_os_str(), args);
    let directory = wide(directory.as_os_str());
    let mut started = PROCESS_INFORMATION::default();
    // SAFETY: every buffer is NUL-terminated and outlives the call, the environment block ends
    // in the two NULs Windows looks for, `line` is writable as the call requires, `info`
    // describes a live attribute list, and `started` receives the handles. Handed-down handles
    // are limited by the attribute list to exactly those; with a console none are inherited.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            inherit,
            creation,
            environment.map_or(ptr::null(), |block| block.as_ptr().cast()),
            directory.as_ptr(),
            (&raw const info).cast(),
            &raw mut started,
        )
    };
    if created == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateProcessW succeeded, so this handle is new and owned by nobody else.
    let process = Process(unsafe { OwnedHandle::from_raw_handle(started.hProcess) });
    // SAFETY: as for the process handle.
    let thread = unsafe { OwnedHandle::from_raw_handle(started.hThread) };
    // SAFETY: both handles are open.
    if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process.0.as_raw_handle()) }
        == 0_i32
    {
        let error = io::Error::last_os_error();
        // SAFETY: the process is suspended and was never let run; ending it is all that is left.
        let _: i32 = unsafe { TerminateProcess(process.0.as_raw_handle(), 1) };
        return Err(error);
    }
    // SAFETY: the thread handle is open; it was created suspended once, so one resume starts it.
    match unsafe { ResumeThread(thread.as_raw_handle()) } {
        // The count it was suspended by before this resume: once, since it was made that way.
        1 => Ok(process),
        u32::MAX => Err(io::Error::last_os_error()),
        // It was running already, so it may have done something before it was in the job.
        count => {
            // SAFETY: as above; the program is ended rather than left running outside the rules.
            let _: i32 = unsafe { TerminateProcess(process.0.as_raw_handle(), 1) };
            Err(io::Error::other(format!(
                "the program was not held suspended until it was in its job (count {count})"
            )))
        }
    }
}

/// A process-thread attribute list with one attribute: the only handles a new process inherits,
/// or the pseudo console it runs on.
struct AttributeList {
    /// Pointer-aligned storage for the list, which Windows sizes.
    storage: Vec<usize>,
    /// The handles the list points at, when it hands any down, kept for as long as the list.
    _handles: Option<Box<[RawHandle; 2]>>,
}

impl AttributeList {
    fn handing_down(handles: &[RawHandle; 2]) -> io::Result<Self> {
        let kept = Box::new(*handles);
        let value = (&raw const *kept).cast();
        Self::with(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            value,
            size_of::<[RawHandle; 2]>(),
            Some(kept),
        )
    }

    /// The pseudo console attribute's value is the console handle itself, not a pointer to it.
    fn on_console(console: HPCON) -> io::Result<Self> {
        let value = ptr::without_provenance(console.cast_unsigned());
        Self::with(
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            value,
            size_of::<HPCON>(),
            None,
        )
    }

    fn with(
        attribute: u32,
        value: *const c_void,
        size_of_value: usize,
        handles: Option<Box<[RawHandle; 2]>>,
    ) -> io::Result<Self> {
        let mut size = 0_usize;
        // SAFETY: a null list with a size to fill is how the needed size is asked for; the call
        // then fails with ERROR_INSUFFICIENT_BUFFER, which is expected.
        let _: i32 =
            unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        let mut list = Self {
            storage: vec![0; size.div_ceil(size_of::<usize>())],
            _handles: handles,
        };
        // SAFETY: `storage` holds at least `size` bytes, aligned for the list.
        if unsafe { InitializeProcThreadAttributeList(list.pointer(), 1, 0, &raw mut size) }
            == 0_i32
        {
            return Err(io::Error::last_os_error());
        }
        let attribute = usize::try_from(attribute).map_err(io::Error::other)?;
        // SAFETY: the list was initialised for one attribute; a value that points anywhere
        // points into `list.handles`, which lives as long as the list does.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.pointer(),
                0,
                attribute,
                value,
                size_of_value,
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0_i32 {
            Err(io::Error::last_os_error())
        } else {
            Ok(list)
        }
    }

    const fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialised before any AttributeList that could be dropped existed
        // with it, and is deleted once, here.
        unsafe {
            DeleteProcThreadAttributeList(self.pointer());
        }
    }
}

/// `program` and `args` as one command line, each quoted the way Windows' own argument parser
/// reads them back.
fn command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut line = Vec::new();
    quote(program, &mut line);
    for arg in args {
        line.push(u16::from(b' '));
        quote(arg, &mut line);
    }
    line.push(0);
    line
}

/// Appends `arg` to `line`, quoted when it needs to be. Inside quotes, a run of backslashes is
/// doubled when a quote follows it, and so is one at the end, since the closing quote follows.
fn quote(arg: &OsStr, line: &mut Vec<u16>) {
    let units: Vec<u16> = arg.encode_wide().collect();
    let [backslash, quote_mark, space, tab] = b"\\\" \t".map(u16::from);
    if !units.is_empty()
        && !units
            .iter()
            .any(|unit| [quote_mark, space, tab].contains(unit))
    {
        line.extend(units);
        return;
    }
    line.push(quote_mark);
    let mut backslashes = 0_usize;
    for unit in units {
        if unit == backslash {
            backslashes = backslashes.saturating_add(1);
            continue;
        }
        let doubled = if unit == quote_mark {
            backslashes.saturating_mul(2).saturating_add(1)
        } else {
            backslashes
        };
        line.extend(iter::repeat_n(backslash, doubled));
        line.push(unit);
        backslashes = 0;
    }
    line.extend(iter::repeat_n(backslash, backslashes.saturating_mul(2)));
    line.push(quote_mark);
}

/// steamship's own environment with `changes` made to it, as the block `CreateProcessW` takes,
/// or none when there are no changes and the program inherits steamship's as it stands.
fn environment_block(changes: &[(OsString, OsString)]) -> Option<Vec<u16>> {
    if changes.is_empty() {
        return None;
    }
    let mut variables: Vec<(OsString, OsString)> = env::vars_os()
        .filter(|(name, _)| {
            !changes
                .iter()
                .any(|(changed, _)| changed.eq_ignore_ascii_case(name))
        })
        .chain(changes.iter().cloned())
        .collect();
    // Windows keeps the block in order of name, ignoring case, and names are compared that way.
    variables.sort_by_key(|(name, _)| name.to_ascii_uppercase());
    let mut block = Vec::new();
    for (name, value) in variables {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Some(block)
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// Whether Windows answered that there is no such credential.
fn not_found(error: &io::Error) -> bool {
    error.raw_os_error() == i32::try_from(ERROR_NOT_FOUND).ok()
}

/// The generic credential the Web API key for the steamship home `account` is kept as.
fn key_target(account: &str) -> Vec<u16> {
    wide(OsStr::new(&format!("steamship:web-api-key:{account}")))
}

fn store_failed(error: &io::Error) -> keychain::Error {
    keychain::Error::Failed(error.to_string())
}

/// Whether the Credential Manager keeps a secret for the steamship home `account`.
///
/// # Errors
///
/// When the Credential Manager cannot be read.
pub fn has_secret(account: &str) -> Result<bool, keychain::Error> {
    Ok(kept_secret(account)?.is_some())
}

/// The secret the Credential Manager keeps for `account`, if it keeps one.
///
/// # Errors
///
/// When the Credential Manager cannot be read.
pub fn kept_secret(account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, keychain::Error> {
    let target = key_target(account);
    let mut found: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call; on success Windows points `found`
    // at a credential it allocated.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) } == 0_i32 {
        let error = io::Error::last_os_error();
        return if not_found(&error) {
            Ok(None)
        } else {
            Err(store_failed(&error))
        };
    }
    let found = Credential(found);
    // SAFETY: CredReadW succeeded, so this is one credential, left alone until it is freed.
    let credential = unsafe { &*found.0 };
    let size = usize::try_from(credential.CredentialBlobSize).unwrap_or_default();
    let secret = if credential.CredentialBlob.is_null() {
        Zeroizing::new(Vec::new())
    } else {
        // SAFETY: the blob is `CredentialBlobSize` bytes, which nothing else is using.
        let blob = unsafe { slice::from_raw_parts_mut(credential.CredentialBlob, size) };
        let secret = Zeroizing::new(blob.to_vec());
        // Windows frees the blob without wiping it.
        blob.zeroize();
        secret
    };
    Ok(Some(secret))
}

/// A credential `CredReadW` allocated, freed once it is dropped.
#[derive(Debug)]
struct Credential(*mut CREDENTIALW);

impl Drop for Credential {
    fn drop(&mut self) {
        // SAFETY: the credential came from CredReadW and is freed once, here.
        unsafe {
            CredFree(self.0.cast_const().cast());
        }
    }
}

/// Keeps `secret` for `account`, in place of any kept before, for this user on this machine only.
///
/// # Errors
///
/// When the Credential Manager refuses it.
pub fn keep_secret(account: &str, secret: &[u8]) -> Result<(), keychain::Error> {
    let mut target = key_target(account);
    let mut label = wide(OsStr::new(keychain::LABEL));
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: u32::try_from(secret.len())
            .ok()
            .ok_or_else(|| keychain::Error::Failed("the key is too long".to_owned()))?,
        CredentialBlob: secret.as_ptr().cast_mut(),
        // Kept for this user on this machine, never carried along with a roaming profile.
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: label.as_mut_ptr(),
        ..CREDENTIALW::default()
    };
    // SAFETY: every pointer in `credential` is valid for the call, which copies what it keeps
    // and writes through none of them.
    if unsafe { CredWriteW(&raw const credential, 0) } == 0_i32 {
        Err(store_failed(&io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

/// Removes the secret kept for `account`, and says whether there was one.
///
/// # Errors
///
/// When the Credential Manager refuses.
pub fn forget_secret(account: &str) -> Result<bool, keychain::Error> {
    let target = key_target(account);
    // SAFETY: `target` is NUL-terminated and outlives the call.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0_i32 {
        let error = io::Error::last_os_error();
        return if not_found(&error) {
            Ok(false)
        } else {
            Err(store_failed(&error))
        };
    }
    Ok(true)
}

/// Leaves `folder` to the current user alone, and says whether it had to change anything.
///
/// One entry grants them everything, inherited by all the folder holds, and nothing is inherited
/// from above, where a shared parent could let others in. Windows carries the entry down to what
/// the folder already holds, which walks all of it, so that is only done when the folder is not
/// already so.
///
/// # Errors
///
/// When the folder's permissions cannot be read or set.
pub fn restrict(folder: &Path) -> io::Result<bool> {
    let user = User::current()?;
    let name = wide(folder.as_os_str());
    if Security::of(&name)?.is_only(&user) {
        return Ok(false);
    }
    // SAFETY: the SID lives in `user`, which outlives the call.
    let sid_length = unsafe { GetLengthSid(user.sid()) };
    // An entry's size counts its SID in place of the SidStart field that marks where it begins.
    let size = size_of_u32::<ACL>()
        .saturating_add(size_of_u32::<ACCESS_ALLOWED_ACE>())
        .saturating_sub(size_of_u32::<u32>())
        .saturating_add(sid_length);
    let mut storage = vec![0_u32; usize::try_from(size.div_ceil(4)).map_err(io::Error::other)?];
    let acl: *mut ACL = storage.as_mut_ptr().cast();
    // SAFETY: `storage` holds at least `size` bytes, aligned as an ACL must be.
    if unsafe { InitializeAcl(acl, size, ACL_REVISION) } == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the list was sized for exactly this one entry, and the SID is copied into it.
    if unsafe {
        AddAccessAllowedAceEx(
            acl,
            ACL_REVISION,
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            FILE_ALL_ACCESS,
            user.sid(),
        )
    } == 0_i32
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `name` is NUL-terminated and `acl` is a complete list; the owner, the group and the
    // audit list are left as they are.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null(),
        )
    };
    win32(status).map(|()| true)
}

fn win32(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status.cast_signed()))
    }
}

/// The user this process runs as.
struct User {
    /// A `TOKEN_USER` and the SID it points into, as Windows wrote them, pointer-aligned.
    storage: Vec<usize>,
}

impl User {
    fn current() -> io::Result<Self> {
        // SAFETY: returns a pseudo handle for this process, which needs no closing.
        let process = unsafe { GetCurrentProcess() };
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: `token` is a valid place for the new handle.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0_i32 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle is new and owned by nobody else.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut size = 0_u32;
        // SAFETY: no buffer with a size of 0 is how the needed size is asked for; the call then
        // fails with ERROR_INSUFFICIENT_BUFFER, which is expected.
        let _: BOOL = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &raw mut size,
            )
        };
        let length = usize::try_from(size).map_err(io::Error::other)?;
        let mut storage = vec![0_usize; length.div_ceil(size_of::<usize>())];
        // SAFETY: `storage` holds at least `size` bytes, aligned for a TOKEN_USER.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                size,
                &raw mut size,
            )
        } == 0_i32
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { storage })
    }

    const fn sid(&self) -> PSID {
        let user: *const TOKEN_USER = self.storage.as_ptr().cast();
        // SAFETY: `storage` holds the TOKEN_USER that GetTokenInformation wrote.
        unsafe { (*user).User.Sid }
    }
}

/// A file or folder's permissions as Windows reports them.
struct Security {
    /// Freed with `LocalFree`; `acl` points into it.
    descriptor: PSECURITY_DESCRIPTOR,
    /// None for a null list, which Windows reads as everyone being allowed everything.
    acl: Option<NonNull<ACL>>,
}

impl Security {
    fn of(name: &[u16]) -> io::Result<Self> {
        let mut acl = MaybeUninit::<*mut ACL>::uninit();
        let mut descriptor = MaybeUninit::<PSECURITY_DESCRIPTOR>::uninit();
        // SAFETY: `name` is NUL-terminated, and both outputs are valid places for pointers.
        let status = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                acl.as_mut_ptr(),
                ptr::null_mut(),
                descriptor.as_mut_ptr(),
            )
        };
        win32(status)?;
        // SAFETY: the call succeeded, and on success it writes both.
        let descriptor = unsafe { descriptor.assume_init() };
        // SAFETY: as above.
        let acl = unsafe { acl.assume_init() };
        Ok(Self {
            descriptor,
            acl: NonNull::new(acl),
        })
    }

    fn is_protected(&self) -> bool {
        let mut control = 0_u16;
        let mut revision = 0_u32;
        // SAFETY: the descriptor is the one Windows returned, alive until `self` is dropped.
        let read = unsafe {
            GetSecurityDescriptorControl(self.descriptor, &raw mut control, &raw mut revision)
        };
        read != 0_i32 && control & SE_DACL_PROTECTED != 0
    }

    /// Each entry's type, flags and access, and whether it is for `user`.
    fn entries(&self, user: &User) -> Vec<(u32, u32, u32, bool)> {
        let Some(acl) = self.acl else {
            return Vec::new();
        };
        // SAFETY: the list points into the descriptor, alive as long as `self`.
        let count = unsafe { acl.as_ref() }.AceCount;
        (0..u32::from(count))
            .filter_map(|index| {
                let mut found = MaybeUninit::<*mut c_void>::uninit();
                // SAFETY: `index` is below the list's count, and `found` is a place for a pointer.
                if unsafe { GetAce(acl.as_ptr(), index, found.as_mut_ptr()) } == 0_i32 {
                    return None;
                }
                // SAFETY: the call succeeded, and on success it writes the pointer.
                let entry =
                    NonNull::new(unsafe { found.assume_init() })?.cast::<ACCESS_ALLOWED_ACE>();
                // SAFETY: every kind of entry starts with its header and then its access mask,
                // and SidStart is read only for the kind that has one.
                let ACCESS_ALLOWED_ACE {
                    Header: header,
                    Mask: mask,
                    ..
                } = unsafe { entry.read() };
                let kind = u32::from(header.AceType);
                let mine = kind == ACCESS_ALLOWED_ACE_TYPE && {
                    // SAFETY: an access-allowed entry holds its SID from SidStart on.
                    let sid = unsafe { &raw const (*entry.as_ptr()).SidStart };
                    // SAFETY: both SIDs are valid for the length of the call.
                    let equal = unsafe { EqualSid(sid.cast_mut().cast(), user.sid()) };
                    equal != 0_i32
                };
                Some((kind, u32::from(header.AceFlags), mask, mine))
            })
            .collect()
    }

    fn is_only(&self, user: &User) -> bool {
        only(self.is_protected(), &self.entries(user))
    }
}

/// Whether permissions that are `protected` from the parent's, with `entries`, leave a folder to
/// its user alone: one entry, allowing that user everything, passed down to all it holds.
fn only(protected: bool, entries: &[(u32, u32, u32, bool)]) -> bool {
    protected
        && entries
            == [(
                ACCESS_ALLOWED_ACE_TYPE,
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                true,
            )]
}

impl Drop for Security {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from GetNamedSecurityInfoW and is freed once, here.
        let _: HLOCAL = unsafe { LocalFree(self.descriptor) };
    }
}

/// Windows has no executable bit, so there is nothing to mark.
///
/// # Errors
///
/// Never; the signature is the one every system shares.
pub const fn mark_if_program(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Only macOS packages hold links, and they are only ever unpacked on macOS, so a link in a
/// package for Windows is refused rather than guessed at.
///
/// # Errors
///
/// Always.
pub fn make_link(_: &str, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a package for this system holds a link",
    ))
}

/// Who signed steamcmd's own programs and libraries.
pub const VALVE: &str = "Valve Corp.";

/// Valve ships a web UI for its content server alongside steamcmd, bundling Microsoft's runtime
/// (signed by Microsoft) and a few third-party libraries (not signed at all). steamship never
/// runs it; its files are held to their pinned SHA-256 like every other, but not to Valve's
/// signature, which they do not carry.
const NOT_VALVES: &str = "siteserverui/";

/// Every one of `files` under `root` that is steamcmd's own program or library and does not carry
/// a valid signature from `signer`, [`VALVE`] for steamcmd, each said as a change to the install.
pub fn unsigned<'file, Files>(root: &Path, files: Files, signer: &str) -> Vec<String>
where
    Files: IntoIterator<Item = &'file String>,
{
    files
        .into_iter()
        .filter(|relative| !relative.starts_with(NOT_VALVES))
        .filter(|relative| {
            Path::new(relative.as_str())
                .extension()
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("dll")
                })
        })
        .filter_map(|relative| match signature(&root.join(relative)) {
            Signature::Signed(name) if name == signer => None,
            Signature::Signed(name) => {
                Some(format!("{relative} is signed by {name}, not {signer}"))
            }
            Signature::Refused(status) => Some(format!(
                "{relative} has no valid signature (Windows says {status:#010x})"
            )),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    /// Windows verified the signature, and this is the signer's name.
    Signed(String),
    /// Windows did not accept the file's signature, or found none; this is its answer.
    Refused(u32),
}

/// Sets how a signature is checked: with no window, ever, and with the revocation of every
/// certificate in the chain checked but its root's. Neither shows in what Windows answers for a
/// file that is signed or not, only with a certificate that has been revoked, or with someone
/// there to answer a dialogue.
const fn quietly_to_the_root(data: &mut WINTRUST_DATA) {
    data.dwUIChoice = WTD_UI_NONE;
    data.fdwRevocationChecks = WTD_REVOKE_WHOLECHAIN;
    data.dwProvFlags = WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT;
}

/// What Windows makes of `path`'s Authenticode signature, checked as [`quietly_to_the_root`]
/// sets out.
#[must_use]
pub fn signature(path: &Path) -> Signature {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of_u32::<WINTRUST_FILE_INFO>(),
        pcwszFilePath: wide.as_ptr(),
        ..WINTRUST_FILE_INFO::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of_u32::<WINTRUST_DATA>(),
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &raw mut file,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..WINTRUST_DATA::default()
    };
    quietly_to_the_root(&mut data);
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and the `file` it points to are fully initialised and outlive the call, and
    // `wide` is a NUL-terminated path that outlives both. An invalid window handle and
    // WTD_UI_NONE together tell WinTrust there is nobody to show anything to.
    let status = unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        )
    };
    let answer = if status == 0_i32 {
        signer_name(data.hWVTStateData).map_or(Signature::Refused(0), Signature::Signed)
    } else {
        Signature::Refused(status.cast_unsigned())
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: the same structures, now asking WinTrust to free the state the call above left in
    // `data.hWVTStateData`, which is what that call's documentation requires.
    let _: i32 = unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        )
    };
    answer
}

/// The simple display name, in practice the common name, of the certificate that signed.
fn signer_name(state: HANDLE) -> Option<String> {
    // SAFETY: `state` is the state a successful verification left open; it stays valid until
    // the closing call, which comes after this function returns.
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return None;
    }
    // SAFETY: `provider` is non-null and belongs to the same open state.
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, 0, 0) };
    // SAFETY: a non-null signer points at a structure WinTrust owns for as long as the state.
    let chain = unsafe { signer.as_ref() }?.pasCertChain;
    // SAFETY: the first element of a non-null chain is the signing certificate's entry.
    let certificate = unsafe { chain.as_ref() }?.pCert;
    if certificate.is_null() {
        return None;
    }
    let mut name = vec![0_u16; 256];
    let capacity = u32::try_from(name.len()).ok()?;
    // SAFETY: `certificate` is valid while the state is open, and `name` holds `capacity` units.
    let written = unsafe {
        CertGetNameStringW(
            certificate,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            ptr::null(),
            name.as_mut_ptr(),
            capacity,
        )
    };
    // The count includes the terminating NUL, so one means an empty name.
    let length = usize::try_from(written).ok()?.checked_sub(1)?;
    name.truncate(length);
    String::from_utf16(&name)
        .ok()
        .filter(|name| !name.is_empty())
}

/// The size of a Win32 structure, as the `cbStruct` fields want it.
const fn size_of_u32<Structure>() -> u32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "a Win32 structure is a few hundred bytes, nowhere near 4 GiB"
    )]
    let size = size_of::<Structure>() as u32;
    size
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::env;
    use std::fs;
    use std::os::windows::ffi::OsStringExt as _;
    use std::path::PathBuf;
    use std::slice;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::Globalization::lstrlenW;
    use windows_sys::Win32::Security::INHERITED_ACE;
    use windows_sys::Win32::System::StationsAndDesktops::EnumDesktopWindows;
    use windows_sys::Win32::System::StationsAndDesktops::{DESKTOP_READOBJECTS, OpenDesktopW};
    use windows_sys::Win32::System::Threading::GetProcessId;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};

    /// A library Windows installs with a signature of its own inside it, rather than in a
    /// catalogue, which is the kind steamcmd's files carry.
    fn signed_by_microsoft() -> PathBuf {
        Path::new(&env::var_os("SystemRoot").unwrap()).join("System32/vcruntime140.dll")
    }

    /// What Windows answers for a file that carries no signature.
    const TRUST_E_NOSIGNATURE: u32 = 0x800b_0100;

    #[test]
    fn names_the_signer_of_a_signed_file() {
        let found = signature(&signed_by_microsoft());
        assert!(
            matches!(&found, Signature::Signed(name) if name.starts_with("Microsoft")),
            "{found:?}"
        );
    }

    #[test]
    fn refuses_a_file_with_no_signature() {
        let unsigned = env::current_exe().unwrap();
        assert_eq!(
            signature(&unsigned),
            Signature::Refused(TRUST_E_NOSIGNATURE)
        );
    }

    #[test]
    fn holds_steamcmds_own_files_to_valves_signature_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let copy = |relative: &str, from: &Path| {
            let to = root.path().join(relative);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            let _: u64 = fs::copy(from, to).unwrap();
        };
        copy("steamcmd.exe", &signed_by_microsoft());
        copy("bin/unsigned.DLL", &env::current_exe().unwrap());
        copy("siteserverui/win32/node.dll", &env::current_exe().unwrap());
        copy("public/strings.txt", &env::current_exe().unwrap());
        let files: Vec<String> = [
            "steamcmd.exe",
            "bin/unsigned.DLL",
            "siteserverui/win32/node.dll",
            "public/strings.txt",
        ]
        .map(str::to_owned)
        .into();
        let found = unsigned(root.path(), &files, VALVE);
        let [microsoft, bare] = found.as_slice() else {
            panic!("expected two findings, got {found:?}");
        };
        assert!(
            microsoft.starts_with("steamcmd.exe is signed by Microsoft"),
            "{microsoft}"
        );
        assert!(microsoft.ends_with(", not Valve Corp."), "{microsoft}");
        let refused = "bin/unsigned.DLL has no valid signature (Windows says 0x800b0100)";
        assert_eq!(bare, refused);
        let Signature::Signed(signer) = signature(&signed_by_microsoft()) else {
            panic!("the Microsoft library is not signed");
        };
        assert_eq!(unsigned(root.path(), &files, &signer), [refused]);
    }

    /// The arguments Windows' own parser reads out of `line`, a NUL-terminated command line.
    fn parsed(line: &[u16]) -> Vec<OsString> {
        let mut count = 0_i32;
        // SAFETY: `line` is NUL-terminated and outlives the call.
        let block = unsafe { CommandLineToArgvW(line.as_ptr(), &raw mut count) };
        assert!(!block.is_null(), "{}", io::Error::last_os_error());
        // SAFETY: Windows returned `count` pointers, which live until the block is freed below.
        let pointers = unsafe { slice::from_raw_parts(block, usize::try_from(count).unwrap()) };
        let read = pointers
            .iter()
            .map(|&arg| {
                // SAFETY: each argument is NUL-terminated.
                let length = usize::try_from(unsafe { lstrlenW(arg) }).unwrap();
                // SAFETY: the argument holds `length` units before its NUL.
                OsString::from_wide(unsafe { slice::from_raw_parts(arg, length) })
            })
            .collect();
        // SAFETY: CommandLineToArgvW allocated the block as one, freed once, here, after the
        // last read of it.
        let _: HLOCAL = unsafe { LocalFree(block.cast()) };
        read
    }

    struct Search {
        process: u32,
        found: usize,
    }

    /// Counts `search`'s process's visible windows, one window per call.
    unsafe extern "system" fn count(window: HWND, pointer: LPARAM) -> BOOL {
        let place: *mut Search = ptr::with_exposed_provenance_mut(pointer.cast_unsigned());
        // SAFETY: `search` is the one `visible_windows` exposed, which outlives the enumeration
        // and is reached through nothing else meanwhile.
        let search = unsafe { &mut *place };
        let mut owner = 0_u32;
        // SAFETY: `owner` is a valid place for the id; a window that has gone just leaves it 0.
        let _: u32 = unsafe { GetWindowThreadProcessId(window, &raw mut owner) };
        // SAFETY: any handle may be asked about; one that has gone is not visible.
        if owner == search.process && unsafe { IsWindowVisible(window) } != 0_i32 {
            search.found = search.found.saturating_add(1);
        }
        1_i32
    }

    /// How many visible windows the process `process` has on `desktop`, or on the caller's own
    /// desktop when `desktop` is null.
    fn visible_windows(desktop: HDESK, process: u32) -> usize {
        let mut search = Search { process, found: 0 };
        let pointer = (&raw mut search).expose_provenance().cast_signed();
        // SAFETY: `count` matches the callback's signature and `pointer` leads to `search`,
        // which outlives the call.
        let _: BOOL = unsafe { EnumDesktopWindows(desktop, Some(count), pointer) };
        search.found
    }

    #[test]
    fn a_program_shows_its_window_only_on_a_desktop_nobody_sees() {
        let desktop = Desktop::create().unwrap();
        let job = Job::create().unwrap();
        let input = File::open("NUL").unwrap();
        let output = File::options().write(true).open("NUL").unwrap();
        let winver = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\winver.exe");
        let process = spawn(
            &winver,
            &[],
            None,
            &env::temp_dir(),
            &desktop,
            &job,
            Streams::Handles([input.as_raw_handle(), output.as_raw_handle()]),
        )
        .unwrap();
        // SAFETY: the process handle is open for as long as `process`.
        let id = unsafe { GetProcessId(process.0.as_raw_handle()) };
        // Finding the window where it was put is what shows that the search would see it.
        let shown = (0..400_u32).any(|_| {
            thread::sleep(Duration::from_millis(50));
            visible_windows(desktop.handle, id) > 0
        });
        assert!(shown, "winver never showed its window");
        assert_eq!(visible_windows(ptr::null_mut(), id), 0);
    }

    /// Whether a desktop called `name` exists.
    fn desktop_exists(name: &[u16]) -> bool {
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let handle = unsafe { OpenDesktopW(name.as_ptr(), 0, 0, DESKTOP_READOBJECTS) };
        if handle.is_null() {
            return false;
        }
        // SAFETY: the handle was just opened, and is closed once, here.
        let _: BOOL = unsafe { CloseDesktop(handle) };
        true
    }

    #[test]
    fn a_desktop_goes_away_once_steamship_lets_go_of_it() {
        let desktop = Desktop::create().unwrap();
        let name = desktop.name.clone();
        assert!(desktop_exists(&name));
        drop(desktop);
        assert!(!desktop_exists(&name));
    }

    #[test]
    fn a_restricted_folder_is_its_users_alone_down_to_what_it_already_held() {
        let folder = tempfile::tempdir().unwrap();
        let held = folder.path().join("config.vdf");
        fs::write(&held, "token").unwrap();
        let user = User::current().unwrap();
        let name = wide(folder.path().as_os_str());
        assert!(!Security::of(&name).unwrap().is_only(&user));
        assert!(restrict(folder.path()).unwrap());
        assert!(Security::of(&name).unwrap().is_only(&user));
        assert_eq!(
            Security::of(&wide(held.as_os_str()))
                .unwrap()
                .entries(&user),
            [(
                ACCESS_ALLOWED_ACE_TYPE,
                INHERITED_ACE,
                FILE_ALL_ACCESS,
                true
            )]
        );
        assert!(!restrict(folder.path()).unwrap(), "already so");
    }

    #[test]
    fn a_new_folder_takes_its_permissions_from_its_parent_and_is_not_only_its_users() {
        let folder = tempfile::tempdir().unwrap();
        let user = User::current().unwrap();
        let security = Security::of(&wide(folder.path().as_os_str())).unwrap();
        assert!(!security.is_protected());
        let entries = security.entries(&user);
        let allowed = |mine: bool| {
            entries
                .iter()
                .any(|entry| entry.0 == ACCESS_ALLOWED_ACE_TYPE && entry.3 == mine)
        };
        // The temporary folder is shared with the system and administrators, besides its user.
        assert!(allowed(true) && allowed(false), "{entries:?}");
        assert!(restrict(folder.path()).unwrap());
        assert!(
            Security::of(&wide(folder.path().as_os_str()))
                .unwrap()
                .is_protected()
        );
    }

    #[test]
    fn a_pseudo_console_windows_will_not_make_is_an_error() {
        let (typed, _input) = io::pipe().unwrap();
        let (_output, written) = io::pipe().unwrap();
        drop(PseudoConsole::create(&typed, &written, COORD { X: 0, Y: 0 }).unwrap_err());
        drop(PseudoConsole::create(&typed, &written, COORD { X: 80, Y: 25 }).unwrap());
    }

    #[test]
    fn only_a_protected_single_entry_for_the_user_is_the_users_alone() {
        let alone = [(
            ACCESS_ALLOWED_ACE_TYPE,
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            FILE_ALL_ACCESS,
            true,
        )];
        assert!(only(true, &alone));
        assert!(!only(false, &alone), "inherits whatever its parent allows");
        let someone_else = [(alone[0].0, alone[0].1, alone[0].2, false)];
        assert!(!only(true, &someone_else));
        assert!(!only(true, &[]));
    }

    #[test]
    fn a_changed_variable_replaces_steamships_whatever_its_case_and_the_block_stays_in_order() {
        assert_eq!(environment_block(&[]), None);
        let block =
            environment_block(&[(OsString::from("path"), OsString::from("changed"))]).unwrap();
        assert_eq!(block.last_chunk(), Some(&[0_u16, 0_u16]));
        let text = String::from_utf16(&block).unwrap();
        let entries: Vec<&str> = text.trim_end_matches('\0').split('\0').collect();
        let paths: Vec<&str> = entries
            .iter()
            .copied()
            .filter(|entry| entry.to_ascii_uppercase().starts_with("PATH="))
            .collect();
        assert_eq!(paths, ["path=changed"]);
        let names: Vec<String> = entries
            .iter()
            .map(|entry| {
                entry
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase()
            })
            .collect();
        assert!(names.is_sorted(), "{names:?}");
    }

    #[test]
    fn a_folder_that_is_not_there_cannot_be_restricted() {
        let folder = tempfile::tempdir().unwrap();
        let error = restrict(&folder.path().join("missing")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    /// Mostly the units that quoting is about, with anything else mixed in.
    fn unit() -> impl Strategy<Value = u16> {
        prop_oneof![
            3 => prop::sample::select(b"\\\" \ta".map(u16::from).to_vec()),
            1 => 1_u16..,
        ]
    }

    proptest! {
        #[test]
        fn windows_reads_every_argument_back_as_it_was_given(
            args in prop::collection::vec(prop::collection::vec(unit(), 0..10), 0..5),
        ) {
            let program = OsString::from(r"C:\Program Files\steamcmd\steamcmd.exe");
            let args: Vec<OsString> = args.iter().map(|units| OsString::from_wide(units)).collect();
            let line = command_line(&program, &args);
            let expected: Vec<OsString> = iter::once(program).chain(args).collect();
            prop_assert_eq!(parsed(&line), expected);
        }
    }
}
