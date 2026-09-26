//! Logging in: steamcmd's side of the conversation read as it comes, and the person's answers
//! passed to it.
//!
//! A password or code goes from the person straight to steamcmd's input, and is wiped once
//! written; it is never shown, logged or kept.

use std::io::{self, Read, Write};

use zeroize::Zeroizing;

use crate::conversation::{self, Step};
use crate::terminal::{Event, Reader};

/// The person logging in, as the conversation needs them.
pub trait Person {
    /// The password, typed.
    ///
    /// # Errors
    ///
    /// When it cannot be read, or the person gives up.
    fn password(&mut self) -> io::Result<Zeroizing<String>>;

    /// A Steam Guard code, typed.
    ///
    /// # Errors
    ///
    /// As for [`Person::password`].
    fn code(&mut self) -> io::Result<Zeroizing<String>>;

    /// steamcmd waits for the login to be approved in the Steam Mobile app.
    fn approving(&mut self);

    /// The login was approved.
    fn approved(&mut self);

    /// steamcmd said something steamship has no meaning for.
    fn said(&mut self, line: &str);
}

/// How the conversation ended, before steamcmd's exit code is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// steamcmd ended without refusing anything.
    Finished,
    /// steamcmd refused the login, for this reason in its own words.
    Refused(String),
}

/// Reads `output` to its end, answering each of steamcmd's prompts with what `person` types,
/// written to `input`.
///
/// # Errors
///
/// When the output cannot be read, the input cannot be written, or `person` gives no answer.
pub fn converse<Output, Input, Someone>(
    output: &mut Output,
    input: &mut Input,
    person: &mut Someone,
) -> io::Result<Ending>
where
    Output: Read,
    Input: Write,
    Someone: Person,
{
    let mut reader = Reader::default();
    let mut refused = None;
    // A prompt is seen again each time more output arrives before the line ends, as when the
    // cursor is moved; it is answered once.
    let mut answered = false;
    let mut chunk = [0_u8; 4096];
    loop {
        let length = output.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        for event in reader.read(chunk.get(..length).unwrap_or_default()) {
            if matches!(event, Event::Line(_)) {
                answered = false;
            }
            match conversation::step(&event) {
                Step::Password | Step::Code if answered => {}
                Step::Password => {
                    answered = true;
                    answer(input, &person.password()?)?;
                }
                Step::Code => {
                    answered = true;
                    answer(input, &person.code()?)?;
                }
                Step::Approve => person.approving(),
                Step::Approved => person.approved(),
                Step::Refused(reason) => {
                    if refused.is_none() {
                        refused = Some(reason);
                    }
                }
                Step::Other(line) => person.said(&line),
                Step::Quiet => {}
            }
        }
    }
    Ok(refused.map_or(Ending::Finished, Ending::Refused))
}

/// Types `text` into steamcmd, and Enter, which a terminal sends as a carriage return.
fn answer<Input>(input: &mut Input, text: &str) -> io::Result<()>
where
    Input: Write,
{
    input.write_all(text.as_bytes())?;
    input.write_all(b"\r")?;
    input.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A person who types the answers given and notes what they were shown.
    #[derive(Default)]
    struct Scripted {
        passwords: Vec<&'static str>,
        codes: Vec<&'static str>,
        seen: Vec<String>,
    }

    impl Person for Scripted {
        fn password(&mut self) -> io::Result<Zeroizing<String>> {
            self.seen.push("password".to_owned());
            let typed = self.passwords.pop().ok_or(io::ErrorKind::Interrupted)?;
            Ok(Zeroizing::new(typed.to_owned()))
        }

        fn code(&mut self) -> io::Result<Zeroizing<String>> {
            self.seen.push("code".to_owned());
            let typed = self.codes.pop().ok_or(io::ErrorKind::Interrupted)?;
            Ok(Zeroizing::new(typed.to_owned()))
        }

        fn approving(&mut self) {
            self.seen.push("approving".to_owned());
        }

        fn approved(&mut self) {
            self.seen.push("approved".to_owned());
        }

        fn said(&mut self, line: &str) {
            self.seen.push(format!("said {line}"));
        }
    }

    /// Output that arrives in the chunks given, as a terminal delivers it.
    struct Chunks(Vec<&'static [u8]>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let chunk = self.0.remove(0);
            let length = chunk.len().min(buf.len());
            buf.get_mut(..length)
                .unwrap_or_default()
                .copy_from_slice(chunk.get(..length).unwrap_or_default());
            Ok(length)
        }
    }

    #[test]
    fn a_password_and_an_approval_are_answered_and_shown_in_turn() {
        let mut output = Chunks(vec![
            b"\x1b[?25lRedirecting stderr to 'x'\r\nLoading Steam API...OK\r\n",
            b"password: ",
            b"\x1b[?25h",
            b"\r\nLogging in user 'build_bot' [U:1:0] to Steam Public...",
            b"This account is protected by a Steam Guard mobile authenticator.\r\n",
            b"Please confirm the login in the Steam Mobile app on your phone.\r\n",
            b"Waiting for confirmation...OK\r\nSomething new\r\n",
        ]);
        let mut input = Vec::new();
        let mut person = Scripted {
            passwords: vec!["hunter2"],
            ..Scripted::default()
        };
        let ending = converse(&mut output, &mut input, &mut person).unwrap();
        assert_eq!(ending, Ending::Finished);
        assert_eq!(input, b"hunter2\r");
        assert_eq!(
            person.seen,
            ["password", "approving", "approved", "said Something new"]
        );
    }

    #[test]
    fn a_code_is_asked_for_and_a_refusal_is_kept_to_the_end() {
        let mut output = Chunks(vec![
            b"password: ",
            b"\r\nSteam Guard code: ",
            b"\r\nLogging in user 'build_bot' [U:1:0] to Steam Public...FAILED (Invalid Login Auth Code)\r\n",
            b"Some later line\r\n",
        ]);
        let mut input = Vec::new();
        let mut person = Scripted {
            passwords: vec!["hunter2"],
            codes: vec!["AB12C"],
            ..Scripted::default()
        };
        let ending = converse(&mut output, &mut input, &mut person).unwrap();
        assert_eq!(
            ending,
            Ending::Refused("Invalid Login Auth Code".to_owned())
        );
        assert_eq!(input, b"hunter2\rAB12C\r");
        assert_eq!(person.seen, ["password", "code", "said Some later line"]);
    }

    #[test]
    fn a_person_who_gives_up_ends_the_conversation() {
        let mut output = Chunks(vec![b"password: ", b"never read\r\n"]);
        let mut input = Vec::new();
        let error = converse(&mut output, &mut input, &mut Scripted::default()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(input.is_empty());
    }

    #[test]
    fn a_password_or_code_echoed_by_the_console_is_never_shown() {
        let mut output = Chunks(vec![
            b"password:\x1b[1C",
            b"hunter2\r\n",
            b"Steam Guard code:\x1b[1C",
            b"AB12C\r\n",
        ]);
        let mut person = Scripted {
            passwords: vec!["hunter2"],
            codes: vec!["AB12C"],
            ..Scripted::default()
        };
        let ending = converse(&mut output, &mut Vec::new(), &mut person).unwrap();
        assert_eq!(ending, Ending::Finished);
        assert_eq!(person.seen, ["password", "code"]);
    }
}
