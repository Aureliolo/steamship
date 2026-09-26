//! Windows' error dialogues, in a test process of their own: the setting is process-wide, and a
//! test beside this one that ran a program would set it too and hide a failure here.
#![cfg(windows)]
#![expect(
    unsafe_code,
    reason = "the error mode is only reachable through Win32 calls"
)]

use steamship::windows::{NO_DIALOGUES, silence_error_dialogues};
use windows_sys::Win32::System::Diagnostics::Debug::{GetErrorMode, SetErrorMode};

#[test]
fn crashes_and_missing_files_put_up_no_dialogue() {
    // SAFETY: changes one process-wide flag; nothing else runs in this process.
    let _: u32 = unsafe { SetErrorMode(0) };
    silence_error_dialogues();
    // SAFETY: reads one process-wide flag.
    let mode = unsafe { GetErrorMode() };
    assert_eq!(mode & NO_DIALOGUES, NO_DIALOGUES);
    assert_ne!(NO_DIALOGUES.count_ones(), 0);
}
