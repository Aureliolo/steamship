//! An API name, a stat and two texts, split at NUL. Any text is read as the achievement editor's
//! tables without a panic. An achievement written to them reads back as it was written, whatever
//! characters it holds, and its two texts still differ once they change places.
#![no_main]

use libfuzzer_sys::fuzz_target;
use steamship::achievements::{Definition, Listed, Progress, Wanted};
use steamship::editor;

/// The definitions and the localisation written for one achievement.
fn written(api_name: &str, stat: &str, at: f64, texts: [&str; 2]) -> [String; 2] {
    let [name, description] = texts.map(str::to_owned);
    let listed = Listed {
        achievements: vec![Wanted {
            api_name: api_name.to_owned(),
            name,
            description,
            hidden: false,
            id: None,
            defined: Definition {
                progress: Some(Progress {
                    stat: stat.to_owned(),
                    at,
                }),
                ..Definition::default()
            },
        }],
        ..Listed::default()
    };
    let mut tables = editor::tables(480, &listed)
        .into_iter()
        .map(|table| table.text);
    [(); 2].map(|()| tables.next().unwrap_or_default())
}

fuzz_target!(|text: &str| {
    drop(editor::rebarred(text, text));
    drop(editor::reworded((text, text), (text, text)));

    let mut parts = text.splitn(4, '\0');
    let [api_name, stat, one, other] = [(); 4].map(|()| parts.next().unwrap_or_default());
    let [defined_before, localised_before] = written(api_name, stat, 1.0, [one, other]);
    let [defined_after, localised_after] = written(api_name, stat, 2.0, [other, one]);

    let bars = if stat.is_empty() {
        Vec::new()
    } else {
        vec![(
            api_name.to_owned(),
            format!("{stat} 0 - 1"),
            format!("{stat} 0 - 2"),
        )]
    };
    assert_eq!(
        editor::rebarred(&defined_before, &defined_after),
        bars,
        "the bar read back from:\n{defined_before}"
    );
    let reworded = if one == other {
        Vec::new()
    } else {
        vec![(
            api_name.to_owned(),
            vec!["English name", "English description"],
        )]
    };
    assert_eq!(
        editor::reworded(
            (&defined_before, &localised_before),
            (&defined_after, &localised_after)
        ),
        reworded,
        "the texts read back from:\n{localised_before}"
    );
});
