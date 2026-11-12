//! Reading what a program writes to a terminal as plain text.
//!
//! The escape sequences that colour it or move the cursor are left out. What remains comes as
//! whole lines, and as the line it has started but not ended, which is where a program leaves a
//! prompt it waits at.

use std::{mem, str};

/// How many columns the screen of a program's terminal has: wide enough that steamcmd's longest
/// line is never broken across two.
pub const COLUMNS: u16 = 240;

/// How many rows that screen has.
pub const ROWS: u16 = 50;

/// What the program has written so far and not yet been taken as a line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reader {
    /// Text of the line being written.
    pending: Vec<char>,
    /// Where in the line the next character goes: a backspace moves it back, and what is
    /// written there then takes the place of what was.
    cursor: usize,
    /// The screen row the line starts on, counted from 0, the top row of a new screen. Lines
    /// that scroll the screen count it on past the last row, which a move to a row takes off.
    row: usize,
    /// A carriage return was read and nothing written since. What is written next is the line
    /// again, and a line feed ends the line as it was.
    returned: bool,
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
                    match movement(rest.get(..length).unwrap_or_default()) {
                        Move::Forward(columns) => {
                            self.restart();
                            self.cursor = self.cursor.saturating_add(columns);
                            self.fill();
                        }
                        Move::To { row, column } => self.move_to(row, column, &mut events),
                        Move::Nowhere => {}
                    }
                    // Every turn moves on by a byte at least, which is what ends the loop.
                    at = at.saturating_add(length.max(1));
                }
                b'\n' => {
                    self.row = self.row.saturating_add(self.span()).saturating_add(1);
                    events.push(Event::Line(self.take()));
                    at = at.saturating_add(1);
                }
                // A carriage return moves back over the line. What is written next is the line
                // again, so the part before is dropped then. A line feed ends it as it was.
                b'\r' => {
                    self.returned = true;
                    at = at.saturating_add(1);
                }
                0x07 => at = at.saturating_add(1),
                0x08 => {
                    self.cursor = self.cursor.saturating_sub(1);
                    at = at.saturating_add(1);
                }
                _ => {
                    let rest = bytes.get(at..).unwrap_or_default();
                    let Some(length) = character_length(rest) else {
                        self.carried = rest.to_vec();
                        break;
                    };
                    self.restart();
                    for written in
                        String::from_utf8_lossy(rest.get(..length).unwrap_or_default()).chars()
                    {
                        self.put(written);
                    }
                    at = at.saturating_add(length.max(1));
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

    /// Moves the cursor to `row` and `column` of the screen. A pseudo console writes an empty
    /// line as such a move down past it, so a move below the line ends it and leaves a blank one
    /// for each row it passes. A move onto a row of the line goes on writing it. A move above it,
    /// to write over what is shown, only ends the line.
    fn move_to(&mut self, row: usize, column: usize, events: &mut Vec<Event>) {
        self.returned = false;
        let span = self.span();
        // The screen has scrolled until it shows the whole line, so the line's last row is
        // never below the screen's.
        let lowest = usize::from(ROWS).saturating_sub(1).saturating_sub(span);
        let first = self.row.min(lowest);
        let last = first.saturating_add(span);
        if row < first {
            if !self.pending.is_empty() {
                events.push(Event::Line(self.take()));
            }
            self.row = row;
            self.cursor = column;
        } else if row <= last {
            self.cursor = row
                .saturating_sub(first)
                .saturating_mul(usize::from(COLUMNS))
                .saturating_add(column);
        } else {
            events.push(Event::Line(self.take()));
            for _ in last.saturating_add(1)..row {
                events.push(Event::Line(String::new()));
            }
            self.row = row;
            self.cursor = column;
        }
        self.fill();
    }

    /// How many rows below its first the line reaches, where it is wider than the screen.
    fn span(&self) -> usize {
        self.pending
            .len()
            .saturating_sub(1)
            .checked_div(usize::from(COLUMNS))
            .unwrap_or(0)
    }

    /// Drops the line a carriage return moved back over, before it is written again.
    fn restart(&mut self) {
        if self.returned {
            drop(self.take());
        }
    }

    /// Spaces up to the cursor, where it has moved past the end of the line.
    fn fill(&mut self) {
        self.pending
            .resize(self.pending.len().max(self.cursor), ' ');
    }

    /// The line so far, which starts a new one.
    fn take(&mut self) -> String {
        self.cursor = 0;
        self.returned = false;
        mem::take(&mut self.pending).into_iter().collect()
    }
}

