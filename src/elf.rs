//! Just enough of the ELF header to tell a Linux program from a Linux library.
//!
//! A program needs its executable bit to start; a shared library does not. Both can be `ET_DYN`
//! (a position-independent program is), so the question is whether the file names an
//! interpreter, which only programs do.

use std::io::{self, Read, Seek, SeekFrom};

const MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_INTERP: u32 = 3;

/// Answers whether `file` is an ELF program: one that is started, rather than loaded by another.
///
/// # Errors
///
/// Only when reading fails; a file that is not ELF, or is cut short, is simply not a program.
pub fn is_program(file: &mut (impl Read + Seek)) -> io::Result<bool> {
    let mut header = [0u8; 64];
    let length = read_up_to(file, &mut header)?;
    let header = &header[..length];
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
    for index in 0..u64::from(count) {
        let Some(at) = index
            .checked_mul(u64::from(entry_size))
            .and_then(|o| o.checked_add(table))
        else {
            return Ok(false);
        };
        file.seek(SeekFrom::Start(at))?;
        let mut kind = [0u8; 4];
        if read_up_to(file, &mut kind)? < 4 {
            return Ok(false);
        }
        if order.u32(&kind, 0) == Some(PT_INTERP) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_up_to(file: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
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

    /// A 64-bit little-endian ELF header of `kind` with one program header of `segment`.
    fn elf64(kind: u16, segment: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 64 + 56];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[16..18].copy_from_slice(&kind.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        bytes[64..68].copy_from_slice(&segment.to_le_bytes());
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

    #[test]
    fn a_fixed_address_program_is_a_program() {
        assert!(program(elf64(ET_EXEC, 1)));
    }

    #[test]
    fn a_32_bit_big_endian_program_is_a_program() {
        let mut bytes = vec![0u8; 52 + 32];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4] = 1;
        bytes[5] = 2;
        bytes[16..18].copy_from_slice(&ET_DYN.to_be_bytes());
        bytes[28..32].copy_from_slice(&52u32.to_be_bytes());
        bytes[42..44].copy_from_slice(&32u16.to_be_bytes());
        bytes[44..46].copy_from_slice(&1u16.to_be_bytes());
        bytes[52..56].copy_from_slice(&PT_INTERP.to_be_bytes());
        assert!(program(bytes));
    }

    #[test]
    fn anything_else_or_cut_short_is_not() {
        assert!(!program(b"MZ\x90\x00 a Windows program".to_vec()));
        assert!(!program(Vec::new()));
        assert!(!program(elf64(ET_DYN, PT_INTERP)[..66].to_vec()));
        assert!(!program(elf64(ET_DYN, PT_INTERP)[..40].to_vec()));
    }
}
