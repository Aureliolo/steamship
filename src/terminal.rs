//! Reading what a program writes to a terminal as plain text.
//!
//! The escape sequences that colour it or move the cursor are left out. What remains comes as
//! whole lines, and as the line it has started but not ended, which is where a program leaves a
//! prompt it waits at.

use std::{mem, str};

/// What the program has written so far and not yet been taken as a line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reader {
    /// Text of the line being written.
    pending: Vec<char>,
    /// Where in the line the next character goes: a backspace moves it back, and what is
    /// written there then takes the place of what was.
    cursor: usize,
    /// Bytes of a character or an escape sequence cut off at the end of the last chunk.
    carried: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A line the program ended.
    Line(String),
    /// The line the program has started and not ended, when it stops writing: a prompt.
    Waiting(String),
}

impl Reader {
    /// Takes `chunk`, the next bytes the program wrote, and answers every line it ended in them,
    /// then what it left unended, if anything.
    pub fn read(&mut self, chunk: &[u8]) -> Vec<Event> {
        let mut bytes = mem::take(&mut self.carried);
        bytes.extend_from_slice(chunk);
        let mut events = Vec::new();
        let mut at = 0;
        while let Some(&byte) = bytes.get(at) {
            match byte {
                0x1b => {
                    let rest = bytes.get(at..).unwrap_or_default();
                    let Some(length) = escape_length(rest) else {
                        self.carried = rest.to_vec();
                        break;
                    };
                    self.cursor = self
                        .cursor
                        .saturating_add(forward(rest.get(..length).unwrap_or_default()));
                    if self.pending.len() < self.cursor {
                        self.pending.resize(self.cursor, ' ');
                    }
                    at = at.saturating_add(length);
                }
                b'\n' => {
                    events.push(Event::Line(self.take()));
                    at = at.saturating_add(1);
                }
                // A carriage return alone moves back over the line: what follows is the line
                // again, so the part before is dropped; before a line feed it ends nothing.
                b'\r' => {
                    if bytes.get(at.saturating_add(1)) != Some(&b'\n') {
                        drop(self.take());
                    }
                    at = at.saturating_add(1);
                }
                0x07 => at = at.saturating_add(1),
                0x08 => {
                    self.cursor = self.cursor.saturating_sub(1);
                    at = at.saturating_add(1);
                }
                _ => {
                    let length = utf8_length(byte);
                    let end = at.saturating_add(length);
                    let Some(character) = bytes.get(at..end) else {
                        self.carried = bytes.get(at..).unwrap_or_default().to_vec();
                        break;
                    };
                    for written in String::from_utf8_lossy(character).chars() {
                        self.put(written);
                    }
                    at = end;
                }
            }
        }
        if !self.pending.is_empty() {
            events.push(Event::Waiting(self.pending.iter().collect()));
        }
        events
    }

    /// Writes `character` where the cursor is, over what was there, and moves past it.
    fn put(&mut self, character: char) {
        match self.pending.get_mut(self.cursor) {
            Some(there) => *there = character,
            None => self.pending.push(character),
        }
        self.cursor = self.cursor.saturating_add(1);
    }

    /// The line so far, which starts a new one.
    fn take(&mut self) -> String {
        self.cursor = 0;
        mem::take(&mut self.pending).into_iter().collect()
    }
}

