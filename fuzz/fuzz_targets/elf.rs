//! Any bytes are either a program or not; reading them never panics or runs away.
#![no_main]

use libfuzzer_sys::fuzz_target;
use steamship::elf;

fuzz_target!(|data: &[u8]| {
    let _ = elf::is_program(&mut std::io::Cursor::new(data));
});
