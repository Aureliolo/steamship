//! What only Windows does. Kept in a module of its own so that everything here is compiled,
//! tested and mutation-tested on the system where it runs.
#![expect(
    unsafe_code,
    reason = "Windows' own APIs are the only way to ask Windows these questions"
)]

use std::env;
use std::ffi::{OsStr, OsString, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, PipeReader, PipeWriter};
use std::iter;
use std::mem::{self, MaybeUninit};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle};
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::{
    ERROR_NOT_FOUND, ERROR_SHARING_VIOLATION, ERROR_SUCCESS, GENERIC_ALL, GENERIC_WRITE, HANDLE,
    HANDLE_FLAG_INHERIT, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation,
    WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW, SetSecurityInfo,
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
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
    FileBasicInfo, GetFileInformationByHandleEx, WRITE_DAC,
};
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
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetProcessWindowStation, GetUserObjectInformationW, HDESK,
    UOI_NAME,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, GetProcessAffinityMask, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, SetProcessAffinityMask, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use zeroize::{Zeroize as _, Zeroizing};

use crate::keychain;
use crate::parallel;
use crate::run::{self, Finished, Startup};
use crate::terminal::{COLUMNS, ROWS};
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

/// Held while a handle of steamship's is marked to be handed down, and while the standard library
/// starts a program.
///
/// The standard library hands a program every handle the process has marked so at that moment.
/// steamship marks the output of a run for as long as it takes to start the program, and making a
/// pseudo console marks its pipes for as long as it takes to start the console. A program the
/// standard library started then would hold that output open for as long as it ran.
static HANDING_DOWN: Mutex<()> = Mutex::new(());

/// `HANDING_DOWN`, held until dropped.
pub fn handing_down() -> MutexGuard<'static, ()> {
    // The lock guards no data, so one a panic left is as good as any.
    HANDING_DOWN.lock().unwrap_or_else(PoisonError::into_inner)
}

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
    hopeless: &'static [&'static str],
    startup: Option<Startup<'_>>,
) -> io::Result<Finished> {
    silence_error_dialogues();
    let desktop = Desktop::create()?;
    let job = Job::create()?;
    let (reader, writer) = io::pipe()?;
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
    let seen = Arc::new(AtomicBool::new(false));
    let watching = Arc::clone(&seen);
    let reading = thread::spawn(move || run::watch(reader, hopeless, &watching));
    let code = run::waited(limit, &seen, startup, |pause| process.ended_within(pause))?;
    if code.is_none() {
        job.stop_all();
        process.ended();
    }
    // Whatever the program started and left running ends with the job, which is also what lets
    // the reading finish: a straggler would hold the output open.
    drop(job);
    let output = reading
        .join()
        .unwrap_or_else(|panic| panic::resume_unwind(panic))?;
    drop(desktop);
    Ok(Finished { code, output })
}

/// What told steamship to stop while a program ran: on Windows, nothing does, since Ctrl+C ends
/// steamship and the job then ends the program.
#[must_use]
pub const fn told() -> Option<&'static str> {
    None
}

/// Nothing tells steamship to stop on Windows (see [`told`]).
pub const fn end_as_told() {}

/// Keeps Windows from showing a dialogue when a program crashes or cannot find a disk or file.
///
/// The setting is per process and inherited, so set here it reaches steamcmd and everything
/// steamcmd starts: those failures are then only returned as errors.
pub fn silence_error_dialogues() {
    // SAFETY: changes one process-wide flag and returns the previous one. No memory is involved.
    let _: u32 = unsafe { SetErrorMode(NO_DIALOGUES) };
}

/// Holds steamship, and every program it starts from here on, which inherit it, to the first
/// `cores` of the processors it may run on. Gives how many it is held to, of how many.
///
/// # Errors
///
/// When Windows will not say which processors steamship may run on, or not hold it to some.
pub fn hold_to(cores: usize) -> io::Result<(usize, usize)> {
    let mut allowed: usize = 0;
    let mut system: usize = 0;
    // SAFETY: returns a pseudo handle to the process itself, which needs no closing.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: the call only writes the two masks it is given.
    let read = unsafe { GetProcessAffinityMask(process, &raw mut allowed, &raw mut system) };
    if read == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    let processors: Vec<usize> = (0..usize::BITS)
        .filter(|bit| allowed & (1 << bit) != 0)
        .filter_map(|bit| usize::try_from(bit).ok())
        .collect();
    let held = run::held(&processors, cores);
    let mask: usize = held.iter().map(|bit| 1_usize << bit).sum();
    // SAFETY: the mask holds processors the process may already run on, as Windows asks.
    if unsafe { SetProcessAffinityMask(process, mask) } == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    Ok((held.len(), processors.len()))
}

/// The seconds from 1601, where Windows counts a file's times from, to 1970.
const FROM_1601: u64 = 11_644_473_600;

/// When the file or folder at `path` was last written, made or renamed: the later of its change
/// time and its creation time.
///
/// A copy, as Python's `shutil.copy2` makes one, keeps its source's change time as well as its
/// last-written time, so a file a build copied shows only in when it was made.
///
/// # Errors
///
/// When it cannot be opened to read its times.
pub fn changed(path: &Path) -> io::Result<SystemTime> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    let size = u32::try_from(size_of::<FILE_BASIC_INFO>()).unwrap_or(u32::MAX);
    // SAFETY: the handle is open for the call's length, and `info` is `size` bytes it writes.
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded, so it wrote the whole of `info`.
    let times = unsafe { info.assume_init() };
    let ticks = times.ChangeTime.max(times.CreationTime);
    // Ticks are a hundred nanoseconds each, which in nanoseconds a u64 holds to the year 2185.
    let since_1601 = u64::try_from(ticks)
        .ok()
        .and_then(|ticks| ticks.checked_mul(100))
        .map(Duration::from_nanos);
    SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(FROM_1601))
        .zip(since_1601)
        .and_then(|(start, since)| start.checked_add(since))
        .ok_or_else(|| {
            io::Error::other(format!(
                "{} changed past what a clock holds",
                path.display()
            ))
        })
}

/// No dialogue for a crash, for a disk that is not there, or for a file that cannot be opened.
const NO_DIALOGUES: THREAD_ERROR_MODE =
    SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX;

