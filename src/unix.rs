//! What only a Unix file system can answer or do. Kept in a module of its own so that everything
//! here is compiled, tested and mutation-tested on the systems where it runs.

use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt as _, symlink as make_symlink};
use std::path::Path;

use crate::{elf, magic};

/// Says what is wrong when `file` is a Linux program that nobody may execute.
#[must_use]
pub fn missing_executable_bit(file: &Path) -> Option<String> {
    let mut opened = match fs::File::open(file) {
        Ok(opened) => opened,
        Err(error) => return Some(format!("cannot be read: {error}")),
    };
    match elf::is_program(&mut opened) {
        Ok(false) => None,
        Ok(true) => match opened.metadata() {
            Ok(metadata) if metadata.permissions().mode() & 0o111 == 0 => {
                Some("is a Linux program without its executable bit; run chmod +x on it".to_owned())
            }
            Ok(_) => None,
            Err(error) => Some(format!("cannot be read: {error}")),
        },
        Err(error) => Some(format!("cannot be read: {error}")),
    }
}

/// Valve's zips carry no permissions, so a program is made executable here, as steamcmd's own
/// installer does, and everything else readable only.
///
/// # Errors
///
/// When `path` cannot be read, or the file system refuses the change.
pub fn mark_if_program(path: &Path) -> io::Result<()> {
    let mode = if magic::looks_like_program(path)? {
        0o755
    } else {
        0o644
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Makes `link` a symbolic link to `target`, which is written as given.
///
/// # Errors
///
/// When the file system refuses, or `link` exists.
pub fn make_link(target: &str, link: &Path) -> io::Result<()> {
    make_symlink(target, link)
}
