//! Just enough of the ELF header to tell a Linux program from a Linux library.
//!
//! A program needs its executable bit to start; a shared library does not. Both can be `ET_DYN`
//! (a position-independent program is), so the question is whether the file names an
//! interpreter, which only programs do, or is marked `DF_1_PIE`, which a static-pie program is
//! in place of naming one.

use std::io::{self, Read, Seek, SeekFrom};

const MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_INTERP: u32 = 3;
const PT_DYNAMIC: u32 = 2;
const DT_FLAGS_1: u64 = 0x6FFF_FFFB;
const DF_1_PIE: u64 = 0x0800_0000;
const DYNAMIC_LIMIT: u64 = 64 * 1024;

/// Answers whether `file` is an ELF program: one that is started, rather than loaded by another.
///
/// # Errors
///
/// Only when reading fails; a file that is not ELF, or is cut short, is simply not a program.
pub fn is_program<File>(file: &mut File) -> io::Result<bool>
where
    File: Read + Seek,
{
    let header = read_up_to(file, 64)?;
    let header = header.as_slice();
    if header.get(..4) != Some(&MAGIC[..]) {
        return Ok(false);
    }
    let Some(order) = Order::of(header.get(5).copied()) else {
        return Ok(false);
    };
    let wide = match header.get(4) {
        Some(1) => false,
        Some(2) => true,
        _ => return Ok(false),
    };
    let Some(kind) = order.u16(header, 16) else {
        return Ok(false);
    };
    if kind == ET_EXEC {
        return Ok(true);
    }
    if kind != ET_DYN {
        return Ok(false);
    }
    let fields = if wide {
        (
            order.u64(header, 32),
            order.u16(header, 54),
            order.u16(header, 56),
        )
    } else {
        (
            order.u32(header, 28).map(u64::from),
            order.u16(header, 42),
            order.u16(header, 44),
        )
    };
    let (Some(table), Some(entry_size), Some(count)) = fields else {
        return Ok(false);
    };
    if entry_size < 4 {
        return Ok(false);
    }
    let mut dynamic = None;
    for index in 0..u64::from(count) {
        let Some(at) = index
            .checked_mul(u64::from(entry_size))
            .and_then(|offset| offset.checked_add(table))
        else {
            return Ok(false);
        };
        // An absolute seek lands at `at` by definition, even past the end, where the short read
        // below is what says the table is not there.
        let _: u64 = file.seek(SeekFrom::Start(at))?;
        let segment = read_up_to(file, u64::from(entry_size))?;
        let Some(segment_kind) = order.u32(&segment, 0) else {
            return Ok(false);
        };
        if segment_kind == PT_INTERP {
            return Ok(true);
        }
        if segment_kind == PT_DYNAMIC {
            dynamic = if wide {
                order.u64(&segment, 8).zip(order.u64(&segment, 32))
            } else {
                order
                    .u32(&segment, 4)
                    .zip(order.u32(&segment, 16))
                    .map(|(offset, size)| (u64::from(offset), u64::from(size)))
            };
        }
    }
    // A program linked static-pie names no interpreter, as a library does not; it is told from
    // one by the flag the linker sets on programs alone.
    match dynamic {
        Some((offset, size)) => says_pie(file, order, wide, offset, size),
        None => Ok(false),
    }
}

