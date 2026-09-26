//! What a program writes to a terminal, read in two chunks cut where the first byte says.
//! Reading never panics, no line holds a line end, and where the output is cut changes no line.
#![no_main]

use libfuzzer_sys::fuzz_target;
use steamship::terminal::{Event, Reader};

fn lines(events: Vec<Event>) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|event| match event {
            Event::Line(line) => Some(line),
            Event::Waiting(_) => None,
        })
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let Some((&cut, output)) = data.split_first() else {
        return;
    };
    let (first, second) = output.split_at(usize::from(cut).min(output.len()));
    let whole = lines(Reader::default().read(output));
    let mut reader = Reader::default();
    let mut parts = lines(reader.read(first));
    parts.extend(lines(reader.read(second)));
    assert_eq!(whole, parts, "where the output was cut changed its lines");
    for line in &whole {
        assert!(!line.contains(['\n', '\r']), "a line holds a line end");
    }
});
