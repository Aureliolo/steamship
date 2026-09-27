//! The system's real credential store, as `login --web-api-key`, `builds` and `logout` use it.
//! Each test keeps its key for a home of its own, so it never meets a person's real key, and
//! forgets it again.

use steamship::keychain;
#[cfg(target_os = "macos")]
use steamship::macos as platform;
#[cfg(target_os = "linux")]
use steamship::secret_service as platform;
use steamship::webapi::Key;
#[cfg(windows)]
use steamship::windows as platform;

const FIRST: &str = "0123456789ABCDEF0123456789ABCDEF";
const SECOND: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn a_key_is_kept_replaced_read_back_and_forgotten_for_its_home_alone() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let home = home.path();
    assert!(!keychain::has(home).unwrap());
    assert!(keychain::kept(home).unwrap().is_none());
    assert!(!keychain::forget(home).unwrap());

    keychain::keep(home, &Key::parse(FIRST).unwrap()).unwrap();
    assert!(keychain::has(home).unwrap());
    assert_eq!(keychain::kept(home).unwrap().unwrap().text(), FIRST);
    assert!(
        !keychain::has(other.path()).unwrap(),
        "another home keeps a key of its own"
    );

    keychain::keep(home, &Key::parse(SECOND).unwrap()).unwrap();
    assert_eq!(keychain::kept(home).unwrap().unwrap().text(), SECOND);

    assert!(keychain::forget(home).unwrap());
    assert!(!keychain::has(home).unwrap());
    assert!(keychain::kept(home).unwrap().is_none());
    assert!(!keychain::forget(home).unwrap());
}

#[test]
fn what_is_kept_that_is_not_a_key_is_refused_not_used() {
    let home = tempfile::tempdir().unwrap();
    let account = home.path().display().to_string();
    for kept in [&b"not a key"[..], &[0xff, 0xfe][..]] {
        platform::keep_secret(&account, kept).unwrap();
        assert_eq!(
            keychain::kept(home.path()).unwrap_err(),
            keychain::Error::NotAKey
        );
    }
    assert!(keychain::forget(home.path()).unwrap());
}
