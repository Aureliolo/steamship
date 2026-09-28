//! Any bytes are either a program or not; reading them never panics or runs away, and reading
//! from memory never fails.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use steamship::macho;

fuzz_target!(|data: &[u8]| {
    if let Err(error) = macho::is_program(&mut Cursor::new(data)) {
        panic!("reading from memory failed: {error}");
    }
});
