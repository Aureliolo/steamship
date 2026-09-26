//! Keeping steamcmd's login out of everything steamship prints or writes.
//!
//! steamcmd keeps the token that logs the account in, with no password, in its `config.vdf`,
//! and more about the account in each `localconfig.vdf`. Every long value in those files is
//! taken as a secret, and every copy of one is replaced before output leaves steamship.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::install;

/// What a secret is replaced with.
pub const MARKER: &[u8] = b"[redacted]";

/// Values shorter than this are settings (a number, a flag, a language), not tokens, and would
/// only blank out ordinary words.
const SHORTEST: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Redactor {
    /// Longest first, so that a secret holding another is replaced whole.
    secrets: Vec<Vec<u8>>,
}

impl Redactor {
    /// The secrets in the steamcmd files of `home`, wherever steamcmd put them: beside itself on
    /// Windows, and under the `HOME` steamship gives it elsewhere.
    ///
    /// # Errors
    ///
    /// When one of those files is there but cannot be read.
    pub fn for_home(home: &Path) -> io::Result<Self> {
        let mut texts = Vec::new();
        for folder in steam_folders(home) {
            let config = folder.join("config").join("config.vdf");
            texts.extend(read_if_there(&config)?);
            let users = match fs::read_dir(folder.join("userdata")) {
                Ok(users) => users,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for user in users {
                let local = user?.path().join("config").join("localconfig.vdf");
                texts.extend(read_if_there(&local)?);
            }
        }
        Ok(Self::from_texts(&texts))
    }

    /// The secrets in `texts`, each the contents of a file in steamcmd's `KeyValues` format. Each
    /// is read token by token rather than parsed, so that nothing a parser would refuse can let
    /// a secret through.
    #[must_use]
    pub fn from_texts(texts: &[Vec<u8>]) -> Self {
        Self::sorted(
            texts
                .iter()
                .flat_map(|text| tokens(text))
                .filter(|&(token, is_value)| is_secret(token, is_value))
                .map(|(token, _)| token.to_vec())
                .collect(),
        )
    }

    /// The secrets of both. steamcmd may renew its token as it runs, so what is printed after a
    /// run is redacted with the secrets from before it and after it.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        let mut secrets = self.secrets;
        secrets.extend(other.secrets);
        Self::sorted(secrets)
    }

    fn sorted(mut secrets: Vec<Vec<u8>>) -> Self {
        // Replacing a secret no longer than the marker need not shorten the output, and could
        // put the secret back for the next pass to find, forever.
        secrets.retain(|secret| secret.len() > MARKER.len());
        secrets.sort_by(|left, right| right.len().cmp(&left.len()).then(left.cmp(right)));
        secrets.dedup();
        Self { secrets }
    }

    /// The secrets, longest first.
    #[must_use]
    pub fn secrets(&self) -> &[Vec<u8>] {
        &self.secrets
    }

    /// `output` with every secret replaced by [`MARKER`]. Replacing runs until none is left, so
    /// that no secret survives by being put together from what surrounds a replacement; each
    /// pass shortens the output, so it ends.
    #[must_use]
    pub fn redact(&self, output: &[u8]) -> Vec<u8> {
        let mut text = output.to_vec();
        while let Some(secret) = self
            .secrets
            .iter()
            .find(|secret| find(&text, secret).is_some())
        {
            text = replace(&text, secret);
        }
        text
    }
}

/// Where steamcmd keeps its state under `home`: its own folder on Windows, `$HOME/Steam` on
/// Linux, and the folder macOS keeps application data in.
fn steam_folders(home: &Path) -> [PathBuf; 3] {
    [
        home.join(install::FOLDER),
        home.join("Steam"),
        home.join("Library")
            .join("Application Support")
            .join("Steam"),
    ]
}