/// How many bytes a character starting with `lead` takes in UTF-8. A byte that starts none is
/// taken alone, and replaced when shown.
const fn utf8_length(lead: u8) -> usize {
    match lead {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

/// How many bytes of `bytes` the character at their start takes, or none when it is cut off. A
/// character is its lead byte and the continuation bytes that follow it, as many as the lead
/// calls for: anything else after the lead, a line feed above all, is not part of it.
fn character_length(bytes: &[u8]) -> Option<usize> {
    let wanted = utf8_length(*bytes.first()?);
    let continuing = bytes
        .iter()
        .skip(1)
        .take(wanted.saturating_sub(1))
        .take_while(|byte| (0x80..=0xbf).contains(*byte))
        .count();
    let complete = continuing.saturating_add(1);
    if complete < wanted && complete == bytes.len() {
        None
    } else {
        Some(complete)
    }
}

/// Where an escape sequence moves the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    /// Along the line by this many columns. A pseudo console writes the spaces at the end of
    /// what it shows, such as the one after a prompt, as such a move.
    Forward(usize),
    /// To this row and column of the screen, counted from 0.
    To {
        row: usize,
        column: usize,
    },
    Nowhere,
}

fn movement(sequence: &[u8]) -> Move {
    let Some(inside) = sequence.strip_prefix(b"\x1b[") else {
        return Move::Nowhere;
    };
    if let Some(count) = inside.strip_suffix(b"C") {
        return count_of(count).map_or(Move::Nowhere, Move::Forward);
    }
    let Some(position) = inside
        .strip_suffix(b"H")
        .or_else(|| inside.strip_suffix(b"f"))
    else {
        return Move::Nowhere;
    };
    let mut numbers = position.split(|&byte| byte == b';').map(count_of);
    let row = numbers.next().flatten();
    let column = numbers.next().unwrap_or(Some(1));
    match (row, column, numbers.next()) {
        (Some(row), Some(column), None) => Move::To {
            row: row.saturating_sub(1),
            column: column.saturating_sub(1),
        },
        _ => Move::Nowhere,
    }
}

/// A count in an escape sequence, which left out or 0 means 1, as the standard has it.
fn count_of(digits: &[u8]) -> Option<usize> {
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if digits.is_empty() {
        return Some(1);
    }
    // A console is never larger than this. A larger count is not a screen's worth of anything.
    let count = str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse::<usize>().ok())
        .map_or(LARGEST, |count| count.min(LARGEST));
    Some(count.max(1))
}

