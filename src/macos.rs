//! What only macOS does: the Keychain, which keeps the Web API key there. Kept in a module of its
//! own so that everything here is compiled, tested and mutation-tested on the system where it
//! runs.

use security_framework::base::Error as Status;
use security_framework::item::{ItemClass, ItemSearchOptions};
use security_framework::passwords::{
    PasswordOptions, delete_generic_password, get_generic_password, set_generic_password_options,
};
use zeroize::Zeroizing;

use crate::keychain::{Error, LABEL};

/// The Keychain's errSecItemNotFound.
const NOT_FOUND: i32 = -25_300;

/// errSecNoDefaultKeychain and errSecInteractionNotAllowed: no Keychain to use, as when no one is
/// logged in at the Mac.
const UNAVAILABLE: [i32; 2] = [-25_307, -25_308];

fn failed(status: &Status) -> Error {
    if UNAVAILABLE.contains(&status.code()) {
        Error::Unavailable(status.to_string())
    } else {
        Error::Failed(status.to_string())
    }
}

/// Whether the Keychain keeps a secret for the steamship home `account`.
///
/// # Errors
///
/// When the Keychain cannot be searched.
pub fn has_secret(account: &str) -> Result<bool, Error> {
    // Attributes alone, never the secret, so the Keychain has nothing to ask permission for.
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(LABEL)
        .account(account)
        .load_attributes(true)
        .limit(1)
        .search();
    match found {
        Ok(items) => Ok(!items.is_empty()),
        Err(status) if status.code() == NOT_FOUND => Ok(false),
        Err(status) => Err(failed(&status)),
    }
}

/// The secret the Keychain keeps for `account`, if it keeps one.
///
/// # Errors
///
/// When the Keychain cannot be read.
pub fn kept_secret(account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    match get_generic_password(LABEL, account) {
        Ok(secret) => Ok(Some(Zeroizing::new(secret))),
        Err(status) if status.code() == NOT_FOUND => Ok(None),
        Err(status) => Err(failed(&status)),
    }
}

/// Keeps `secret` for `account`, in place of any kept before.
///
/// # Errors
///
/// When the Keychain refuses it.
pub fn keep_secret(account: &str, secret: &[u8]) -> Result<(), Error> {
    let mut options = PasswordOptions::new_generic_password(LABEL, account);
    options.set_label(LABEL);
    // Kept on this Mac alone, never synchronised to iCloud.
    options.set_access_synchronized(Some(false));
    set_generic_password_options(secret, options).map_err(|status| failed(&status))
}

/// Removes the secret kept for `account`, and says whether there was one.
///
/// # Errors
///
/// When the Keychain refuses.
pub fn forget_secret(account: &str) -> Result<bool, Error> {
    match delete_generic_password(LABEL, account) {
        Ok(()) => Ok(true),
        Err(status) if status.code() == NOT_FOUND => Ok(false),
        Err(status) => Err(failed(&status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_keychain_to_use_is_no_store_and_anything_else_a_failure() {
        for code in UNAVAILABLE {
            assert!(
                matches!(failed(&Status::from_code(code)), Error::Unavailable(_)),
                "{code}"
            );
        }
        assert!(matches!(
            failed(&Status::from_code(NOT_FOUND)),
            Error::Failed(_)
        ));
    }
}