fn read_if_there(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether `token` could be a secret. A value is, when long enough. A key names a setting, and
/// only a long one with a digit in it, like a hash, is taken as more than a name.
fn is_secret(token: &[u8], is_value: bool) -> bool {
    token.len() >= SHORTEST && (is_value || token.iter().any(u8::is_ascii_digit))
}

/// Every quoted string's contents and every bare word in `text`, each with whether it is a value:
/// the token after a key, where a key is followed by either a value or a block.
fn tokens(text: &[u8]) -> Vec<(&[u8], bool)> {
    let mut found = Vec::new();
    let mut rest = text;
    let mut after_key = false;
    while let Some((&first, after)) = rest.split_first() {
        if first == b'"' {
            let end = after
                .iter()
                .position(|&byte| byte == b'"')
                .unwrap_or(after.len());
            let (inside, beyond) = after.split_at(end);
            found.push((inside, after_key));
            after_key = !after_key;
            rest = beyond.get(1..).unwrap_or_default();
        } else if matches!(first, b'{' | b'}') {
            after_key = false;
            rest = after;
        } else if first.is_ascii_whitespace() {
            rest = after;
        } else {
            let end = rest
                .iter()
                .position(|&byte| byte.is_ascii_whitespace() || matches!(byte, b'"' | b'{' | b'}'))
                .unwrap_or(rest.len());
            let (word, beyond) = rest.split_at(end);
            found.push((word, after_key));
            after_key = !after_key;
            rest = beyond;
        }
    }
    found
}

fn find(text: &[u8], secret: &[u8]) -> Option<usize> {
    text.windows(secret.len())
        .position(|window| window == secret)
}

fn replace(text: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut replaced = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = find(rest, secret) {
        let (before, from) = rest.split_at(at);
        // Each turn takes a whole secret off what remains, so the loop ends however `find`
        // answers.
        let Some(after) = from.get(secret.len()..) else {
            break;
        };
        replaced.extend_from_slice(before);
        replaced.extend_from_slice(MARKER);
        rest = after;
    }
    replaced.extend_from_slice(rest);
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TOKEN: &str = "eyJhbGciOiJFZERTQSJ9.eyJpc3MiOiJyOkQ0MTAiLCJzdWIiOiI3NjU2MTE5";
    const CONFIG: &str = r#""InstallConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"ConnectCache"
				{
					"3a1f0c2e"		"eyJhbGciOiJFZERTQSJ9.eyJpc3MiOiJyOkQ0MTAiLCJzdWIiOiI3NjU2MTE5"
				}
				"CellID"		"7"
			}
		}
	}
}
"#;

    #[test]
    fn takes_the_long_values_and_leaves_settings_alone() {
        let redactor = Redactor::from_texts(&[CONFIG.as_bytes().to_vec()]);
        assert_eq!(redactor.secrets, [TOKEN.as_bytes().to_vec()]);
        let output = format!("CellID 7, Steam, token {TOKEN}.");
        assert_eq!(
            redactor.redact(output.as_bytes()),
            b"CellID 7, Steam, token [redacted]."
        );
    }

    #[test]
    fn reads_bare_words_and_files_no_parser_would_take() {
        let text = b"{ key bare_word_that_is_long_enough other \"unterminated_quoted_secret_value";
        let redactor = Redactor::from_texts(&[text.to_vec()]);
        assert_eq!(
            redactor.secrets,
            [
                b"unterminated_quoted_secret_value".to_vec(),
                b"bare_word_that_is_long_enough".to_vec()
            ]
        );
    }

    #[test]
    fn a_key_is_a_name_unless_it_looks_like_a_hash() {
        let text = b"\"RememberedMachineID\" { \"a1b2c3d4e5f60718293a\" \"1\" }";
        let redactor = Redactor::from_texts(&[text.to_vec()]);
        assert_eq!(redactor.secrets, [b"a1b2c3d4e5f60718293a".to_vec()]);
    }

    #[test]
    fn replaces_a_secret_holding_another_whole() {
        let short = "abcdefghijklmnop";
        let long = format!("{short}qrstuvwxyz");
        let redactor =
            Redactor::from_texts(&[format!("\"a\" \"{short}\" \"b\" \"{long}\"").into_bytes()]);
        assert_eq!(redactor.redact(long.as_bytes()), MARKER);
        assert_eq!(
            redactor.redact(format!("{short}!").as_bytes()),
            b"[redacted]!"
        );
    }

    #[test]
    fn finds_steamcmds_files_wherever_each_system_keeps_them() {
        let home = tempfile::tempdir().unwrap();
        let places = [
            "steamcmd/config/config.vdf",
            "Steam/config/config.vdf",
            "Library/Application Support/Steam/config/config.vdf",
            "steamcmd/userdata/12345/config/localconfig.vdf",
            "Steam/userdata/12345/config/localconfig.vdf",
        ];
        for (index, place) in places.iter().enumerate() {
            let path = home.path().join(place);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("\"key\" \"secret_number_{index}_of_five\"")).unwrap();
        }
        let redactor = Redactor::for_home(home.path()).unwrap();
        for index in 0..places.len() {
            let secret = format!("secret_number_{index}_of_five");
            assert_eq!(
                redactor.redact(secret.as_bytes()),
                MARKER,
                "the secret in {}",
                places.get(index).copied().unwrap_or_default()
            );
        }
    }

    #[test]
    fn a_secret_no_longer_than_the_marker_is_never_kept() {
        let as_long = vec![b'x'; MARKER.len()];
        let longer = vec![b'x'; MARKER.len() + 1];
        assert_eq!(
            Redactor::sorted(vec![as_long, longer.clone()]).secrets,
            [longer]
        );
    }

    #[test]
    fn secrets_from_before_and_after_a_run_are_all_redacted() {
        let before = Redactor::from_texts(&[b"\"a\" \"the_token_before_the_run\"".to_vec()]);
        let after = Redactor::from_texts(&[b"\"a\" \"the_token_after_the_run\"".to_vec()]);
        let both = before.and(after);
        assert_eq!(
            both.redact(b"the_token_before_the_run the_token_after_the_run"),
            b"[redacted] [redacted]"
        );
    }

    #[test]
    fn a_home_with_no_login_has_nothing_to_redact() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            Redactor::for_home(home.path()).unwrap(),
            Redactor::default()
        );
    }

    #[test]
    fn a_user_folder_that_cannot_be_listed_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("Steam")).unwrap();
        fs::write(home.path().join("Steam/userdata"), "not a folder").unwrap();
        let error = Redactor::for_home(home.path()).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn answers_its_secrets_longest_first() {
        let redactor = Redactor::from_texts(&[
            b"\"a\" \"sixteen_letters_\" \"b\" \"seventeen_letters\"".to_vec(),
        ]);
        assert_eq!(
            redactor.secrets(),
            [b"seventeen_letters".to_vec(), b"sixteen_letters_".to_vec()]
        );
    }

    #[test]
    fn a_file_that_is_there_but_cannot_be_read_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("Steam/config/config.vdf")).unwrap();
        let error = Redactor::for_home(home.path()).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    proptest! {
        #[test]
        fn no_secret_is_left_in_what_comes_out(
            secrets in prop::collection::vec("[a-z\\[\\]]{16,24}", 1..4),
            pieces in prop::collection::vec("[a-z\\[\\] ]{0,20}", 0..6),
        ) {
            let texts: Vec<Vec<u8>> = secrets
                .iter()
                .map(|secret| format!("\"key\" \"{secret}\"").into_bytes())
                .collect();
            let redactor = Redactor::from_texts(&texts);
            let mut output = String::new();
            for (piece, secret) in pieces.iter().zip(secrets.iter().cycle()) {
                output.push_str(piece);
                output.push_str(secret);
            }
            let redacted = redactor.redact(output.as_bytes());
            for secret in &secrets {
                prop_assert!(find(&redacted, secret.as_bytes()).is_none());
            }
        }
    }
}
