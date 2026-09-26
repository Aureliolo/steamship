//! The build account's name: what steamcmd logs in as, and all of a login steamship ever holds.

use std::error;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Where `login` remembers the account, in the home.
const REMEMBERED: &str = "account";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl fmt::Display for Invalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "\"{}\" is not a Steam account name, which is 3 to 64 letters, digits or underscores",
            self.0
        )
    }
}

impl error::Error for Invalid {}

/// Why asking for the account's name did not give one.
#[derive(Debug)]
pub enum Asked {
    Io(io::Error),
    Invalid(Invalid),
}

impl fmt::Display for Asked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "the account name could not be read: {error}"),
            Self::Invalid(invalid) => invalid.fmt(formatter),
        }
    }
}

impl error::Error for Asked {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Invalid(invalid) => Some(invalid),
        }
    }
}

impl Account {
    /// The account called `name`. The name goes on steamcmd's command line, where anything
    /// starting with `+` is a command of its own, so only what Steam allows in a name passes.
    ///
    /// # Errors
    ///
    /// When `name` is not a Steam account name.
    pub fn parse(name: &str) -> Result<Self, Invalid> {
        let allowed = (3..=64).contains(&name.len())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if allowed {
            Ok(Self(name.to_owned()))
        } else {
            Err(Invalid(name.to_owned()))
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }

    /// The account `login` last remembered in `home`, if it has.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, or holds something that is not an account name.
    pub fn remembered(home: &Path) -> io::Result<Option<Self>> {
        match fs::read_to_string(home.join(REMEMBERED)) {
            Ok(text) => Self::parse(text.trim_end())
                .map(Some)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Asks for the account's name on `output` and reads it from `input`, which is what `login`
    /// does in a terminal when no name was given or remembered. A name is not a secret, so it is
    /// read as typed, and shown.
    ///
    /// # Errors
    ///
    /// When nothing can be read, or what was typed is not an account name.
    pub fn ask<Input, Output>(mut input: Input, mut output: Output) -> Result<Self, Asked>
    where
        Input: BufRead,
        Output: Write,
    {
        output
            .write_all(b"Steam account name: ")
            .and_then(|()| output.flush())
            .map_err(Asked::Io)?;
        let mut line = String::new();
        let _: usize = input.read_line(&mut line).map_err(Asked::Io)?;
        Self::parse(line.trim()).map_err(Asked::Invalid)
    }

    /// Remembers this account in `home` for later runs.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn remember(&self, home: &Path) -> io::Result<()> {
        fs::write(home.join(REMEMBERED), format!("{}\n", self.0))
    }
}

impl fmt::Display for Account {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn takes_what_steam_allows_in_a_name() {
        for name in ["abc", "build_bot_2", &"a".repeat(64)] {
            assert_eq!(Account::parse(name).unwrap().name(), name);
        }
    }

    #[test]
    fn refuses_anything_steamcmd_could_read_as_more_than_a_name() {
        for name in [
            "",
            "ab",
            &"a".repeat(65),
            "+quit",
            "-help",
            "two words",
            "name\n+quit",
            "\"quoted\"",
            "\u{fc}mlaut",
        ] {
            assert_eq!(Account::parse(name), Err(Invalid(name.to_owned())));
        }
    }

    proptest! {
        #[test]
        fn a_name_that_passes_is_one_plain_word(name in ".{0,80}") {
            if let Ok(account) = Account::parse(&name) {
                prop_assert!(!account.name().starts_with(['+', '-']));
                prop_assert!(account.name().bytes().all(|byte| byte.is_ascii_graphic()));
            }
        }
    }

    #[test]
    fn remembers_the_account_for_later_runs() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Account::remembered(home.path()).unwrap(), None);
        let account = Account::parse("build_bot").unwrap();
        account.remember(home.path()).unwrap();
        assert_eq!(Account::remembered(home.path()).unwrap(), Some(account));
    }

    #[test]
    fn a_remembered_name_that_is_not_one_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(REMEMBERED), "+quit\n").unwrap();
        let error = Account::remembered(home.path()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_remembered_name_that_cannot_be_read_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(REMEMBERED)).unwrap();
        let error = Account::remembered(home.path()).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn asks_for_the_name_and_takes_what_is_typed() {
        let mut shown = Vec::new();
        let account = Account::ask(&b"build_bot\r\n"[..], &mut shown).unwrap();
        assert_eq!(account.name(), "build_bot");
        assert_eq!(shown, b"Steam account name: ");
    }

    #[test]
    fn a_typed_name_that_is_not_one_or_nothing_typed_is_refused() {
        for typed in [&b"+quit\n"[..], b""] {
            assert!(
                matches!(Account::ask(typed, io::sink()), Err(Asked::Invalid(_))),
                "{typed:?}"
            );
        }
    }

    /// A reader that fails, as a closed terminal does.
    struct Broken;

    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn a_name_that_cannot_be_read_is_an_error_that_says_so() {
        let asked = Account::ask(io::BufReader::new(Broken), io::sink()).unwrap_err();
        assert!(matches!(asked, Asked::Io(_)));
        assert!(
            asked
                .to_string()
                .starts_with("the account name could not be read: ")
        );
        assert!(error::Error::source(&asked).is_some());
        let invalid = Account::ask(&b"x\n"[..], io::sink()).unwrap_err();
        assert_eq!(invalid.to_string(), Invalid("x".to_owned()).to_string());
        assert!(error::Error::source(&invalid).is_some());
    }

    #[test]
    fn prints_as_its_name() {
        assert_eq!(
            Account::parse("build_bot").unwrap().to_string(),
            "build_bot"
        );
    }
}
