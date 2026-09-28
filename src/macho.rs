//! Just enough of the Mach-O header to tell a macOS program from a macOS library.
//!
//! A program needs its executable bit to start; a library or bundle does not. The header says
//! which it is. A universal file holds one slice per architecture, and its first slice speaks for
//! the rest; it shares its opening bytes with a Java class file, whose version stands where the
//! number of slices would, and is far larger than any universal file's.

use std::io::{self, Read, Seek, SeekFrom};

const MH_EXECUTE: u32 = 2;
const FAT_MAGIC: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBE];
const FAT_MAGIC_64: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBF];
/// More slices than this is a Java class file; `file(1)` draws the same line.
const MOST_SLICES: u32 = 20;

/// Answers whether `file` is a Mach-O program, one that is started rather than loaded.
///
/// # Errors
///
/// Only when reading fails; a file that is not Mach-O, or is cut short, is simply not a program.
pub fn is_program<File>(file: &mut File) -> io::Result<bool>
where
    File: Read + Seek,
{
    let header = read_up_to(file, 24)?;
    let wide = match header.get(..4) {
        Some(magic) if magic == FAT_MAGIC => false,
        Some(magic) if magic == FAT_MAGIC_64 => true,
        _ => return Ok(thin_program(&header)),
    };
    let Some(slices) = be_u32(&header, 4) else {
        return Ok(false);
    };
    if slices == 0 || slices > MOST_SLICES {
        return Ok(false);
    }
    // The first slice's entry follows the eight-byte header: its CPU type and subtype, then
    // where the slice starts.
    let start = if wide {
        bytes::<8>(&header, 16).map(u64::from_be_bytes)
    } else {
        be_u32(&header, 16).map(u64::from)
    };
    let Some(start) = start else {
        return Ok(false);
    };
    let _: u64 = file.seek(SeekFrom::Start(start))?;
    let slice = read_up_to(file, 16)?;
    Ok(thin_program(&slice))
}

/// Whether `header` opens a single-architecture Mach-O file of the executable kind.
fn thin_program(header: &[u8]) -> bool {
    let Some(magic) = header.get(..4) else {
        return false;
    };
    let kind = match magic {
        [0xFE, 0xED, 0xFA, 0xCE | 0xCF] => be_u32(header, 12),
        [0xCE | 0xCF, 0xFA, 0xED, 0xFE] => le_u32(header, 12),
        _ => None,
    };
    kind == Some(MH_EXECUTE)
}

fn bytes<const N: usize>(from: &[u8], at: usize) -> Option<[u8; N]> {
    from.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn be_u32(from: &[u8], at: usize) -> Option<u32> {
    bytes::<4>(from, at).map(u32::from_be_bytes)
}

fn le_u32(from: &[u8], at: usize) -> Option<u32> {
    bytes::<4>(from, at).map(u32::from_le_bytes)
}

/// Up to `limit` bytes from where `file` stands, fewer only at its end.
fn read_up_to<File>(file: &mut File, limit: u64) -> io::Result<Vec<u8>>
where
    File: Read,
{
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map(|_| bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const MH_DYLIB: u32 = 6;

    fn program(bytes: &[u8]) -> bool {
        is_program(&mut Cursor::new(bytes.to_vec())).unwrap()
    }

    /// A 64-bit little-endian header of `kind`, as Apple silicon and Intel Macs write it.
    fn thin(kind: u32) -> Vec<u8> {
        let mut bytes = vec![0xCF, 0xFA, 0xED, 0xFE];
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes
    }

    /// A universal header of `slices` slices whose first starts at 64 and holds `slice`.
    fn universal(magic: [u8; 4], slices: u32, slice: &[u8]) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(&slices.to_be_bytes());
        bytes.extend_from_slice(&[0; 8]);
        if magic == FAT_MAGIC_64 {
            bytes.extend_from_slice(&64_u64.to_be_bytes());
        } else {
            bytes.extend_from_slice(&64_u32.to_be_bytes());
        }
        bytes.resize(64, 0);
        bytes.extend_from_slice(slice);
        bytes
    }

    #[test]
    fn a_program_is_one_and_a_library_is_not() {
        assert!(program(&thin(MH_EXECUTE)));
        assert!(!program(&thin(MH_DYLIB)));
    }

    #[test]
    fn either_byte_order_and_width_is_read() {
        for magic in [[0xFE, 0xED, 0xFA, 0xCE], [0xFE, 0xED, 0xFA, 0xCF]] {
            let mut bytes = magic.to_vec();
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(&MH_EXECUTE.to_be_bytes());
            assert!(program(&bytes), "{magic:x?}");
        }
        let mut small = vec![0xCE, 0xFA, 0xED, 0xFE];
        small.extend_from_slice(&[0; 8]);
        small.extend_from_slice(&MH_EXECUTE.to_le_bytes());
        assert!(program(&small));
    }

    #[test]
    fn a_universal_file_is_what_its_first_slice_is() {
        for magic in [FAT_MAGIC, FAT_MAGIC_64] {
            assert!(
                program(&universal(magic, 2, &thin(MH_EXECUTE))),
                "{magic:x?}"
            );
            assert!(
                !program(&universal(magic, 2, &thin(MH_DYLIB))),
                "{magic:x?}"
            );
        }
    }

    #[test]
    fn a_java_class_file_is_not_a_program() {
        // A class file's minor and major version, 0 and 52, stand where the slice count would.
        assert!(!program(&universal(FAT_MAGIC, 52, &thin(MH_EXECUTE))));
        assert!(program(&universal(
            FAT_MAGIC,
            MOST_SLICES,
            &thin(MH_EXECUTE)
        )));
        assert!(!program(&universal(
            FAT_MAGIC,
            MOST_SLICES + 1,
            &thin(MH_EXECUTE)
        )));
        assert!(!program(&universal(FAT_MAGIC, 0, &thin(MH_EXECUTE))));
    }

    #[test]
    fn a_file_cut_short_or_of_another_kind_is_not_a_program() {
        let cut = |mut bytes: Vec<u8>, length: usize| {
            bytes.truncate(length);
            bytes
        };
        assert!(!program(&[]));
        assert!(!program(b"MZ\x90\x00 a Windows program"));
        assert!(!program(&cut(thin(MH_EXECUTE), 12)));
        assert!(!program(&universal(FAT_MAGIC, 1, &[])));
        assert!(!program(&FAT_MAGIC));
        assert!(!program(&cut(universal(FAT_MAGIC_64, 1, &[]), 20)));
    }
}
