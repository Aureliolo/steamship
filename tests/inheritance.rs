//! What a program run out of sight inherits, in a test process of its own: a program started
//! beside it by anything else would inherit the handle this test watches and keep it open.
#![cfg(windows)]
#![expect(unsafe_code, reason = "making a handle inheritable is a Win32 call")]

use std::ffi::OsString;
use std::io::{self, Read as _};
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use steamship::run::run;
use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};

#[test]
fn a_program_inherits_only_the_handles_it_is_given() {
    let folder = tempfile::tempdir().unwrap();
    let marker = folder.path().join("started");
    let (mut reader, writer) = io::pipe().unwrap();
    // SAFETY: the handle is open for the length of the call.
    let set = unsafe {
        SetHandleInformation(
            writer.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    };
    assert_ne!(set, 0_i32);
    let directory = folder.path().to_path_buf();
    let running = thread::spawn(move || {
        let args = [
            "/d",
            "/s",
            "/c",
            "echo.> started & ping -n 6 127.0.0.1 > nul",
        ]
        .map(OsString::from);
        run(
            Path::new(r"C:\Windows\System32\cmd.exe"),
            &args,
            &[],
            &directory,
            Duration::from_secs(60),
        )
    });
    while !marker.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    drop(writer);
    // Had the program inherited the pipe, it would hold it open for the five seconds it waits.
    let started = Instant::now();
    let mut rest = Vec::new();
    let _: usize = reader.read_to_end(&mut rest).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the pipe stayed open for {:?}",
        started.elapsed()
    );
    assert_eq!(running.join().unwrap().unwrap().code, Some(0_i32));
}