/// Whether the dynamic section at `offset` sets `DF_1_PIE` in its `DT_FLAGS_1`. Only the first
/// 64 KiB is read, which is far more than any linker writes there.
fn says_pie<File>(
    file: &mut File,
    order: Order,
    wide: bool,
    offset: u64,
    size: u64,
) -> io::Result<bool>
where
    File: Read + Seek,
{
    let _: u64 = file.seek(SeekFrom::Start(offset))?;
    let section = read_up_to(file, size.min(DYNAMIC_LIMIT))?;
    let entry = if wide { 16 } else { 8 };
    let flags = section.chunks_exact(entry).find_map(|pair| {
        let (tag, value) = if wide {
            (order.u64(pair, 0)?, order.u64(pair, 8)?)
        } else {
            (
                u64::from(order.u32(pair, 0)?),
                u64::from(order.u32(pair, 4)?),
            )
        };
        (tag == DT_FLAGS_1).then_some(value)
    });
    Ok(flags.is_some_and(|flags| flags & DF_1_PIE != 0))
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

#[derive(Clone, Copy)]
enum Order {
    Little,
    Big,
}

impl Order {
    const fn of(byte: Option<u8>) -> Option<Self> {
        match byte {
            Some(1) => Some(Self::Little),
            Some(2) => Some(Self::Big),
            _ => None,
        }
    }

    fn bytes<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
        bytes.get(at..at.checked_add(N)?)?.try_into().ok()
    }

    fn u16(self, bytes: &[u8], at: usize) -> Option<u16> {
        let raw = Self::bytes(bytes, at)?;
        Some(match self {
            Self::Little => u16::from_le_bytes(raw),
            Self::Big => u16::from_be_bytes(raw),
        })
    }

    fn u32(self, bytes: &[u8], at: usize) -> Option<u32> {
        let raw = Self::bytes(bytes, at)?;
        Some(match self {
            Self::Little => u32::from_le_bytes(raw),
            Self::Big => u32::from_be_bytes(raw),
        })
    }

    fn u64(self, bytes: &[u8], at: usize) -> Option<u64> {
        let raw = Self::bytes(bytes, at)?;
        Some(match self {
            Self::Little => u64::from_le_bytes(raw),
            Self::Big => u64::from_be_bytes(raw),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Writes `value` at `at`, zero-filling up to it; fields are written in ascending order.
    fn put(bytes: &mut Vec<u8>, at: usize, value: &[u8]) {
        bytes.resize(at, 0);
        bytes.extend_from_slice(value);
    }

    /// A 64-bit little-endian ELF header of `kind` with one program header of `segment`.
    fn elf64(kind: u16, segment: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        put(&mut bytes, 0, &MAGIC);
        put(&mut bytes, 4, &[2, 1]);
        put(&mut bytes, 16, &kind.to_le_bytes());
        put(&mut bytes, 32, &64_u64.to_le_bytes());
        put(&mut bytes, 54, &56_u16.to_le_bytes());
        put(&mut bytes, 56, &1_u16.to_le_bytes());
        put(&mut bytes, 64, &segment.to_le_bytes());
        put(&mut bytes, 120, &[]);
        bytes
    }

    fn cut(mut bytes: Vec<u8>, length: usize) -> Vec<u8> {
        bytes.truncate(length);
        bytes
    }

    fn program(bytes: Vec<u8>) -> bool {
        is_program(&mut Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn a_position_independent_program_is_a_program() {
        assert!(program(elf64(ET_DYN, PT_INTERP)));
    }

    #[test]
    fn a_shared_library_is_not() {
        assert!(!program(elf64(ET_DYN, 1)));
    }

    /// A 64-bit little-endian `ET_DYN` file whose one segment is a dynamic section `size` bytes
    /// long at 120, holding `entries` from its start and `DT_FLAGS_1` of `flags` at `flags_at`.
    fn with_dynamic(flags: u64, flags_at: usize, size: u64) -> Vec<u8> {
        let mut bytes = elf64(ET_DYN, PT_DYNAMIC);
        let mut header = bytes.split_off(64);
        header.truncate(8);
        bytes.extend(header);
        put(&mut bytes, 72, &120_u64.to_le_bytes());
        put(&mut bytes, 96, &size.to_le_bytes());
        put(&mut bytes, 120, &[]);
        let at = |from: usize| flags_at.checked_add(from).unwrap();
        put(&mut bytes, at(120), &DT_FLAGS_1.to_le_bytes());
        put(&mut bytes, at(128), &flags.to_le_bytes());
        put(&mut bytes, at(136), &[0; 16]);
        bytes
    }

    #[test]
    fn a_static_pie_program_is_told_from_a_library_by_its_flag() {
        assert!(program(with_dynamic(DF_1_PIE | 1, 16, 48)), "static-pie");
        assert!(
            !program(with_dynamic(1, 16, 48)),
            "a library with other flags"
        );
        assert!(
            !program(with_dynamic(DF_1_PIE, 16, 16)),
            "a flag past the section's end"
        );
    }

    #[test]
    fn only_the_start_of_a_dynamic_section_is_read() {
        let at = usize::try_from(DYNAMIC_LIMIT).unwrap() + 16;
        assert!(
            !program(with_dynamic(DF_1_PIE, at, DYNAMIC_LIMIT * 2)),
            "a flag beyond the limit is not looked for"
        );
        let inside = usize::try_from(DYNAMIC_LIMIT - 32).unwrap();
        assert!(
            program(with_dynamic(DF_1_PIE, inside, DYNAMIC_LIMIT * 2)),
            "a flag just inside the limit is found"
        );
    }

    #[test]
    fn a_32_bit_static_pie_program_is_a_program() {
        let mut bytes = Vec::new();
        put(&mut bytes, 0, &MAGIC);
        put(&mut bytes, 4, &[1, 1]);
        put(&mut bytes, 16, &ET_DYN.to_le_bytes());
        put(&mut bytes, 28, &52_u32.to_le_bytes());
        put(&mut bytes, 42, &32_u16.to_le_bytes());
        put(&mut bytes, 44, &1_u16.to_le_bytes());
        put(&mut bytes, 52, &PT_DYNAMIC.to_le_bytes());
        put(&mut bytes, 56, &84_u32.to_le_bytes());
        put(&mut bytes, 68, &16_u32.to_le_bytes());
        put(
            &mut bytes,
            84,
            &u32::try_from(DT_FLAGS_1).unwrap().to_le_bytes(),
        );
        put(
            &mut bytes,
            88,
            &u32::try_from(DF_1_PIE).unwrap().to_le_bytes(),
        );
        put(&mut bytes, 92, &[0; 8]);
        assert!(program(bytes));
    }

    #[test]
    fn a_fixed_address_program_is_a_program() {
        assert!(program(elf64(ET_EXEC, 1)));
    }

    #[test]
    fn a_32_bit_big_endian_program_is_a_program() {
        let mut bytes = Vec::new();
        put(&mut bytes, 0, &MAGIC);
        put(&mut bytes, 4, &[1, 2]);
        put(&mut bytes, 16, &ET_DYN.to_be_bytes());
        put(&mut bytes, 28, &52_u32.to_be_bytes());
        put(&mut bytes, 42, &32_u16.to_be_bytes());
        put(&mut bytes, 44, &1_u16.to_be_bytes());
        put(&mut bytes, 52, &PT_INTERP.to_be_bytes());
        put(&mut bytes, 84, &[]);
        assert!(program(bytes));
    }

    #[test]
    fn a_64_bit_big_endian_program_is_a_program() {
        let mut bytes = Vec::new();
        put(&mut bytes, 0, &MAGIC);
        put(&mut bytes, 4, &[2, 2]);
        put(&mut bytes, 16, &ET_DYN.to_be_bytes());
        put(&mut bytes, 32, &64_u64.to_be_bytes());
        put(&mut bytes, 54, &56_u16.to_be_bytes());
        put(&mut bytes, 56, &1_u16.to_be_bytes());
        put(&mut bytes, 64, &PT_INTERP.to_be_bytes());
        put(&mut bytes, 120, &[]);
        assert!(program(bytes));
    }

    /// `elf64(ET_DYN, PT_INTERP)` with the bytes at `at` replaced by `value`.
    fn altered(at: usize, value: &[u8]) -> Vec<u8> {
        let mut bytes = elf64(ET_DYN, PT_INTERP);
        let tail = bytes.split_off(at);
        bytes.extend_from_slice(value);
        bytes.extend(tail.into_iter().skip(value.len()));
        bytes
    }

    #[test]
    fn a_header_that_does_not_say_program_is_not_one() {
        assert!(
            program(altered(0, &[])),
            "the unaltered header is a program"
        );
        assert!(
            program(altered(54, &4_u16.to_le_bytes())),
            "program headers just big enough to hold their type are enough"
        );
        for (what, at, value) in [
            ("an unknown class", 4, &[3][..]),
            ("an unknown byte order", 5, &[0][..]),
            ("a relocatable object", 16, &1_u16.to_le_bytes()[..]),
            (
                "program headers smaller than a type",
                54,
                &2_u16.to_le_bytes()[..],
            ),
            (
                "a table past the end of the file",
                32,
                &4096_u64.to_le_bytes()[..],
            ),
            (
                "a table at the end of the address space",
                32,
                &u64::MAX.to_le_bytes()[..],
            ),
        ] {
            assert!(!program(altered(at, value)), "{what}");
        }
    }

    #[test]
    fn anything_else_or_cut_short_is_not() {
        assert!(!program(b"MZ\x90\x00 a Windows program".to_vec()));
        assert!(!program(Vec::new()));
        assert!(!program(cut(elf64(ET_DYN, PT_INTERP), 66)));
        assert!(!program(cut(elf64(ET_DYN, PT_INTERP), 40)));
    }
}