/// A job that ends every process in it when steamship lets go of it, however steamship ends, and
/// ends a process that crashes rather than leaving it at Windows' crash handling.
const JOB_LIMITS: JOB_OBJECT_LIMIT =
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;

/// How a program is started. It is suspended until it is in the job, so that nothing it does
/// happens outside it. Its environment block is UTF-16, and the attribute list limits what it
/// inherits.
const CREATION: PROCESS_CREATION_FLAGS =
    CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;

/// A program given pipes has no console window either, which on a desktop nobody sees would
/// only be waste. One on a pseudo console must not be told so: it would get a console of its own
/// in place of the pseudo console, and nothing it wrote would arrive.
const PIPED: PROCESS_CREATION_FLAGS = CREATION | CREATE_NO_WINDOW;

/// The desktop a program runs on. Nobody switches to it, so nothing on it is ever seen, and
/// Windows keeps the keyboard focus from crossing from one desktop to another.
///
/// Its name carries the station it is on. In the session of whoever is signed in, Windows looks a
/// desktop named alone up on `WinSta0`, whatever station it was made on.
#[derive(Debug)]
struct Desktop {
    handle: HDESK,
    name: Vec<u16>,
}

/// The name of the window station this process is on, which goes in front of a desktop's name.
fn station() -> io::Result<Vec<u16>> {
    // SAFETY: takes no arguments. The handle belongs to the process and is never closed.
    let station = unsafe { GetProcessWindowStation() };
    let mut needed = 0_u32;
    // SAFETY: no buffer with a length of 0 is how the needed size is asked for. The call then
    // fails with ERROR_INSUFFICIENT_BUFFER, which is expected.
    let _: BOOL = unsafe {
        GetUserObjectInformationW(station, UOI_NAME, ptr::null_mut(), 0, &raw mut needed)
    };
    let length = usize::try_from(needed).map_err(io::Error::other)?;
    let mut name = vec![0_u16; length.div_ceil(2)];
    // SAFETY: `name` holds at least `needed` bytes and outlives the call.
    if unsafe {
        GetUserObjectInformationW(
            station,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    } == 0_i32
    {
        return Err(io::Error::last_os_error());
    }
    let end = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    name.truncate(end);
    Ok(name)
}

impl Desktop {
    fn create() -> io::Result<Self> {
        let station = station()?;
        let number = DESKTOPS.fetch_add(1, Ordering::Relaxed);
        let bare = wide(OsStr::new(&format!("steamship-{}-{number}", process::id())));
        // SAFETY: `bare` is NUL-terminated and outlives the call. Null for the other arguments
        // means the default display settings and security.
        let handle = unsafe {
            CreateDesktopW(
                bare.as_ptr(),
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
            let mut name = station;
            name.push(0x5C);
            name.extend_from_slice(&bare);
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
        // SAFETY: the job handle is open. Ending its processes is what it is for.
        let _: i32 = unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }

    fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
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

    /// The process's exit code, if it ends within `pause`.
    fn ended_within(&self, pause: Duration) -> io::Result<Option<i32>> {
        let handle = self.0.as_raw_handle();
        // A pause too long to count in milliseconds becomes no wait, which is still at most the pause.
        let milliseconds = u32::try_from(pause.as_millis()).unwrap_or(0);
        // SAFETY: the process handle is open for as long as `self`.
        match unsafe { WaitForSingleObject(handle, milliseconds) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            _ => return Err(io::Error::last_os_error()),
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
        let console = PseudoConsole::create(
            &typed,
            &written,
            COORD {
                X: COLUMNS.cast_signed(),
                Y: ROWS.cast_signed(),
            },
        )?;
        // The console holds its own copies now. Ours would keep the output open after it closes.
        drop(typed);
        drop(written);
        let process = spawn(
            program,
            args,
            environment_block(environment).as_deref(),
            directory,
            &desktop,
            &job,
            Streams::Console(console.console),
        )?;
        let watched = Process(process.0.try_clone()?);
        let leftovers = job.try_clone()?;
        // Whatever the program left running is still attached to the console, and closing the
        // console only asks it to go, which a busy machine can leave unanswered for as long as
        // it likes. So it is ended first, and the output ends with the program.
        let closing = thread::spawn(move || {
            watched.ended();
            leftovers.stop_all();
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
        self.process.ended_within(Duration::ZERO)
    }
}

impl Drop for Terminal {
    /// Ends the job outright: the thread closing the console holds the job open too, so its
    /// ending cannot be left to the last handle closing.
    fn drop(&mut self) {
        self.job.stop_all();
    }
}

type CreateConsole = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> i32;
type CloseConsole = unsafe extern "system" fn(HPCON);

/// The two entry points of Microsoft's `conpty.dll`, which make a pseudo console the same way
/// Windows' own do, but through the `OpenConsole.exe` beside the library.
#[derive(Clone, Copy)]
struct Conpty {
    create: CreateConsole,
    close: CloseConsole,
}

/// The maker of every pseudo console in this process, or why Windows' own is used. Filled at the
/// first console, or at the first [`console`].
static CONSOLE: OnceLock<Result<Conpty, String>> = OnceLock::new();

/// Loads the `conpty.dll` in `folder` and makes sure the `OpenConsole.exe` beside it hosts a
/// console, since the library falls back to Windows' own host without saying so when it finds
/// none.
fn shipped(folder: &Path) -> Result<Conpty, String> {
    if !folder.join("conpty.dll").is_file() {
        return Err("conpty.dll is not beside steamship.exe".to_owned());
    }
    if !folder.join("OpenConsole.exe").is_file() {
        return Err("OpenConsole.exe is not beside steamship.exe".to_owned());
    }
    let path = wide(folder.join("conpty.dll").as_os_str());
    // SAFETY: `path` is a NUL-terminated wide string that outlives the call, and the flags
    // search only the library's own folder and System32, so no other library can stand in.
    let library = unsafe {
        LoadLibraryExW(
            path.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if library.is_null() {
        return Err(format!(
            "conpty.dll beside steamship.exe did not load: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: `library` is a loaded module, and the name is NUL-terminated.
    let create = unsafe { GetProcAddress(library, c"ConptyCreatePseudoConsole".as_ptr().cast()) };
    // SAFETY: `library` is a loaded module, and the name is NUL-terminated.
    let close = unsafe { GetProcAddress(library, c"ConptyClosePseudoConsole".as_ptr().cast()) };
    let (Some(create), Some(close)) = (create, close) else {
        return Err("conpty.dll beside steamship.exe is not Microsoft's ConPTY".to_owned());
    };
    // SAFETY: Microsoft's library exports this entry point with exactly this signature, and
    // never unloads, so the pointer stays valid.
    let create =
        unsafe { mem::transmute::<unsafe extern "system" fn() -> isize, CreateConsole>(create) };
    // SAFETY: Microsoft's library exports this entry point with exactly this signature, and
    // never unloads, so the pointer stays valid.
    let close =
        unsafe { mem::transmute::<unsafe extern "system" fn() -> isize, CloseConsole>(close) };
    let conpty = Conpty { create, close };
    let (typed, _input) = io::pipe().map_err(|error| error.to_string())?;
    let (_output, written) = io::pipe().map_err(|error| error.to_string())?;
    PseudoConsole::create_with(Some(conpty), &typed, &written, COORD { X: 80, Y: 25 })
        .map(drop)
        .map_err(|error| format!("OpenConsole.exe beside steamship.exe did not start: {error}"))?;
    Ok(conpty)
}

/// Where the shipped console is looked for: the folder of the program itself, links followed, as
/// winget and Homebrew install it. A test build takes the folder `STEAMSHIP_TEST_CONPTY` names
/// when it is set, since a copy of the tree has no files beside its programs.
fn folder() -> Option<PathBuf> {
    #[cfg(feature = "fake-steamcmd")]
    if let Some(folder) = env::var_os("STEAMSHIP_TEST_CONPTY") {
        return Some(PathBuf::from(folder));
    }
    let program = fs::canonicalize(env::current_exe().ok()?).ok()?;
    program.parent().map(Path::to_path_buf)
}

fn maker() -> &'static Result<Conpty, String> {
    CONSOLE.get_or_init(|| {
        let folder =
            folder().ok_or_else(|| "steamship cannot tell where its program is".to_owned())?;
        shipped(&folder)
    })
}

/// Whether steamcmd's console is the one shipped beside steamship, or else why it is Windows' own.
///
/// # Errors
///
/// The reason the shipped console is not used.
pub fn console() -> Result<(), &'static str> {
    maker().as_ref().map(drop).map_err(String::as_str)
}

#[derive(Debug)]
struct PseudoConsole {
    console: HPCON,
    close: Option<CloseConsole>,
}

impl PseudoConsole {
    fn create(input: &PipeReader, output: &PipeWriter, size: COORD) -> io::Result<Self> {
        Self::create_with(maker().as_ref().ok().copied(), input, output, size)
    }

    fn create_with(
        maker: Option<Conpty>,
        input: &PipeReader,
        output: &PipeWriter,
        size: COORD,
    ) -> io::Result<Self> {
        let _handing_down = handing_down();
        let mut console: HPCON = 0;
        let windows: CreateConsole = CreatePseudoConsole;
        let create = maker.map_or(windows, |maker| maker.create);
        // SAFETY: both pipe ends are open for the length of the call, and the console takes its
        // own copies of them.
        let result = unsafe {
            create(
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
            Ok(Self {
                console,
                close: maker.map(|maker| maker.close),
            })
        }
    }
}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        let windows: CloseConsole = ClosePseudoConsole;
        let close = self.close.unwrap_or(windows);
        // SAFETY: the console came from this maker's create call and is closed once, here, by the
        // same maker.
        unsafe {
            close(self.console);
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
        // SAFETY: asks for this process's own standard input. Nothing is freed.
        let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        // SAFETY: `mode` is a valid place for the answer. A handle that is no console fails.
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
            // A high surrogate is half a character. The other half is the next unit.
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

/// In a test build whose test names a shell in `STEAMSHIP_FAKE_STEAMCMD`, that shell running the
/// test's fake `steamcmd.sh` with `args`, in place of the `steamcmd.exe` beside it that `program`
/// names. Windows cannot start a script itself, and a release build never has a fake to run.
#[cfg_attr(
    not(feature = "fake-steamcmd"),
    expect(
        clippy::missing_const_for_fn,
        reason = "a test build reads the environment here, which no const fn can"
    )
)]
fn faked(program: &Path, args: &[OsString]) -> Option<(PathBuf, Vec<OsString>)> {
    #[cfg(feature = "fake-steamcmd")]
    let shell = env::var_os("STEAMSHIP_FAKE_STEAMCMD")
        .filter(|_| program.file_name() == Some(OsStr::new("steamcmd.exe")))
        .map(|shell| {
            let script = program.with_file_name("steamcmd.sh").into_os_string();
            (
                PathBuf::from(shell),
                iter::once(script).chain(args.iter().cloned()).collect(),
            )
        });
    #[cfg(not(feature = "fake-steamcmd"))]
    let shell = {
        let _: (&Path, &[OsString]) = (program, args);
        None
    };
    shell
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
    let shelled = faked(program, args);
    let (program, args) = shelled.as_ref().map_or((program, args), |(shell, script)| {
        (shell.as_path(), script.as_slice())
    });
    let _handing_down = handing_down();
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
    // are limited by the attribute list to exactly those. With a console none are inherited.
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
    if inherit != 0_i32 {
        for handle in [input, output] {
            // SAFETY: both handles are open for the length of this call. Should Windows refuse,
            // the handle is the caller's to close straight after, which ends the moment as well.
            let _: i32 = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        }
    }
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
        // SAFETY: the process is suspended and was never let run. Ending it is all that is left.
        let _: i32 = unsafe { TerminateProcess(process.0.as_raw_handle(), 1) };
        return Err(error);
    }
    // SAFETY: the thread handle is open. It was created suspended once, so one resume starts it.
    match unsafe { ResumeThread(thread.as_raw_handle()) } {
        // The count it was suspended by before this resume: once, since it was made that way.
        1 => Ok(process),
        u32::MAX => Err(io::Error::last_os_error()),
        // It was running already, so it may have done something before it was in the job.
        count => {
            // SAFETY: as above. The program is ended rather than left running outside the rules.
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
        // SAFETY: a null list with a size to fill is how the needed size is asked for. The call
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
        // SAFETY: the list was initialised for one attribute. A value that points anywhere
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

/// Whether Windows refused to open a file because another program holds it open.
#[must_use]
pub fn held_elsewhere(error: &io::Error) -> bool {
    error.raw_os_error() == i32::try_from(ERROR_SHARING_VIOLATION).ok()
}

/// The name of the generic credential `item` is kept as, such as
/// `steamship:web-api-key:<home>`.
fn target_name(item: &keychain::Item) -> String {
    format!("steamship:{}:{}", item.kind, item.account())
}

/// [`target_name`], as Windows takes it.
fn target(item: &keychain::Item) -> Vec<u16> {
    wide(OsStr::new(&target_name(item)))
}

fn store_failed(error: &io::Error) -> keychain::Error {
    keychain::Error::Failed(error.to_string())
}

/// Whether the Credential Manager keeps `item`.
///
/// # Errors
///
/// When the Credential Manager cannot be read.
pub fn has_secret(item: &keychain::Item) -> Result<bool, keychain::Error> {
    Ok(kept_secret(item)?.is_some())
}

/// The secret the Credential Manager keeps as `item`, if it keeps one.
///
/// # Errors
///
/// When the Credential Manager cannot be read.
pub fn kept_secret(item: &keychain::Item) -> Result<Option<Zeroizing<Vec<u8>>>, keychain::Error> {
    let target = target(item);
    let mut found: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call. On success Windows points `found`
    // at a credential it allocated.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) } == 0_i32 {
        let error = io::Error::last_os_error();
        return if not_found(&error) {
            Ok(None)
        } else {
            Err(store_failed(&error))
        };
    }
    let found = Credential(NonNull::new(found).ok_or_else(|| {
        keychain::Error::Failed("Windows found the credential but gave none back".to_owned())
    })?);
    // SAFETY: CredReadW succeeded and gave back a credential, left alone until it is freed.
    let credential = unsafe { found.0.as_ref() };
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
struct Credential(NonNull<CREDENTIALW>);

impl Drop for Credential {
    fn drop(&mut self) {
        // SAFETY: the credential came from CredReadW and is freed once, here.
        unsafe {
            CredFree(self.0.as_ptr().cast_const().cast());
        }
    }
}

/// Keeps `secret` as `item`, in place of any kept before, for this user on this machine only.
///
/// # Errors
///
/// When the Credential Manager refuses it.
pub fn keep_secret(item: &keychain::Item, secret: &[u8]) -> Result<(), keychain::Error> {
    let mut target = target(item);
    let mut label = wide(OsStr::new(item.label));
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: u32::try_from(secret.len())
            .ok()
            .ok_or_else(|| keychain::Error::Failed("the secret is too long".to_owned()))?,
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

/// Removes `item`, and says whether it was kept.
///
/// # Errors
///
/// When the Credential Manager refuses.
pub fn forget_secret(item: &keychain::Item) -> Result<bool, keychain::Error> {
    let target = target(item);
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
    allow_alone(&name, &[user.sid()]).map(|()| true)
}

/// Opens `path` to write, made or emptied, as the current user's alone: its permissions are its
/// own, not its folder's, before anything is in it.
///
/// # Errors
///
/// When the file cannot be opened, or its permissions cannot be set.
pub fn create_private(path: &Path) -> io::Result<File> {
    // Nobody else can open the file while it is held, so nobody can in the moment between its
    // being made with its folder's permissions and its getting its own.
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .access_mode(GENERIC_WRITE | WRITE_DAC)
        .share_mode(0)
        .open(path)?;
    let user = User::current()?;
    let list = list_allowing(&[user.sid()], 0)?;
    // SAFETY: the handle is open with WRITE_DAC, and `list` is a complete list. The owner, the
    // group and the audit list are left as they are.
    let status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            list.as_ptr().cast(),
            ptr::null(),
        )
    };
    win32(status)?;
    Ok(file)
}

/// Gives the file or folder `name` permissions of its own, not inherited from above: one entry
/// for each of `sids`, which allows everything and is passed down to all a folder holds.
fn allow_alone(name: &[u16], sids: &[PSID]) -> io::Result<()> {
    let list = list_allowing(sids, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE)?;
    // SAFETY: `name` is NUL-terminated and `list` is a complete list. The owner, the group and
    // the audit list are left as they are.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            list.as_ptr().cast(),
            ptr::null(),
        )
    };
    win32(status)
}

/// An access list with one entry for each of `sids`, allowing everything, with `flags`. It is held in
/// words so that it is aligned as an ACL must be.
fn list_allowing(sids: &[PSID], flags: u32) -> io::Result<Vec<u32>> {
    // An entry's size counts its SID in place of the SidStart field that marks where it begins.
    let entry = size_of_u32::<ACCESS_ALLOWED_ACE>().saturating_sub(size_of_u32::<u32>());
    let size = sids.iter().fold(size_of_u32::<ACL>(), |size, &sid| {
        // SAFETY: the caller's SIDs are valid for the length of this function.
        let sid_length = unsafe { GetLengthSid(sid) };
        size.saturating_add(entry).saturating_add(sid_length)
    });
    let mut storage = vec![0_u32; usize::try_from(size.div_ceil(4)).map_err(io::Error::other)?];
    let acl: *mut ACL = storage.as_mut_ptr().cast();
    // SAFETY: `storage` holds at least `size` bytes, aligned as an ACL must be.
    if unsafe { InitializeAcl(acl, size, ACL_REVISION) } == 0_i32 {
        return Err(io::Error::last_os_error());
    }
    for &sid in sids {
        // SAFETY: the list was sized for exactly these entries, and each SID is copied into it.
        if unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, flags, FILE_ALL_ACCESS, sid) } == 0_i32
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(storage)
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
        // SAFETY: no buffer with a size of 0 is how the needed size is asked for. The call then
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
/// Never. The signature is the one every system shares.
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
/// runs it. Its files are held to their pinned SHA-256 like every other, but not to Valve's
/// signature, which they do not carry.
const NOT_VALVES: &str = "siteserverui/";

/// Every one of `files` under `root` that is steamcmd's own program or library and does not carry
/// a valid signature from `signer`, [`VALVE`] for steamcmd, each said as a change to the install.
pub fn unsigned<'file, Files>(root: &Path, files: Files, signer: &str) -> Vec<String>
where
    Files: IntoIterator<Item = &'file String>,
{
    let programs: Vec<_> = files
        .into_iter()
        .filter(|relative| !relative.starts_with(NOT_VALVES))
        .filter(|relative| {
            Path::new(relative.as_str())
                .extension()
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("dll")
                })
        })
        .collect();
    parallel::map(&programs, |relative| {
        match signature(&root.join(relative)) {
            Signature::Signed(name) if name == signer => None,
            Signature::Signed(name) => {
                Some(format!("{relative} is signed by {name}, not {signer}"))
            }
            Signature::Refused(status) => Some(format!(
                "{relative} has no valid signature (Windows says {status:#010x})"
            )),
        }
    })
    .into_iter()
    .flatten()
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    /// Windows verified the signature, and this is the signer's name.
    Signed(String),
    /// Windows did not accept the file's signature, or found none. This is its answer.
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

/// What Windows makes of `path`'s Authenticode signature, checked as `quietly_to_the_root`
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
    // SAFETY: `state` is the state a successful verification left open. It stays valid until
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
    use std::mem;
    use std::os::windows::ffi::OsStringExt as _;
    use std::path::PathBuf;
    use std::slice;
    use std::sync::mpsc;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::E_INVALIDARG;
    use windows_sys::Win32::Foundation::GetHandleInformation;
    use windows_sys::Win32::Globalization::lstrlenW;
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, INHERITED_ACE, SECURITY_MAX_SID_SIZE, WinLocalSystemSid,
    };
    use windows_sys::Win32::System::StationsAndDesktops::{
        CreateWindowStationW, GetThreadDesktop, SetProcessWindowStation,
    };
    use windows_sys::Win32::System::StationsAndDesktops::{DESKTOP_READOBJECTS, OpenDesktopW};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
    use windows_sys::Win32::UI::WindowsAndMessaging::WINSTA_ALL_ACCESS;

    /// A library Windows installs with a signature of its own inside it, rather than in a
    /// catalogue, which is the kind steamcmd's files carry.
    fn signed_by_microsoft() -> PathBuf {
        Path::new(&env::var_os("SystemRoot").unwrap()).join("System32/vcruntime140.dll")
    }

    /// What Windows answers for a file that carries no signature.
    const TRUST_E_NOSIGNATURE: u32 = 0x800b_0100;

    #[test]
    fn a_runs_handles_are_handed_down_only_while_its_program_starts() {
        let desktop = Desktop::create().unwrap();
        let job = Job::create().unwrap();
        let (_reader, writer) = io::pipe().unwrap();
        let input = File::open("NUL").unwrap();
        let cmd = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        let process = spawn(
            &cmd,
            &["/d", "/c", "exit 0"].map(OsString::from),
            None,
            &env::temp_dir(),
            &desktop,
            &job,
            Streams::Handles([input.as_raw_handle(), writer.as_raw_handle()]),
        )
        .unwrap();
        for handle in [input.as_raw_handle(), writer.as_raw_handle()] {
            let mut flags = 0_u32;
            // SAFETY: the handle is open, and `flags` receives its flags.
            let read = unsafe { GetHandleInformation(handle, &raw mut flags) };
            assert_ne!(read, 0_i32);
            assert_eq!(
                flags & HANDLE_FLAG_INHERIT,
                0,
                "a program started next would be handed it"
            );
        }
        process.ended();
    }

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

    /// The name of the window station or desktop behind `object`. It is read apart from
    /// `station`, whose answer the test below holds it against.
    fn name_of(object: HANDLE) -> String {
        let mut needed = 0_u32;
        // SAFETY: no buffer with a length of 0 asks for the size needed. The call then fails,
        // which is expected.
        let _: BOOL = unsafe {
            GetUserObjectInformationW(object, UOI_NAME, ptr::null_mut(), 0, &raw mut needed)
        };
        let mut name = vec![0_u16; usize::try_from(needed).unwrap().div_ceil(2)];
        // SAFETY: `name` holds at least `needed` bytes and outlives the call.
        let read = unsafe {
            GetUserObjectInformationW(
                object,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                needed,
                &raw mut needed,
            )
        };
        assert_ne!(read, 0_i32, "{}", io::Error::last_os_error());
        String::from_utf16_lossy(name.split(|&unit| unit == 0).next().unwrap())
    }

    #[test]
    #[ignore = "a part of the test below, which runs it on a desktop made for it"]
    fn the_station_and_desktop_this_process_runs_on() {
        // SAFETY: takes no arguments. The handle belongs to the process and is never closed.
        let station = unsafe { GetProcessWindowStation() };
        // SAFETY: takes no arguments and touches no memory.
        let id = unsafe { GetCurrentThreadId() };
        // SAFETY: `id` names this thread. The handle is the thread's own and is never closed.
        let desktop = unsafe { GetThreadDesktop(id) };
        println!("runs on {}\\{}", name_of(station), name_of(desktop));
    }

    /// The program is this test binary, which says where it runs and makes no window. Windows
    /// keeps a thread's windows on the desktop the thread runs on, so a program's windows are
    /// where it says it is. A program with a window of its own would put it in front of whoever
    /// is at the machine whenever a fault here left it off its desktop.
    #[test]
    fn a_program_runs_on_the_desktop_made_for_it() {
        let desktop = Desktop::create().unwrap();
        let job = Job::create().unwrap();
        let (mut reader, writer) = io::pipe().unwrap();
        let input = File::open("NUL").unwrap();
        let process = spawn(
            &env::current_exe().unwrap(),
            &[
                "--exact",
                "windows::tests::the_station_and_desktop_this_process_runs_on",
                "--ignored",
                "--nocapture",
            ]
            .map(OsString::from),
            None,
            &env::temp_dir(),
            &desktop,
            &job,
            Streams::Handles([input.as_raw_handle(), writer.as_raw_handle()]),
        )
        .unwrap();
        // The program has its own copy. Holding this one would keep the output open after it ends.
        drop(writer);
        // A program that hangs fails the test rather than holding it up for good.
        let (sender, receiver) = mpsc::channel();
        let _reading = thread::spawn(move || {
            let mut said = String::new();
            drop(sender.send(io::Read::read_to_string(&mut reader, &mut said).map(|_| said)));
        });
        let said = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("the program never ended its output")
            .unwrap();
        let code = process.ended_within(Duration::from_secs(30)).unwrap();
        let name = String::from_utf16_lossy(&desktop.name);
        let place = format!("runs on {}\n", name.trim_end_matches('\0'));
        // 0xC0000142 is STATUS_DLL_INIT_FAILED, which Windows gives a program that cannot start
        // on the desktop it was sent to.
        let cause = if code == Some(-0x3fff_febe_i32) {
            ", the code of a program that could not start on its desktop"
        } else {
            ""
        };
        assert!(
            !said.is_empty(),
            "the program printed nothing on {name:?} (exit code {code:?}{cause})"
        );
        assert!(said.contains(&place), "no {place:?} in:\n{said}");
    }

    /// A process that runs for a minute on a desktop of its own, inside a job.
    fn minute_long() -> (Desktop, Job, Process) {
        let desktop = Desktop::create().unwrap();
        let job = Job::create().unwrap();
        let input = File::open("NUL").unwrap();
        let output = File::options().write(true).open("NUL").unwrap();
        let cmd = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        let args: Vec<OsString> = ["/d", "/s", "/c", "ping -n 60 127.0.0.1 > nul"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let process = spawn(
            &cmd,
            &args,
            None,
            &env::temp_dir(),
            &desktop,
            &job,
            Streams::Handles([input.as_raw_handle(), output.as_raw_handle()]),
        )
        .unwrap();
        (desktop, job, process)
    }

    #[test]
    fn a_look_at_a_running_program_takes_its_whole_pause() {
        let (_desktop, job, process) = minute_long();
        let started = Instant::now();
        for _ in 0..10_u32 {
            assert_eq!(
                process.ended_within(Duration::from_millis(50)).unwrap(),
                None
            );
        }
        let took = started.elapsed();
        job.stop_all();
        process.ended();
        assert!(
            took >= Duration::from_millis(300),
            "10 looks of 50 ms took {took:?}"
        );
        assert_eq!(process.ended_within(Duration::ZERO).unwrap(), Some(1_i32));
    }

    #[test]
    fn a_look_at_a_running_program_takes_no_longer_than_its_pause() {
        let (_desktop, job, process) = minute_long();
        let started = Instant::now();
        for _ in 0..10_u32 {
            assert_eq!(
                process.ended_within(Duration::from_millis(1)).unwrap(),
                None
            );
        }
        let took = started.elapsed();
        job.stop_all();
        process.ended();
        assert!(
            took < Duration::from_millis(300),
            "10 looks of 1 ms took {took:?}"
        );
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
        let slash = desktop.name.iter().position(|&unit| unit == 0x5C).unwrap();
        let (_, rest) = desktop.name.split_at(slash.saturating_add(1));
        let name = rest.to_vec();
        assert!(desktop_exists(&name));
        drop(desktop);
        assert!(!desktop_exists(&name));
    }

    #[test]
    fn a_desktop_is_named_with_the_station_it_is_on() {
        let desktop = Desktop::create().unwrap();
        let name = String::from_utf16_lossy(&desktop.name);
        let slash = desktop.name.iter().position(|&unit| unit == 0x5C);
        assert!(slash.is_some_and(|slash| slash > 0), "{name:?}");
        let nul = desktop.name.iter().position(|&unit| unit == 0);
        assert_eq!(nul, Some(desktop.name.len().saturating_sub(1)), "{name:?}");
    }

    #[test]
    fn a_program_starts_on_its_desktop_when_steamship_is_on_a_station_without_input() {
        let output = run::output(run::command(env::current_exe().unwrap()).args([
            "--exact",
            "windows::tests::a_run_and_a_terminal_from_a_station_without_input",
            "--ignored",
        ]))
        .unwrap();
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && said.contains("test result: ok. 1 passed"),
            "{said}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "a part of the test above, which runs it in a process of its own"]
    fn a_run_and_a_terminal_from_a_station_without_input() {
        // SAFETY: a null name and null security make an unnamed station with default security.
        let station = unsafe {
            CreateWindowStationW(
                ptr::null(),
                0,
                WINSTA_ALL_ACCESS.cast_unsigned(),
                ptr::null(),
            )
        };
        assert!(!station.is_null(), "{}", io::Error::last_os_error());
        // SAFETY: `station` was just made and stays open for as long as the process runs.
        let moved = unsafe { SetProcessWindowStation(station) };
        assert_ne!(moved, 0_i32, "{}", io::Error::last_os_error());
        let whoami = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\whoami.exe");
        let finished = run(
            &whoami,
            &[],
            &[],
            &env::temp_dir(),
            Duration::from_secs(30),
            &[],
            None,
        )
        .unwrap();
        let said = String::from_utf8_lossy(&finished.output);
        assert_eq!(finished.code, Some(0_i32), "{said}");
        assert!(said.contains('\\'), "{said}");
        let (terminal, mut output, _input) =
            Terminal::start(&whoami, &[], &[], &env::temp_dir()).unwrap();
        let mut seen = Vec::new();
        let _: usize = io::Read::read_to_end(&mut output, &mut seen).unwrap();
        assert_eq!(terminal.wait().unwrap(), Some(0_i32));
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

    /// A temporary folder shared by the current user and the system, wherever the temporary folder
    /// lives.
    fn shared_folder() -> tempfile::TempDir {
        let parent = tempfile::tempdir().unwrap();
        let user = User::current().unwrap();
        let mut system = vec![0_u32; usize::try_from(SECURITY_MAX_SID_SIZE.div_ceil(4)).unwrap()];
        let mut size = SECURITY_MAX_SID_SIZE;
        // SAFETY: `system` holds `size` bytes, aligned as a SID must be.
        let made = unsafe {
            CreateWellKnownSid(
                WinLocalSystemSid,
                ptr::null_mut(),
                system.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        assert_ne!(made, 0_i32, "{}", io::Error::last_os_error());
        allow_alone(
            &wide(parent.path().as_os_str()),
            &[user.sid(), system.as_mut_ptr().cast()],
        )
        .unwrap();
        parent
    }

    #[test]
    fn a_private_file_is_its_users_alone_whatever_its_folder_allows_new_or_already_there() {
        let parent = shared_folder();
        let user = User::current().unwrap();
        let fresh = parent.path().join("login.txt");
        let there = parent.path().join("older login.txt");
        fs::write(&there, "an older login").unwrap();
        assert_eq!(
            Security::of(&wide(there.as_os_str()))
                .unwrap()
                .entries(&user)
                .len(),
            2
        );
        for path in [&fresh, &there] {
            let held = create_private(path).unwrap();
            assert!(
                File::open(path).is_err(),
                "{} can be opened while it is held",
                path.display()
            );
            drop(held);
            let security = Security::of(&wide(path.as_os_str())).unwrap();
            assert!(security.is_protected(), "{}", path.display());
            assert_eq!(
                security.entries(&user),
                [(ACCESS_ALLOWED_ACE_TYPE, 0, FILE_ALL_ACCESS, true)],
                "{}",
                path.display()
            );
            assert_eq!(fs::read(path).unwrap(), b"", "{}", path.display());
        }
    }

    #[test]
    fn a_new_folder_takes_its_permissions_from_its_parent_and_is_not_only_its_users() {
        let parent = shared_folder();
        let user = User::current().unwrap();
        let folder = parent.path().join("steamcmd");
        fs::create_dir_all(&folder).unwrap();
        let security = Security::of(&wide(folder.as_os_str())).unwrap();
        assert!(!security.is_protected());
        let passed_down = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE | INHERITED_ACE;
        assert_eq!(
            security.entries(&user),
            [
                (ACCESS_ALLOWED_ACE_TYPE, passed_down, FILE_ALL_ACCESS, true),
                (ACCESS_ALLOWED_ACE_TYPE, passed_down, FILE_ALL_ACCESS, false),
            ]
        );
        assert!(restrict(&folder).unwrap());
        assert!(
            Security::of(&wide(folder.as_os_str()))
                .unwrap()
                .is_protected()
        );
    }

    #[test]
    fn a_pseudo_console_windows_will_not_make_is_an_error() {
        let (typed, _input) = io::pipe().unwrap();
        let (_output, written) = io::pipe().unwrap();
        let refused = PseudoConsole::create(&typed, &written, COORD { X: 0, Y: 0 }).unwrap_err();
        assert_eq!(refused.raw_os_error(), Some(E_INVALIDARG));
        drop(PseudoConsole::create(&typed, &written, COORD { X: 80, Y: 25 }).unwrap());
    }

    fn copy_file(from: &Path, to: &Path) {
        let _: u64 = fs::copy(from, to).unwrap();
    }

    /// A new folder holding the package's two files, as installed beside steamship.
    fn package() -> tempfile::TempDir {
        let source = env::var_os("STEAMSHIP_TEST_CONPTY").expect(
            "STEAMSHIP_TEST_CONPTY names no folder: run `bash .github/conpty.sh x64 <folder>` and set it to that folder",
        );
        let folder = tempfile::tempdir().unwrap();
        for file in ["conpty.dll", "OpenConsole.exe"] {
            copy_file(&Path::new(&source).join(file), &folder.path().join(file));
        }
        folder
    }

    /// The reason `shipped` gives for `folder`.
    fn refused(folder: &Path) -> String {
        match shipped(folder) {
            Ok(_) => panic!("{} was taken as Microsoft's ConPTY", folder.display()),
            Err(reason) => reason,
        }
    }

    #[test]
    fn a_console_made_from_the_shipped_files_is_run_by_the_openconsole_beside_them() {
        let folder = package();
        let conpty = shipped(folder.path()).unwrap();
        let desktop = Desktop::create().unwrap();
        let job = Job::create().unwrap();
        let (typed, _input) = io::pipe().unwrap();
        let (mut output, written) = io::pipe().unwrap();
        let console =
            PseudoConsole::create_with(Some(conpty), &typed, &written, COORD { X: 80, Y: 25 })
                .unwrap();
        drop(typed);
        drop(written);
        let cmd = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        let process = spawn(
            &cmd,
            &["/d", "/c", "exit 0"].map(OsString::from),
            None,
            &env::temp_dir(),
            &desktop,
            &job,
            Streams::Console(console.console),
        )
        .unwrap();
        assert_eq!(
            process.ended_within(Duration::from_secs(30)).unwrap(),
            Some(0_i32)
        );
        drop(console);
        // Output that never ends fails the test rather than holding it up for good.
        let (sender, receiver) = mpsc::channel();
        let _reading = thread::spawn(move || {
            let mut seen = Vec::new();
            drop(sender.send(io::Read::read_to_end(&mut output, &mut seen).map(|_| seen)));
        });
        let seen = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("the output never ended")
            .unwrap();
        // Only Microsoft's host asks a terminal what it is, and Windows' own never does.
        assert!(
            seen.windows(3).any(|window| window == b"\x1b[c"),
            "{:?}",
            String::from_utf8_lossy(&seen)
        );
    }

    #[test]
    fn a_folder_without_either_file_leaves_the_console_to_windows_and_says_which() {
        let package = package();
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            refused(empty.path()),
            "conpty.dll is not beside steamship.exe"
        );
        let library_only = tempfile::tempdir().unwrap();
        copy_file(
            &package.path().join("conpty.dll"),
            &library_only.path().join("conpty.dll"),
        );
        assert_eq!(
            refused(library_only.path()),
            "OpenConsole.exe is not beside steamship.exe"
        );
        let host_only = tempfile::tempdir().unwrap();
        copy_file(
            &package.path().join("OpenConsole.exe"),
            &host_only.path().join("OpenConsole.exe"),
        );
        assert_eq!(
            refused(host_only.path()),
            "conpty.dll is not beside steamship.exe"
        );
    }

    #[test]
    fn a_library_of_another_architecture_leaves_the_console_to_windows() {
        let package = package();
        let folder = tempfile::tempdir().unwrap();
        copy_file(
            &package.path().join("OpenConsole.exe"),
            &folder.path().join("OpenConsole.exe"),
        );
        let x86 = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"SysWOW64\version.dll");
        copy_file(&x86, &folder.path().join("conpty.dll"));
        let reason = refused(folder.path());
        assert!(
            reason.starts_with("conpty.dll beside steamship.exe did not load: "),
            "{reason}"
        );
    }

    #[test]
    fn a_library_that_is_not_conpty_leaves_the_console_to_windows() {
        let package = package();
        let folder = tempfile::tempdir().unwrap();
        copy_file(
            &package.path().join("OpenConsole.exe"),
            &folder.path().join("OpenConsole.exe"),
        );
        copy_file(&signed_by_microsoft(), &folder.path().join("conpty.dll"));
        assert_eq!(
            refused(folder.path()),
            "conpty.dll beside steamship.exe is not Microsoft's ConPTY"
        );
    }

    #[test]
    fn an_openconsole_windows_cannot_start_leaves_the_console_to_windows() {
        let package = package();
        let folder = tempfile::tempdir().unwrap();
        copy_file(
            &package.path().join("conpty.dll"),
            &folder.path().join("conpty.dll"),
        );
        fs::write(folder.path().join("OpenConsole.exe"), "not a program").unwrap();
        let reason = refused(folder.path());
        assert!(
            reason.starts_with("OpenConsole.exe beside steamship.exe did not start: "),
            "{reason}"
        );
    }

    #[test]
    fn the_shipped_files_are_signed_by_microsoft() {
        let package = package();
        for file in ["conpty.dll", "OpenConsole.exe"] {
            assert_eq!(
                signature(&package.path().join(file)),
                Signature::Signed("Microsoft Corporation".to_owned()),
                "{file}"
            );
        }
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

    #[test]
    fn a_credential_windows_cannot_hold_is_an_error_not_one_missing() {
        // Longer than any generic credential's name may be.
        let item = keychain::Item::key(Path::new(&"x".repeat(40_000)));
        assert!(matches!(
            kept_secret(&item),
            Err(keychain::Error::Failed(_))
        ));
        assert!(matches!(
            forget_secret(&item),
            Err(keychain::Error::Failed(_))
        ));
        assert!(matches!(
            keep_secret(&item, b"0123"),
            Err(keychain::Error::Failed(_))
        ));
    }

    #[test]
    fn each_secret_is_a_generic_credential_named_for_its_kind_and_whose_it_is() {
        let home = Path::new(r"C:\Users\a\AppData\Local\steamship");
        assert_eq!(
            target_name(&keychain::Item::key(home)),
            r"steamship:web-api-key:C:\Users\a\AppData\Local\steamship"
        );
        assert_eq!(
            target_name(&keychain::Item::branch(home, 480, "testing")),
            r"steamship:branch-password:C:\Users\a\AppData\Local\steamship:480:testing"
        );
    }

    #[test]
    fn a_kept_key_is_shown_in_credential_manager_by_what_it_is() {
        // The lock the integration tests take (tests/common): the Credential Manager loses
        // changes made at the same moment by different processes.
        let store = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(env::temp_dir().join("steamship-tests-credentials.lock"))
            .unwrap();
        store.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        for item in [
            keychain::Item::key(home.path()),
            keychain::Item::branch(home.path(), 480, "testing"),
        ] {
            keep_secret(&item, b"0123456789abcdef0123456789abcdef").unwrap();
            let listed =
                run::output(run::command("cmdkey").arg(format!("/list:{}", target_name(&item))))
                    .unwrap();
            assert!(forget_secret(&item).unwrap());
            let listed = String::from_utf8_lossy(&listed.stdout);
            assert!(
                listed.contains(&format!("User: {}", item.label)),
                "{listed}"
            );
        }
    }

    /// What one thread of a run does: marks a pipe's handle to be handed down for a while, or
    /// starts a program.
    #[derive(Debug, Clone, Copy)]
    enum Step {
        Mark,
        Start,
    }

    fn steps() -> impl Strategy<Value = Step> {
        prop_oneof![Just(Step::Mark), Just(Step::Start)]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4))]

        #[test]
        fn no_program_starts_while_a_handle_is_marked_to_be_handed_down(
            threads in prop::collection::vec(prop::collection::vec(steps(), 1..3), 2..3),
        ) {
            use std::process::Stdio;
            use std::sync::Barrier;
            use windows_sys::Win32::Foundation::SetHandleInformation;

            let sort = Path::new(&env::var_os("SystemRoot").unwrap()).join(r"System32\sort.exe");
            let begin = Arc::new(Barrier::new(threads.len()));
            let readers = Arc::new(Mutex::new(Vec::new()));
            let children = Arc::new(Mutex::new(Vec::new()));
            let workers: Vec<_> = threads
                .into_iter()
                .map(|steps| {
                    let (begin, readers, children, sort) =
                        (Arc::clone(&begin), Arc::clone(&readers), Arc::clone(&children), sort.clone());
                    thread::spawn(move || {
                        let _leader = begin.wait().is_leader();
                        for step in steps {
                            match step {
                                Step::Mark => {
                                    let _held = handing_down();
                                    let (reader, writer) = io::pipe().unwrap();
                                    let handle = writer.as_raw_handle();
                                    // SAFETY: the handle is open for the length of the call.
                                    let set = unsafe {
                                        SetHandleInformation(
                                            handle,
                                            HANDLE_FLAG_INHERIT,
                                            HANDLE_FLAG_INHERIT,
                                        )
                                    };
                                    assert_ne!(set, 0_i32);
                                    thread::sleep(Duration::from_millis(20));
                                    // SAFETY: as above.
                                    let cleared =
                                        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
                                    assert_ne!(cleared, 0_i32);
                                    readers.lock().unwrap().push((reader, writer));
                                }
                                Step::Start => {
                                    let mut command = run::command(&sort);
                                    let _: &mut process::Command = command
                                        .stdin(Stdio::piped())
                                        .stdout(Stdio::null())
                                        .stderr(Stdio::null());
                                    children.lock().unwrap().push(run::spawn(&mut command).unwrap());
                                }
                            }
                        }
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
            // Every program is still running, so a write end it inherited is still open.
            let pipes: Vec<_> = mem::take(&mut *readers.lock().unwrap());
            let (sender, receiver) = mpsc::channel();
            let waiting = pipes.len();
            for (mut reader, writer) in pipes {
                drop(writer);
                let sender = sender.clone();
                drop(thread::spawn(move || {
                    let mut rest = Vec::new();
                    let read = io::Read::read_to_end(&mut reader, &mut rest);
                    sender.send(read.is_ok()).unwrap();
                }));
            }
            let ended = (0..waiting)
                .filter(|_| receiver.recv_timeout(Duration::from_secs(1)).is_ok_and(|ok| ok))
                .count();
            let started = mem::take(&mut *children.lock().unwrap());
            for mut child in started {
                drop(child.stdin.take());
                let _: process::ExitStatus = child.wait().unwrap();
            }
            prop_assert_eq!(ended, waiting, "a program inherited a marked handle");
        }
    }
}
