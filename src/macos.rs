//! What only macOS does: the Keychain, which keeps the Web API key there. Kept in a module of its
//! own so that everything here is compiled, tested and mutation-tested on the system where it
//! runs.

use security_framework::base::Error as Status;
use security_framework::item::{ItemClass, ItemSearchOptions};
use security_framework::passwords::{
    PasswordOptions, delete_generic_password, get_generic_password, set_generic_password_options,
};
use zeroize::Zeroizing;

use crate::keychain::{Error, Item};

/// The Keychain's errSecItemNotFound.
const NOT_FOUND: i32 = -25_300;

/// errSecNoDefaultKeychain and errSecInteractionNotAllowed: no Keychain to use, as when no one is
/// logged in at the Mac.
const UNAVAILABLE: [i32; 2] = [-25_307, -25_308];

fn failed(status: Status) -> Error {
    if UNAVAILABLE.contains(&status.code()) {
        Error::Unavailable(status.to_string())
    } else {
        Error::Failed(status.to_string())
    }
}

/// What the Keychain answered, with `missing` standing for an item it has not got.
fn answered<Found>(answer: Result<Found, Status>, missing: Found) -> Result<Found, Error> {
    match answer {
        Ok(found) => Ok(found),
        Err(status) if status.code() == NOT_FOUND => Ok(missing),
        Err(status) => Err(failed(status)),
    }
}

/// Whether the Keychain keeps `item`: a generic password whose service is the item's label and
/// whose account is the item's own name.
///
/// # Errors
///
/// When the Keychain cannot be searched.
pub fn has_secret(item: &Item) -> Result<bool, Error> {
    // Attributes alone, never the secret, so the Keychain has nothing to ask permission for.
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(item.label)
        .account(&item.account())
        .load_attributes(true)
        .limit(1)
        .search();
    answered(found.map(|items| !items.is_empty()), false)
}

/// The secret the Keychain keeps as `item`, if it keeps one.
///
/// # Errors
///
/// When the Keychain cannot be read.
pub fn kept_secret(item: &Item) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    answered(
        get_generic_password(item.label, &item.account())
            .map(|secret| Some(Zeroizing::new(secret))),
        None,
    )
}

/// Keeps `secret` as `item`, in place of any kept before.
///
/// # Errors
///
/// When the Keychain refuses it.
pub fn keep_secret(item: &Item, secret: &[u8]) -> Result<(), Error> {
    let mut options = PasswordOptions::new_generic_password(item.label, &item.account());
    options.set_label(item.label);
    // Kept on this Mac alone, never synchronised to iCloud.
    options.set_access_synchronized(Some(false));
    set_generic_password_options(secret, options).map_err(failed)
}

/// Removes `item`, and says whether it was kept.
///
/// # Errors
///
/// When the Keychain refuses.
pub fn forget_secret(item: &Item) -> Result<bool, Error> {
    answered(
        delete_generic_password(item.label, &item.account()).map(|()| true),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The codes as Apple's Security framework defines them, written out so that a wrong constant
    /// shows.
    #[test]
    fn no_keychain_to_use_is_no_store_and_anything_else_a_failure() {
        for code in [-25_307_i32, -25_308_i32] {
            assert!(
                matches!(failed(Status::from_code(code)), Error::Unavailable(_)),
                "{code}"
            );
        }
        for code in [-25_300_i32, -25_299_i32, 25_307_i32] {
            assert!(
                matches!(failed(Status::from_code(code)), Error::Failed(_)),
                "{code}"
            );
        }
    }

    #[test]
    fn an_item_not_found_is_what_is_missing_and_any_other_answer_an_error() {
        assert_eq!(answered(Ok(true), false), Ok(true));
        assert_eq!(answered(Err(Status::from_code(-25_300)), false), Ok(false));
        assert!(matches!(
            answered(Err(Status::from_code(-25_299)), false),
            Err(Error::Failed(_))
        ));
        assert!(matches!(
            answered(Err(Status::from_code(-25_308)), None::<u8>),
            Err(Error::Unavailable(_))
        ));
    }
}