/// The most columns, or rows, a move is taken to cross.
const LARGEST: usize = 512;

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
            reader.read(
                b"\ra\x1b[Cb\x1b[3Cc\x1b[99999Cd\x1b[1;2Ce\x1b[0Cf\x1b[99999999999999999999999Cg\n"
            ),
            [Event::Line(format!(
                "a b   c{}de f{}g",
                " ".repeat(LARGEST),
                " ".repeat(LARGEST)
            ))]
        );
    }

    /// What steamship wrote to a pseudo console, a blank line and all, from a trial.
    const BLANK: &[u8] =
        b"\x1b[?25l\x1b[2J\x1b[m\x1b[1m\x1b[Hsteamship\x1b[22m\x1b[2m\x1b[1Clogout\r\n\
        \x1b]0;cmd.exe\x07\x1b[?25h  \x1b[2mlogin     \x1b[22mnone saved\r\n    \x1b[2mthe next \
        upload\x1b[22m\x1b[5;1H  \x1b[33m\xe2\x86\x91 \x1b[m\x1b[1mnewer\x1b[22m\r\n\x1b[?25h";

    #[test]
    fn a_move_down_past_rows_ends_the_line_and_leaves_them_blank() {
        assert_eq!(
            lines(&Reader::default().read(BLANK)),
            [
                "steamship logout",
                "  login     none saved",
                "    the next upload",
                "",
                "  \u{2191} newer"
            ]
        );
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"\x1b[2;1Ha\r\n\x1b[5;3Hb"),
            [
                Event::Line(String::new()),
                Event::Line("a".to_owned()),
                Event::Line(String::new()),
                Event::Line(String::new()),
                Event::Waiting("  b".to_owned())
            ]
        );
    }

    #[test]
    fn a_move_along_the_row_or_up_it_writes_over_ends_no_line_but_one_written() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"\x1b[3;1H[ 10%]\x1b[3;1f[ 90%]\x1b[3;9Hdone"),
            [
                Event::Line(String::new()),
                Event::Line(String::new()),
                Event::Waiting("[ 90%]  done".to_owned())
            ]
        );
        assert_eq!(
            reader.read(b"\x1b[1Habcdef\x1b[Hxy"),
            [
                Event::Line("[ 90%]  done".to_owned()),
                Event::Waiting("xycdef".to_owned())
            ]
        );
        assert_eq!(
            reader.read(b"\x1b[;4H!\x1b[1:1H\x1b[1;2;3H\x1b[?25h"),
            [Event::Waiting("xyc!ef".to_owned())]
        );
    }

    #[test]
    fn a_move_down_from_a_row_nothing_is_written_on_leaves_it_blank_and_a_move_up_ends_no_line() {
        assert_eq!(
            Reader::default().read(b"\x1b[9;1H"),
            vec![Event::Line(String::new()); 8]
        );
        let mut far = vec![Event::Line(String::new()); 511];
        far.push(Event::Waiting("b".to_owned()));
        assert_eq!(
            Reader::default().read(b"\x1b[99999999999999999999;1Hb"),
            far
        );
        assert_eq!(
            Reader::default().read(b"a\r\n\x1b[1;1Hb"),
            [Event::Line("a".to_owned()), Event::Waiting("b".to_owned())]
        );
    }

    fn read_all(reader: &mut Reader, chunks: &[&[u8]]) -> Vec<Event> {
        chunks.iter().flat_map(|chunk| reader.read(chunk)).collect()
    }

    /// What Microsoft's console writes before a program's first line, from a trial. It moves
    /// the cursor nowhere.
    const OPENING: [&[u8]; 4] = [
        b"\x1b[1t",
        b"\x1b[c\x1b[?1004h\x1b[?9001h",
        b"\x1b[?7l",
        b"\x1b[?7h",
    ];

    #[test]
    fn a_new_screen_starts_on_its_top_row_so_an_echo_moved_to_the_prompts_row_continues_its_line() {
        let mut reader = Reader::default();
        let mut seen = read_all(&mut reader, &OPENING);
        seen.extend(read_all(
            &mut reader,
            &[
                b"exited 0 \r\n",
                b"next: ",
                b"\x1b[2;7Htyped after\x1b[2;18H",
                b"\x1b[2;18H\r\n",
                b"\x1b[?1004l\x1b[?9001l",
            ],
        ));
        assert_eq!(lines(&seen), ["exited 0 ", "next: typed after"]);
    }

    /// The 60 lines a trial wrote down a screen of 50 rows, each in a chunk of its own.
    fn sixty_lines(reader: &mut Reader) -> Vec<Event> {
        let mut seen = read_all(reader, &OPENING);
        for number in 1_u8..=60 {
            seen.extend(reader.read(format!("line {number}\r\n").as_bytes()));
        }
        seen
    }

    #[test]
    fn at_the_foot_of_the_screen_a_line_feed_scrolls_and_the_next_line_is_on_the_last_row() {
        let mut reader = Reader::default();
        let mut seen = sixty_lines(&mut reader);
        seen.extend(read_all(
            &mut reader,
            &[
                b"next: ",
                b"\x1b[50;7Htyped after\x1b[50;18H",
                b"\x1b[50;18H",
                b"\r\n",
            ],
        ));
        let seen = lines(&seen);
        assert_eq!(seen.len(), 61);
        assert_eq!(seen.last(), Some(&"next: typed after"));
    }

    /// A prompt of 256 characters as a trial wrote it, in two chunks, on a screen 240 wide.
    fn wide_prompt(reader: &mut Reader) -> Vec<Event> {
        let mut seen = reader.read(&[b'x'; 160]);
        let mut rest = vec![b'x'; 90];
        rest.extend_from_slice(b"next: ");
        seen.extend(reader.read(&rest));
        seen
    }

    #[test]
    fn a_line_wider_than_the_screen_wraps_and_a_move_to_its_second_row_continues_it() {
        let mut reader = Reader::default();
        let mut seen = read_all(&mut reader, &OPENING);
        seen.extend(wide_prompt(&mut reader));
        seen.extend(read_all(
            &mut reader,
            &[b"\x1b[2;17Htyped after\x1b[2;28H", b"\x1b[2;28H\r\n"],
        ));
        assert_eq!(
            lines(&seen),
            [format!("{}next: typed after", "x".repeat(250))]
        );
    }

    #[test]
    fn a_wrapped_line_at_the_foot_scrolls_the_screen_and_its_rows_move_up() {
        let mut reader = Reader::default();
        let mut seen = sixty_lines(&mut reader);
        seen.extend(wide_prompt(&mut reader));
        seen.extend(read_all(
            &mut reader,
            &[b"\x1b[50;17Htyped after\x1b[50;28H", b"\x1b[50;28H\r\n"],
        ));
        let seen = lines(&seen);
        assert_eq!(seen.len(), 61);
        assert_eq!(
            seen.last().copied(),
            Some(format!("{}next: typed after", "x".repeat(250)).as_str())
        );
    }

    #[test]
    fn a_carriage_return_then_escapes_then_a_line_feed_ends_the_line_as_it_was() {
        let mut reader = Reader::default();
        let seen = read_all(
            &mut reader,
            &[
                b"\nafter\r",
                b"\nnext:\x1b[1C",
                b"typed after\r",
                b"\x1b[?9001l\x1b[?1004l\n",
            ],
        );
        assert_eq!(lines(&seen), ["", "after", "next: typed after"]);
    }

    #[test]
    fn after_a_carriage_return_a_move_forward_starts_the_line_again_and_a_move_to_a_row_keeps_it() {
        assert_eq!(
            Reader::default().read(b"ab\r\x1b[2Cc\n"),
            [Event::Line("  c".to_owned())]
        );
        assert_eq!(
            Reader::default().read(b"abc\r\x1b[3;1Hd"),
            [
                Event::Line("abc".to_owned()),
                Event::Line(String::new()),
                Event::Waiting("d".to_owned())
            ]
        );
        assert_eq!(
            Reader::default().read(b"abc\r\x1b[1;2Hd\n"),
            [Event::Line("adc".to_owned())]
        );
    }

    #[test]
    fn the_line_after_a_wrapped_one_starts_below_its_last_row() {
        let wide = "x".repeat(241);
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(format!("{wide}\r\nb\x1b[3;2Hc").as_bytes()),
            [Event::Line(wide), Event::Waiting("bc".to_owned())]
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
    fn a_line_end_cut_between_chunks_still_ends_the_line() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"Waiting for confirmation...OK\r"),
            [Event::Waiting("Waiting for confirmation...OK".to_owned())]
        );
        assert_eq!(
            reader.read(b"\nnext"),
            [
                Event::Line("Waiting for confirmation...OK".to_owned()),
                Event::Waiting("next".to_owned())
            ]
        );
        assert_eq!(reader.read(b"\r"), [Event::Waiting("next".to_owned())]);
        assert_eq!(reader.read(b"over"), [Event::Waiting("over".to_owned())]);
    }

    #[test]
    fn characters_of_every_length_are_read_whole_even_when_cut_between_chunks() {
        let text = "a\u{e9}\u{20ac}\u{1f600}";
        assert_eq!(
            Reader::default().read(format!("{text}\n").as_bytes()),
            [Event::Line(text.to_owned())]
        );
        let bytes = text.as_bytes();
        for cut in 1..bytes.len() {
            let (first, second) = bytes.split_at(cut);
            let mut reader = Reader::default();
            let mut events = reader.read(first);
            events.extend(reader.read(second));
            assert_eq!(
                events.last(),
                Some(&Event::Waiting(text.to_owned())),
                "{cut}"
            );
        }
    }

    #[test]
    fn an_escape_inside_a_title_does_not_end_it() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"\x1b]0;a\x1bXb\x07c\n"),
            [Event::Line("c".to_owned())]
        );
    }

    #[test]
    fn a_character_broken_off_by_a_line_feed_does_not_take_the_line_feed_with_it() {
        let mut reader = Reader::default();
        assert_eq!(
            reader.read(b"a\xe2\x82\nb\xf0\n"),
            [
                Event::Line("a\u{fffd}".to_owned()),
                Event::Line("b\u{fffd}".to_owned())
            ]
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
