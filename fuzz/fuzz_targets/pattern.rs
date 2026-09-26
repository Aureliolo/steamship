//! A pattern and a text, split at the first NUL. Matching never panics, and text with no
//! wildcards in it always matches itself.
#![no_main]

use libfuzzer_sys::fuzz_target;
use steamship::pattern;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let (wanted, text) = input.split_once('\0').unwrap_or((input, ""));
    for fold in [false, true] {
        let _ = pattern::matches_folding(wanted, text, fold);
        if !text.contains(['*', '?']) {
            assert!(pattern::matches_folding(text, text, fold));
        }
    }
});
