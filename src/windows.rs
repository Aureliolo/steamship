//! What only Windows does. Kept in a module of its own so that everything here is compiled,
//! tested and mutation-tested on the system where it runs.

use std::io;
use std::path::Path;

/// Windows has no executable bit, so there is nothing to mark.
///
/// # Errors
///
/// Never; the signature is the one every system shares.
pub const fn mark_if_program(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Only macOS packages hold links, and they are only ever unpacked on macOS, so a link in a
/// package for Windows is refused rather than guessed at.
///
/// # Errors
///
/// Always.
pub fn make_link(_: &str, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "a package for this system holds a link",
    ))
}
