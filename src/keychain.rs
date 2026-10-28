//! The system's own credential store, where steamship keeps the Web API key, and the passwords of
//! the beta branches it fetches, between runs.
//!
//! That is the Credential Manager on Windows, the Keychain on macOS, and on Linux the Secret
//! Service that GNOME Keyring and `KWallet` provide. One key is kept for each steamship home, as
//! the Steam login is, so that `logout` forgets both and a test with a home of its own never meets
//! a person's real key; a branch's password is kept for the home, the app and the branch.

use std::error;
use std::fmt;
use std::iter;
use std::path::Path;
use std::str;

use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
use crate::macos as platform;
#[cfg(target_os = "linux")]
use crate::secret_service as platform;
use crate::webapi::Key;
#[cfg(windows)]
use crate::windows as platform;

/// The store, by the name its system gives it.
#[cfg(windows)]
pub const STORE: &str = "Windows Credential Manager";
#[cfg(target_os = "macos")]
pub const STORE: &str = "the macOS Keychain";
#[cfg(target_os = "linux")]
pub const STORE: &str = "the Secret Service";

/// What to know about the store once a key is kept in it, a line at a time. The Keychain trusts
/// only the binary that kept the key, and an ad-hoc signed steamship is another binary after each
/// upgrade.
#[cfg(target_os = "macos")]
pub const KEPT_NOTE: &[&str] = &[
    "after each steamship upgrade, macOS asks once whether it may use the key; Always Allow",
    "holds until the next upgrade",
    "a way around that question is welcome as a pull request: github.com/Aureliolo/steamship",
];
#[cfg(not(target_os = "macos"))]
pub const KEPT_NOTE: &[&str] = &[];

/// What the key is called where the store shows it.
pub const LABEL: &str = "steamship Web API key";

/// What a branch's password is called where the store shows it.
pub const BRANCH_LABEL: &str = "steamship branch password";

/// A secret as the store keeps it: what kind it is and what the store calls it, the steamship
/// home it is kept for, and what else tells it apart within that home, by name and value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: &'static str,
    pub label: &'static str,
    pub home: String,
    pub within: Vec<(&'static str, String)>,
}

impl Item {
    /// The Web API key kept for `home`.
    #[must_use]
    pub fn key(home: &Path) -> Self {
        Self {
            kind: "web-api-key",
            label: LABEL,
            home: home.display().to_string(),
            within: Vec::new(),
        }
    }

    /// The password of `app`'s `branch` kept for `home`.
    #[must_use]
    pub fn branch(home: &Path, app: u32, branch: &str) -> Self {
        Self {
            kind: "branch-password",
            label: BRANCH_LABEL,
            home: home.display().to_string(),
            within: vec![("app", app.to_string()), ("branch", branch.to_owned())],
        }
    }

    /// The one name the item is found by among those of its kind: the home, then what tells it
    /// apart within the home, each after a colon.
    #[must_use]
    pub fn account(&self) -> String {
        let parts: Vec<&str> = iter::once(self.home.as_str())
            .chain(self.within.iter().map(|(_, value)| value.as_str()))
            .collect();
        parts.join(":")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// There is no store to reach, as on a machine with no desktop session.
    Unavailable(String),
    /// The store refused, for this reason.
    Failed(String),
    /// What the store keeps is not a Web API key.
    NotAKey,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(formatter, "no credential store: {why}"),
            Self::Failed(why) => write!(formatter, "{STORE}: {why}"),
            Self::NotAKey => write!(
                formatter,
                "{STORE} keeps something that is not a publisher Web API key"
            ),
        }
    }
}

impl error::Error for Error {}

/// Whether a key is kept for `home`, found out without asking to unlock anything.
///
/// # Errors
///
/// When the store cannot be reached or read.
pub fn has(home: &Path) -> Result<bool, Error> {
    platform::has_secret(&Item::key(home))
}

/// The key kept for `home`, if there is one. The store may ask to be unlocked first.
///
/// # Errors
///
/// When the store cannot be reached or read, or keeps something that is not a key.
pub fn kept(home: &Path) -> Result<Option<Key>, Error> {
    let Some(secret) = platform::kept_secret(&Item::key(home))? else {
        return Ok(None);
    };
    let text = str::from_utf8(&secret).ok().ok_or(Error::NotAKey)?;
    Key::parse(text).ok().ok_or(Error::NotAKey).map(Some)
}

/// Keeps `key` for `home`, in place of any kept before.
///
/// # Errors
///
/// When the store cannot be reached, or refuses.
pub fn keep(home: &Path, key: &Key) -> Result<(), Error> {
    platform::keep_secret(&Item::key(home), key.text().as_bytes())
}

/// Forgets the key kept for `home`, and says whether there was one.
///
/// # Errors
///
/// When the store cannot be reached, or refuses.
pub fn forget(home: &Path) -> Result<bool, Error> {
    platform::forget_secret(&Item::key(home))
}

/// The password of `app`'s `branch` kept for `home`, if there is one.
///
/// # Errors
///
/// When the store cannot be reached or read, or keeps something that is not text there.
pub fn branch_password(
    home: &Path,
    app: u32,
    branch: &str,
) -> Result<Option<Zeroizing<String>>, Error> {
    let Some(secret) = platform::kept_secret(&Item::branch(home, app, branch))? else {
        return Ok(None);
    };
    let text = str::from_utf8(&secret)
        .ok()
        .ok_or_else(|| Error::Failed("it keeps a branch password that is not text".to_owned()))?;
    Ok(Some(Zeroizing::new(text.to_owned())))
}

/// Keeps `password` as that of `app`'s `branch` for `home`, in place of any kept before.
///
/// # Errors
///
/// When the store cannot be reached, or refuses.
pub fn keep_branch_password(
    home: &Path,
    app: u32,
    branch: &str,
    password: &str,
) -> Result<(), Error> {
    platform::keep_secret(&Item::branch(home, app, branch), password.as_bytes())
}

/// Forgets the password of `app`'s `branch` kept for `home`, and says whether there was one.
///
/// # Errors
///
/// When the store cannot be reached, or refuses.
pub fn forget_branch_password(home: &Path, app: u32, branch: &str) -> Result<bool, Error> {
    platform::forget_secret(&Item::branch(home, app, branch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_the_store_and_never_what_it_keeps() {
        assert_eq!(
            Error::Unavailable("no D-Bus session bus".to_owned()).to_string(),
            "no credential store: no D-Bus session bus"
        );
        assert_eq!(
            Error::Failed("refused".to_owned()).to_string(),
            format!("{STORE}: refused")
        );
        assert_eq!(
            Error::NotAKey.to_string(),
            format!("{STORE} keeps something that is not a publisher Web API key")
        );
    }

    #[test]
    fn a_key_is_kept_for_its_home_and_a_password_for_its_home_app_and_branch() {
        let key = Item::key(Path::new("/a/b"));
        assert_eq!((key.kind, key.label), ("web-api-key", LABEL));
        assert_eq!(key.account(), "/a/b");
        let password = Item::branch(Path::new("/a/b"), 480, "testing");
        assert_eq!(
            (password.kind, password.label),
            ("branch-password", BRANCH_LABEL)
        );
        assert_eq!(password.home, "/a/b");
        assert_eq!(password.account(), "/a/b:480:testing");
    }
}
