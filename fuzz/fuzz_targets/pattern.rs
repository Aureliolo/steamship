//! A pattern and a text, split at the first NUL. Matching never panics, ignoring case only ever
//! adds matches, and text with no wildcards in it always matches itself.
#![no_main]

use std::str;

use libfuzzer_sys::fuzz_target;
use steamship::pattern::matches_folding;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = str::from_utf8(data) else {
        return;
    };
    let (wanted, text) = input.split_once('\0').unwrap_or((input, ""));
    let exact = matches_folding(wanted, text, false);
    let folded = matches_folding(wanted, text, true);
    assert!(
        !exact || folded,
        "{wanted:?} matches {text:?} only when case matters"
    );
    if !text.contains(['*', '?']) {
        assert!(
            matches_folding(text, text, false),
            "{text:?} does not match itself"
        );
    }
});