/// How many bytes a character starting with `lead` takes in UTF-8; a byte that starts none is
/// taken alone, and replaced when shown.
const fn utf8_length(lead: u8) -> usize {
    match lead {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

/// How far `sequence` moves the cursor forward along the line. A pseudo console writes the
/// spaces at the end of what it shows, such as the one after a prompt, as such a move.
fn forward(sequence: &[u8]) -> usize {
    let Some(count) = sequence
        .strip_prefix(b"\x1b[")
        .and_then(|rest| rest.strip_suffix(b"C"))
    else {
        return 0;
    };
    if count.is_empty() {
        return 1;
    }
    // A console is never wider than this; a larger count is not a line's worth of spaces.
    str::from_utf8(count)
        .ok()
        .and_then(|count| count.parse::<usize>().ok())
        .map_or(0, |count| count.min(WIDEST))
}

/// The most columns a move forward is taken to cross.
const WIDEST: usize = 512;

/// The length of the escape sequence at the start of `bytes`, or none when it is cut off.
/// Control sequences (`ESC [` ... a final byte), operating system commands (`ESC ]` ... BEL or
/// `ESC \`) and the two-byte escapes are all a terminal program like steamcmd writes.
fn escape_length(bytes: &[u8]) -> Option<usize> {
    match bytes.get(1)? {
        b'[' => bytes
            .iter()
            .skip(2)
            .position(|byte| (0x40..=0x7e).contains(byte))
            .map(|end| end.saturating_add(3)),
        b']' => {
            let rest = bytes.get(2..)?;
            rest.iter()
                .enumerate()
                .find_map(|(index, &byte)| match byte {
                    0x07 => Some(index.saturating_add(3)),
                    0x1b if rest.get(index.saturating_add(1)) == Some(&b'\\') => {
                        Some(index.saturating_add(4))
                    }
                    _ => None,
                })
        }
        _ => Some(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Line(line) => Some(line.as_str()),
                Event::Waiting(_) => None,
            })
            .collect()
    }

    /// What steamcmd wrote to a pseudo console while logging in, as it arrived, from a trial.
    const TRIAL: [&[u8]; 8] = [
        b"\x1b[?9001h\x1b[?1004h",
        b"\x1b[?25l\x1b[2J\x1b[m\x1b[HRedirecting stderr to 'x'\r\n\x1b]0;C:\\steamcmd.exe\x07\x1b[?25h",
        b"Steam Console Client (c) Valve Corporation",
        b" - version 1788292693\r\n-- type 'quit' to exit --\r\nLoading Steam API...",
        b"OK\r\n",
        b"\x1b[38;5;15mCached credentials not found.\r\n",
        b"\x1b[m\r\npassword: ",
        b"\r\n\x1b[38;5;15m\r\nProceeding with login using username/password.\x1b[m\r\n",
    ];

    #[test]
    fn reads_steamcmds_lines_and_its_prompt_without_the_escapes() {
        let mut reader = Reader::default();
        let mut seen = Vec::new();
        for chunk in TRIAL {
            seen.extend(reader.read(chunk));
        }
        assert_eq!(
            lines(&seen),
            [
                "Redirecting stderr to 'x'",
                "Steam Console Client (c) Valve Corporation - version 1788292693",
                "-- type 'quit' to exit --",
                "Loading Steam API...OK",
                "Cached credentials not found.",
                "",
                "password: ",
                "",
                "Proceeding with login using username/password.",
            ]
        );
        assert!(seen.contains(&Event::Waiting("password: ".to_owned())));
    }

    #[test]
    fn an_escape_or_a_character_cut_between_chunks_is_finished_by_the_next() {
        let mut reader = Reader::default();
        assert_eq!(reader.read(b"a\x1b[38;5"), [Event::Waiting("a".to_owned())]);
        assert_eq!(reader.read(b";15mb\xc3"), [Event::Waiting("ab".to_owned())]);
        assert_eq!(reader.read(b"\xa9\n"), [Event::Line("ab\u{e9}".to_owned())]);
    }

    #[test]
    fn a_title_ends_at_bel_or_at_the_string_terminator() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"\x1b]0;one\x07a\x1b]0;two\x1b\\b\n"),
            [Event::Line("ab".to_owned())]
        );
    }

    #[test]
    fn a_line_written_over_keeps_only_what_was_written_last() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"Waiting...\rWaiting...OK\r\n"),
            [Event::Line("Waiting...OK".to_owned())]
        );
    }

    #[test]
    fn a_move_forward_is_read_as_the_spaces_it_stands_for() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"\x1b[Hpassword:\x1b[1C\x1b]0;cmd\x07\x1b[?25h"),
            [Event::Waiting("password: ".to_owned())]
        );
        assert_eq!(
            reader.read(b"\ra\x1b[Cb\x1b[3Cc\x1b[99999Cd\x1b[1;2Ce\n"),
            [Event::Line(format!("a b   c{}de", " ".repeat(WIDEST)))]
        );
    }

    #[test]
    fn bells_and_bytes_that_are_not_utf_8_do_no_harm() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"a\x07\xffb\x1bXc\n"),
            [Event::Line("a\u{fffd}bc".to_owned())]
        );
    }

    #[test]
    fn a_backspace_moves_back_and_what_follows_is_written_over_the_line() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"+qx\x08 \x08uit"),
            [Event::Waiting("+quit".to_owned())]
        );
        assert_eq!(
            reader.read(b"\x08\x08\x08\x08\x08\x08\x08-\n"),
            [Event::Line("-quit".to_owned())]
        );
        assert_eq!(
            reader.read(b"ab\x08\x1b[3Cc\n"),
            [Event::Line("ab  c".to_owned())]
        );
    }
}
