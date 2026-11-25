//! The system's own credential store, where steamship keeps the Web API key, and the passwords of
//! the beta branches it fetches, between runs.
//!
//! That is the Credential Manager on Windows, the Keychain on macOS, and on Linux the Secret
//! Service that GNOME Keyring and `KWallet` provide. One key is kept for each steamship home, as
//! the Steam login is, so that `logout` forgets both and a test with a home of its own never meets
//! a person's real key. A branch's password is kept for the home, the app and the branch.

#[cfg(feature = "fake-steamcmd")]
use std::env;
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
    "after each steamship upgrade, macOS asks once whether it may use the key, and Always Allow",
    "holds until the next upgrade",
];
#[cfg(not(target_os = "macos"))]
pub const KEPT_NOTE: &[&str] = &[];

/// What the key is called where the store shows it.
pub const LABEL: &str = "steamship Web API key";

/// What a branch's password is called where the store shows it.
pub const BRANCH_LABEL: &str = "steamship branch password";

/// What kind an item is when it holds the Web API key.
pub const KEY_KIND: &str = "web-api-key";

/// What kind an item is when it holds a branch's password.
pub const BRANCH_KIND: &str = "branch-password";

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
            kind: KEY_KIND,
            label: LABEL,
            home: home.display().to_string(),
            within: Vec::new(),
        }
    }

    /// The password of `app`'s `branch` kept for `home`.
    #[must_use]
    pub fn branch(home: &Path, app: u32, branch: &str) -> Self {
        Self {
            kind: BRANCH_KIND,
            label: BRANCH_LABEL,
            home: home.display().to_string(),
            within: vec![("app", app.to_string()), ("branch", branch.to_owned())],
        }
    }

    /// The branch password the store lists for `home` with these `app` and `branch` texts, when
    /// they are exactly what [`Item::branch`] writes. Anything else is not steamship's for this
    /// home, so forgetting it would take another home's item.
    #[must_use]
    pub fn listed_branch(home: &Path, app: &str, branch: &str) -> Option<Self> {
        let number: u32 = app.parse().ok()?;
        (number.to_string() == app && !branch.is_empty() && !branch.contains(':'))
            .then(|| Self::branch(home, number, branch))
    }

    /// The branch password whose account, as [`Item::account`] writes it, is `account`, when it is
    /// one kept for `home`. A home that only starts the same, such as `/a` beside `/a b`, has
    /// none.
    #[must_use]
    pub fn branch_in_account(home: &Path, account: &str) -> Option<Self> {
        Self::listed(BRANCH_KIND, account).filter(|item| same_home(&item.home, home))
    }

    /// The item of this `kind` whose account, as [`Item::account`] writes it, is `account`, when
    /// steamship writes such an account. The home is whatever the account names: a branch ends in
    /// an app and a branch that hold no colon, and a home may hold some.
    #[must_use]
    pub fn listed(kind: &str, account: &str) -> Option<Self> {
        match kind {
            KEY_KIND => (!account.is_empty()).then(|| Self::key(Path::new(account))),
            BRANCH_KIND => {
                let mut parts = account.rsplitn(3, ':');
                let branch = parts.next()?;
                let app = parts.next()?;
                let home = parts.next().filter(|home| !home.is_empty())?;
                Self::listed_branch(Path::new(home), app, branch)
            }
            _ => None,
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
    /// The store took a change but had not shown it after a while, so whether it holds is not
    /// known.
    Unsettled(String),
    /// What the store keeps is not a Web API key.
    NotAKey,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(formatter, "no credential store: {why}"),
            Self::Failed(why) | Self::Unsettled(why) => write!(formatter, "{STORE}: {why}"),
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

#[cfg(windows)]
use crate::windows::same_home;

/// Whether `listed`, a home as the store lists it, is `home`, letter for letter, as the
/// Keychain and the Secret Service match it.
#[cfg(not(windows))]
#[must_use]
pub fn same_home(listed: &str, home: &Path) -> bool {
    listed == home.display().to_string()
}

/// The branch passwords kept for `home`, as items, found out without asking to unlock anything.
///
/// # Errors
///
/// When the store cannot be reached or read.
pub fn branch_passwords(home: &Path) -> Result<Vec<Item>, Error> {
    platform::branch_items(home)
}

/// Forgets every branch password kept for `home`, and says how many there were. One the store
/// refuses does not stop the others.
///
/// # Errors
///
/// When the store cannot be reached or read, or refuses one: the first refusal.
pub fn forget_branch_passwords(home: &Path) -> Result<usize, Error> {
    forget_each(branch_passwords(home)?, forget_listed)
}

/// Forgets a branch password listed for a home. A test build refuses each while
/// `STEAMSHIP_TEST_REFUSE_PASSWORDS` is set: the real store refuses only along with the key, so
/// a refusal of the passwords alone cannot otherwise be tested.
fn forget_listed(item: &Item) -> Result<bool, Error> {
    #[cfg(feature = "fake-steamcmd")]
    if env::var_os("STEAMSHIP_TEST_REFUSE_PASSWORDS").is_some() {
        return Err(Error::Failed(format!(
            "it refused to remove {}",
            item.account()
        )));
    }
    platform::forget_secret(item)
}

/// Forgets each of `items` with `forget`, and counts those that were kept. A refusal does not stop
/// the rest.
fn forget_each(
    items: Vec<Item>,
    mut forget: impl FnMut(&Item) -> Result<bool, Error>,
) -> Result<usize, Error> {
    let mut forgotten = 0_usize;
    let mut refused = None;
    for item in items {
        match forget(&item) {
            Ok(true) => forgotten = forgotten.saturating_add(1),
            Ok(false) => {}
            Err(error) => {
                if refused.is_none() {
                    refused = Some(error);
                }
            }
        }
    }
    refused.map_or(Ok(forgotten), Err)
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
            Error::Unsettled("not shown yet".to_owned()).to_string(),
            format!("{STORE}: not shown yet")
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

    #[test]
    fn a_listed_account_is_a_branch_password_of_the_home_only_when_it_is_exactly_one() {
        let home = Path::new("/a");
        assert_eq!(
            Item::branch_in_account(home, "/a:480:testing"),
            Some(Item::branch(home, 480, "testing"))
        );
        for account in [
            "/a b:480:testing",
            "/ab:480:testing",
            "/a:1000:friends:480:testing",
            "/a:0480:testing",
            "/a:+480:testing",
            "/a:480:",
            "/a:480",
            "/a:x:testing",
            "/a:99999999999:testing",
            "/a",
            "",
        ] {
            assert_eq!(Item::branch_in_account(home, account), None, "{account}");
        }
        assert_eq!(
            Item::branch_in_account(Path::new("/a:1000:friends"), "/a:1000:friends:480:testing"),
            Some(Item::branch(Path::new("/a:1000:friends"), 480, "testing"))
        );
    }

    #[test]
    fn a_listed_item_is_read_back_into_the_home_it_was_kept_for() {
        let home = Path::new("C:\\Temp\\a:1000");
        for item in [Item::key(home), Item::branch(home, 480, "testing")] {
            assert_eq!(Item::listed(item.kind, &item.account()), Some(item));
        }
        for (kind, account) in [
            ("web-api-key", ""),
            ("branch-password", ":480:testing"),
            ("branch-password", "480:testing"),
            ("branch-password", "/a:480:"),
            ("other", "/a"),
        ] {
            assert_eq!(Item::listed(kind, account), None, "{kind} {account}");
        }
    }

    #[test]
    fn a_refusal_while_forgetting_leaves_the_rest_forgotten_and_is_the_error() {
        let home = Path::new("/a");
        let items: Vec<Item> = [(480, "testing"), (480, "staging"), (1000, "friends")]
            .iter()
            .map(|&(app, branch)| Item::branch(home, app, branch))
            .collect();
        let mut tried = Vec::new();
        let answer = forget_each(items.clone(), |item| {
            tried.push(item.account());
            if item.account().ends_with("staging") {
                Err(Error::Failed("refused".to_owned()))
            } else {
                Ok(true)
            }
        });
        assert_eq!(answer, Err(Error::Failed("refused".to_owned())));
        assert_eq!(tried.len(), 3);
        assert_eq!(forget_each(items, |_| Ok(true)), Ok(3));
        assert_eq!(forget_each(Vec::new(), |_| Ok(true)), Ok(0));
    }

    #[test]
    fn a_listed_password_already_gone_is_not_counted_as_forgotten() {
        let home = Path::new("/a");
        let items = vec![
            Item::branch(home, 480, "testing"),
            Item::branch(home, 480, "staging"),
            Item::branch(home, 1000, "friends"),
        ];
        let answer = forget_each(items, |item| Ok(!item.account().ends_with("staging")));
        assert_eq!(answer, Ok(2));
    }

    #[test]
    fn a_listed_home_in_another_letter_case_is_the_same_only_where_the_store_ignores_case() {
        let home = Path::new("C:\\Temp\\Home");
        let listed = Item::branch_in_account(home, "c:\\temp\\HOME:480:testing");
        if cfg!(windows) {
            assert_eq!(
                listed,
                Some(Item::branch(Path::new("c:\\temp\\HOME"), 480, "testing"))
            );
        } else {
            assert_eq!(listed, None);
        }
        assert!(same_home("C:\\Temp\\Home", home));
        assert!(!same_home("C:\\Temp\\Homes", home));
        assert!(!same_home("C:\\Temp\\Hom", home));
        assert!(!same_home("C:\\Temp\\Hose", home));
    }
}
