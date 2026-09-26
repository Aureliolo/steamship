//! What only Windows does. Kept in a module of its own so that everything here is compiled,
//! tested and mutation-tested on the system where it runs.
#![expect(
    unsafe_code,
    reason = "Windows' own APIs are the only way to ask Windows these questions"
)]

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read as _};
use std::iter;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle};
use std::panic;
use std::path::Path;
use std::process;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_SUCCESS, GENERIC_ALL, HANDLE, HANDLE_FLAG_INHERIT, HLOCAL, INVALID_HANDLE_VALUE,
    LocalFree, SetHandleInformation, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorControl,
    GetTokenInformation, InitializeAcl, OBJECT_INHERIT_ACE, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
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
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

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
        directory,
        &desktop,
        &job,
        [input.as_raw_handle(), writer.as_raw_handle()],
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
/// outside it; with no console window, which on a desktop nobody sees would only be waste; and
/// with the attribute list that limits what it inherits.
const CREATION: PROCESS_CREATION_FLAGS =
    CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT;

/// The desktop a program runs on. Nobody switches to it, so nothing on it is ever seen, and
/// Windows keeps the keyboard focus from crossing from one desktop to another.
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

struct Process(OwnedHandle);

impl Process {
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

/// Starts `program` suspended, on `desktop`, in `job`, handing down exactly `handles` (its input
/// first, then the one its output and errors both go to), and only then lets it run, so that it
/// never does anything outside the job.
fn spawn(
    program: &Path,
    args: &[OsString],
    directory: &Path,
    desktop: &Desktop,
    job: &Job,
    handles: [RawHandle; 2],
) -> io::Result<Process> {
    for handle in handles {
        // SAFETY: both handles are open for the length of this call.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
            == 0_i32
        {
            return Err(io::Error::last_os_error());
        }
    }
    let mut list = AttributeList::handing_down(&handles)?;
    // STARTUPINFOW asks for a writable name, though CreateProcessW only reads it.
    let mut desktop_name = desktop.name.clone();
    let [input, output] = handles;
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
    // SAFETY: every buffer is NUL-terminated and outlives the call, `line` is writable as the
    // call requires, `info` describes a live attribute list, and `started` receives the handles.
    // Inheritance is on, and the attribute list limits it to exactly `handles`.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATION,
            ptr::null(),
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

/// A process-thread attribute list naming the only handles a new process inherits.
struct AttributeList {
    /// Pointer-aligned storage for the list, which Windows sizes.
    storage: Vec<usize>,
    /// The handles the list points at, kept here for as long as the list.
    handles: Box<[RawHandle; 2]>,
}

impl AttributeList {
    fn handing_down(handles: &[RawHandle; 2]) -> io::Result<Self> {
        let mut size = 0_usize;
        // SAFETY: a null list with a size to fill is how the needed size is asked for; the call
        // then fails with ERROR_INSUFFICIENT_BUFFER, which is expected.
        let _: i32 =
            unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
        let mut list = Self {
            storage: vec![0; size.div_ceil(size_of::<usize>())],
            handles: Box::new(*handles),
        };
        // SAFETY: `storage` holds at least `size` bytes, aligned for the list.
        if unsafe { InitializeProcThreadAttributeList(list.pointer(), 1, 0, &raw mut size) }
            == 0_i32
        {
            return Err(io::Error::last_os_error());
        }
        let handed = &raw const *list.handles;
        let attribute =
            usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST).map_err(io::Error::other)?;
        // SAFETY: the list was initialised for one attribute, and the handle array it is told
        // about lives in `list` itself, so it outlives every use of the list.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.pointer(),
                0,
                attribute,
                handed.cast(),
                size_of::<[RawHandle; 2]>(),
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

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
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
    acl: *mut ACL,
}

impl Security {
    fn of(name: &[u16]) -> io::Result<Self> {
        let mut security = Self {
            descriptor: ptr::null_mut(),
            acl: ptr::null_mut(),
        };
        // SAFETY: `name` is NUL-terminated, and both outputs are valid places for pointers.
        let status = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut security.acl,
                ptr::null_mut(),
                &raw mut security.descriptor,
            )
        };
        win32(status)?;
        Ok(security)
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
        if self.acl.is_null() {
            return Vec::new();
        }
        // SAFETY: a list that is there points into the descriptor, alive as long as `self`.
        let count = unsafe { (*self.acl).AceCount };
        (0..u32::from(count))
            .filter_map(|index| {
                let mut entry = ptr::null_mut();
                // SAFETY: `index` is below the list's count.
                if unsafe { GetAce(self.acl, index, &raw mut entry) } == 0_i32 {
                    return None;
                }
                let entry: *const ACCESS_ALLOWED_ACE = entry.cast();
                // SAFETY: every kind of entry starts with its header and then its access mask,
                // and SidStart is read only for the kind that has one.
                let ACCESS_ALLOWED_ACE {
                    Header: header,
                    Mask: mask,
                    ..
                } = unsafe { *entry };
                let kind = u32::from(header.AceType);
                let mine = kind == ACCESS_ALLOWED_ACE_TYPE && {
                    // SAFETY: an access-allowed entry holds its SID from SidStart on.
                    let sid = unsafe { &raw const (*entry).SidStart };
                    // SAFETY: both SIDs are valid for the length of the call.
                    let equal = unsafe { EqualSid(sid.cast_mut().cast(), user.sid()) };
                    equal != 0_i32
                };
                Some((kind, u32::from(header.AceFlags), mask, mine))
            })
            .collect()
    }

    fn is_only(&self, user: &User) -> bool {
        self.is_protected()
            && self.entries(user)
                == [(
                    ACCESS_ALLOWED_ACE_TYPE,
                    OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                    FILE_ALL_ACCESS,
                    true,
                )]
    }
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
            &env::temp_dir(),
            &desktop,
            &job,
            [input.as_raw_handle(), output.as_raw_handle()],
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
