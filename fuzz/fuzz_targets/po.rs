//! Any text is a gettext catalogue or is refused with the line it went wrong on; reading it never
//! panics or runs away, and a catalogue read holds each context and msgid once.
#![no_main]
#![expect(
    clippy::panic,
    reason = "a panic is how a fuzz target reports what it found"
)]

use libfuzzer_sys::fuzz_target;
use steamship::po;

fuzz_target!(|text: &str| {
    match po::read(text) {
        Ok(catalogue) => {
            for (index, entry) in catalogue.entries.iter().enumerate() {
                assert!(
                    !catalogue
                        .entries
                        .iter()
                        .skip(index.saturating_add(1))
                        .any(|other| other.context == entry.context && other.id == entry.id),
                    "read {entry:?} twice"
                );
            }
        }
        Err(why) if !why.starts_with("line ") => panic!("refused without its line: {why}"),
        Err(_) => {}
    }
});
