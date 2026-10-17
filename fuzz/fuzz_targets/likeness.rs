//! Any two lots of bytes, taken as an icon's file and the image Steam serves of it, are compared
//! or refused with a reason; comparing never panics or runs away, and no more pixels differ than
//! were compared.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use libfuzzer_sys::fuzz_target;
use steamship::likeness;

fuzz_target!(|data: &[u8]| {
    let (file, served) = data.split_at(data.len().checked_div(2).unwrap_or(0));
    match likeness::compare(file, served) {
        Ok(alike) if alike.differing > alike.compared => {
            panic!("more differ than compared: {alike:?}")
        }
        Err(why) if why.is_empty() => panic!("refused with no reason"),
        Ok(_) | Err(_) => {}
    }
});
