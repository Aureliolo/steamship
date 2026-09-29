//! What kind of file a file is, from its first bytes.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

/// How a program starts on any of the three systems: a Windows executable, an ELF or Mach-O file
/// (32-bit, 64-bit and universal, in either byte order), or a script.
const PROGRAM_STARTS: [&[u8]; 8] = [
    b"MZ",
    b"#!",
    b"\x7fELF",
    b"\xfe\xed\xfa\xce",
    b"\xfe\xed\xfa\xcf",
    b"\xce\xfa\xed\xfe",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
];

/// Whether `path` starts the way a program does.
///
/// # Errors
///
/// When `path` cannot be read.
pub fn looks_like_program(path: &Path) -> io::Result<bool> {
    let start = start(path)?;
    Ok(PROGRAM_STARTS.iter().any(|magic| start.starts_with(magic)))
}

/// Whether `path` starts the way a Windows executable does.
///
/// # Errors
///
/// When `path` cannot be read.
pub fn looks_like_windows_program(path: &Path) -> io::Result<bool> {
    Ok(start(path)?.starts_with(b"MZ"))
}

fn start(path: &Path) -> io::Result<Vec<u8>> {
    let mut start = Vec::new();
    let _: usize = File::open(path)?.take(4).read_to_end(&mut start)?;
    Ok(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn knows_a_program_by_how_it_starts() {
        let folder = tempfile::tempdir().unwrap();
        let cases: [(&[u8], bool); 11] = [
            (b"MZ\x90\x00", true),
            (b"#!/bin/sh", true),
            (b"\x7fELF\x02", true),
            (b"\xfe\xed\xfa\xce", true),
            (b"\xfe\xed\xfa\xcf", true),
            (b"\xce\xfa\xed\xfe", true),
            (b"\xcf\xfa\xed\xfe", true),
            (b"\xca\xfe\xba\xbe", true),
            (b"text", false),
            (b"M", false),
            (b"", false),
        ];
        for (start, program) in cases {
            let path = folder.path().join("file");
            fs::write(&path, start).unwrap();
            assert_eq!(looks_like_program(&path).unwrap(), program, "{start:?}");
        }
        assert!(
            looks_like_program(&folder.path().join("absent"))
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn knows_a_windows_program_from_the_others() {
        let folder = tempfile::tempdir().unwrap();
        let cases: [(&[u8], bool); 4] = [
            (b"MZ\x90\x00", true),
            (b"\x7fELF\x02", false),
            (b"M", false),
            (b"", false),
        ];
        for (start, windows) in cases {
            let path = folder.path().join("file");
            fs::write(&path, start).unwrap();
            assert_eq!(
                looks_like_windows_program(&path).unwrap(),
                windows,
                "{start:?}"
            );
        }
        assert!(
            looks_like_windows_program(&folder.path().join("absent"))
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        );
    }
}
