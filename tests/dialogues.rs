//! Windows' error dialogues, in a test process of their own: the setting is process-wide, and a
//! test beside this one that ran a program would set it too and hide a failure here.
#![cfg(windows)]
#![expect(
    unsafe_code,
    reason = "the error mode is only reachable through Win32 calls"
)]

use steamship::windows::silence_error_dialogues;
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
    SetErrorMode,
};

#[test]
fn crashes_and_missing_files_put_up_no_dialogue() {
    // SAFETY: changes one process-wide flag; nothing else runs in this process.
    let _: u32 = unsafe { SetErrorMode(0) };
    silence_error_dialogues();
    // SAFETY: reads one process-wide flag.
    let mode = unsafe { GetErrorMode() };
    for (name, flag) in [
        ("a crash", SEM_NOGPFAULTERRORBOX),
        ("a missing disk", SEM_FAILCRITICALERRORS),
        ("a file that cannot be opened", SEM_NOOPENFILEERRORBOX),
    ] {
        assert_eq!(mode & flag, flag, "{name} would still put up a dialogue");
    }
}
