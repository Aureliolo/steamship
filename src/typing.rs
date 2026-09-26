//! What a person types at one of steamship's prompts: shown as typed, or as a dot a character
//! when it is a password, and wiped from memory once it has been used.

use std::io::{self, BufRead, Write};

use zeroize::Zeroizing;

#[cfg(unix)]
use crate::unix as native;
#[cfg(windows)]
use crate::windows as native;

/// The most characters taken at a prompt. Steam allows 64 in a password; the room for this many
/// is set aside at once, so the text is never moved to a larger buffer, which would leave a copy
/// behind that nothing wipes.
const LONGEST: usize = 256;

/// Where typed characters come from, one at a time.
pub trait Keyboard {
    /// The next character, or none when input has ended.
    ///
    /// # Errors
    ///
    /// When input cannot be read.
    fn key(&mut self) -> io::Result<Option<char>>;
}

impl Keyboard for native::Keys {
    fn key(&mut self) -> io::Result<Option<char>> {
        self.read_key()
    }
}

/// How what is typed is shown as it is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Echo {
    Typed,
    Dots,
}

/// One line typed at `keys`, shown on `shown` as `echo` says. Backspace takes back a character,
/// Enter ends the line, and Ctrl+C gives it up.
///
/// # Errors
///
/// When input ends before Enter, is given up, or cannot be read or shown.
pub fn line<Keys, Shown>(
    keys: &mut Keys,
    shown: &mut Shown,
    echo: Echo,
) -> io::Result<Zeroizing<String>>
where
    Keys: Keyboard,
    Shown: Write,
{
    let mut typed = Zeroizing::new(String::with_capacity(LONGEST.saturating_mul(4)));
    loop {
        let Some(key) = keys.key()? else {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "input ended"));
        };
        match key {
            '\r' | '\n' => break,
            '\u{3}' => {
                shown.write_all(b"\n")?;
                shown.flush()?;
                return Err(io::Error::new(io::ErrorKind::Interrupted, "given up"));
            }
            '\u{8}' | '\u{7f}' => {
                if typed.pop().is_some() {
                    shown.write_all(b"\x08 \x08")?;
                }
            }
            _ if key.is_control() || typed.chars().count() >= LONGEST => {}
            _ => {
                typed.push(key);
                match echo {
                    Echo::Typed => write!(shown, "{key}")?,
                    Echo::Dots => shown.write_all("\u{2022}".as_bytes())?,
                }
            }
        }
        shown.flush()?;
    }
    shown.write_all(b"\n")?;
    shown.flush()?;
    Ok(typed)
}

/// A line from `input` when it is not a terminal, as when a script pipes it in: nothing is
/// shown, since nothing is being typed.
///
/// # Errors
///
/// When input ends before a line, or cannot be read.
pub fn piped<Input>(input: &mut Input) -> io::Result<Zeroizing<String>>
where
    Input: BufRead,
{
    let mut read = Zeroizing::new(String::with_capacity(LONGEST.saturating_mul(4)));
    if input.read_line(&mut read)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "input ended"));
    }
    let length = read.trim_end_matches(['\r', '\n']).len();
    read.truncate(length);
    Ok(read)
}

/// A line typed at this process's terminal, or piped to it. Either way `shown` is left at the
/// start of a new line.
///
/// # Errors
///
/// As [`line`] and [`piped`].
pub fn ask<Shown>(shown: &mut Shown, echo: Echo) -> io::Result<Zeroizing<String>>
where
    Shown: Write,
{
    if let Some(mut keys) = native::Keys::open()? {
        return line(&mut keys, shown, echo);
    }
    let read = piped(&mut io::stdin().lock())?;
    shown.write_all(b"\n")?;
    shown.flush()?;
    Ok(read)
}

#[cfg(test)]
mod tests {
    use std::vec::IntoIter;

    use super::*;

    struct Typed(IntoIter<char>);

    impl Keyboard for Typed {
        fn key(&mut self) -> io::Result<Option<char>> {
            Ok(self.0.next())
        }
    }

    fn typed(text: &str) -> Typed {
        Typed(text.chars().collect::<Vec<_>>().into_iter())
    }

    #[test]
    fn a_password_is_shown_as_a_dot_a_character_and_backspace_takes_one_back() {
        let mut shown = Vec::new();
        let read = line(&mut typed("hunx\u{7f}ter2\r"), &mut shown, Echo::Dots).unwrap();
        assert_eq!(read.as_str(), "hunter2");
        let shown = String::from_utf8(shown).unwrap();
        assert!(!shown.contains("hunter"), "{shown}");
        assert_eq!(shown.matches('\u{2022}').count(), 8);
        assert!(shown.ends_with('\n'));
    }

    #[test]
    fn a_code_is_shown_as_typed_and_control_keys_do_nothing() {
        let mut shown = Vec::new();
        let read = line(&mut typed("\u{1b}AB\u{8}C12\n"), &mut shown, Echo::Typed).unwrap();
        assert_eq!(read.as_str(), "AC12");
        assert_eq!(String::from_utf8(shown).unwrap(), "AB\x08 \x08C12\n");
    }

    #[test]
    fn ctrl_c_gives_up_and_input_ending_early_is_an_error() {
        let given_up = line(&mut typed("abc\u{3}"), &mut io::sink(), Echo::Dots).unwrap_err();
        assert_eq!(given_up.kind(), io::ErrorKind::Interrupted);
        let ended = line(&mut typed("abc"), &mut io::sink(), Echo::Dots).unwrap_err();
        assert_eq!(ended.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_line_longer_than_any_password_is_cut_off_rather_than_moved() {
        let long = format!("{}\r", "x".repeat(LONGEST.saturating_add(10)));
        let read = line(&mut typed(&long), &mut io::sink(), Echo::Dots).unwrap();
        assert_eq!(read.chars().count(), LONGEST);
    }

    #[test]
    fn a_piped_line_is_read_without_its_line_end() {
        assert_eq!(piped(&mut &b"hunter2\r\n"[..]).unwrap().as_str(), "hunter2");
        assert_eq!(
            piped(&mut &b""[..]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
