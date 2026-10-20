//! Any text is read as Godot's export presets without a panic, never as more presets than it
//! has sections for, and with each custom feature split, trimmed and not empty.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;
use steamship::presets;

fuzz_target!(|text: &str| {
    let presets = presets::parse(Path::new("game"), text);
    let sections = text.matches("[preset.").count();
    assert!(
        presets.len() <= sections,
        "{} presets read from {sections} sections",
        presets.len()
    );
    for preset in &presets {
        for feature in &preset.features {
            assert!(
                !feature.is_empty() && !feature.contains(',') && feature.trim() == feature,
                "{preset:?} has the feature {feature:?}"
            );
        }
    }
});
