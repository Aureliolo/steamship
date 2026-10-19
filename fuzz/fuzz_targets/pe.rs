//! Any bytes are a Windows file's imports, not a Windows file, or a Windows file said to be
//! unreadable; reading them never panics or runs away, and reading from memory never fails.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use std::io::{Cursor, ErrorKind};

use libfuzzer_sys::fuzz_target;
use steamship::pe;

fuzz_target!(|data: &[u8]| {
    if let Err(error) = pe::imports(&mut Cursor::new(data))
        && error.kind() != ErrorKind::InvalidData
    {
        panic!("reading from memory failed: {error}");
    }
});
