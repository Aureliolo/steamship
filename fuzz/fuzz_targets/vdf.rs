//! Any text either fails to parse or survives a write and a second parse unchanged.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use std::str;

use libfuzzer_sys::fuzz_target;
use steamship::vdf;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = str::from_utf8(data) else {
        return;
    };
    let Ok(tree) = vdf::parse(text) else {
        return;
    };
    let written =
        vdf::write(&tree).unwrap_or_else(|error| panic!("parsed but unwritable: {error}"));
    assert_eq!(vdf::parse(&written), Ok(tree), "written as:\n{written}");
});
