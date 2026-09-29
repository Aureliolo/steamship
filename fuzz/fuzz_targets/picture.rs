//! Any bytes are a picture of some size or are refused with a reason; reading them never panics
//! or runs away, and a picture that is read has a size.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use libfuzzer_sys::fuzz_target;
use steamship::picture;

fuzz_target!(|data: &[u8]| {
    match picture::read(data) {
        Ok(read) if read.width == 0 || read.height == 0 => panic!("read with no size: {read:?}"),
        Err(why) if why.is_empty() => panic!("refused with no reason"),
        Ok(_) | Err(_) => {}
    }
});
