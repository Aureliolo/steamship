//! SHA-256 of whatever passes through a writer, so a file is hashed as it is copied rather than
//! read twice.

use std::fmt::Write as _;
use std::io::{self, Read, Write};

use sha2::{Digest as _, Sha256};

/// Writes to `inner` and hashes the same bytes.
#[derive(Debug)]
pub struct Hashing<Inner> {
    inner: Inner,
    hasher: Sha256,
}

impl<Inner> Hashing<Inner> {
    pub fn new(inner: Inner) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// The SHA-256 of everything written, and the writer it went to.
    pub fn finish(self) -> ([u8; 32], Inner) {
        (self.hasher.finalize().into(), self.inner)
    }
}

impl<Inner> Write for Hashing<Inner>
where
    Inner: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write_all(buf)?;
        self.hasher.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The SHA-256 of everything `reader` yields.
///
/// # Errors
///
/// When reading fails.
pub fn sha256<Reader>(reader: &mut Reader) -> io::Result<[u8; 32]>
where
    Reader: Read,
{
    let mut hashing = Hashing::new(io::sink());
    let _: u64 = io::copy(reader, &mut hashing)?;
    Ok(hashing.finish().0)
}

/// The lower-case hexadecimal digits of `digest`.
#[must_use]
pub fn hex(digest: &[u8]) -> String {
    // Writing to a String cannot fail, so the empty fallback is never taken.
    digest
        .iter()
        .try_fold(String::new(), |mut out, byte| {
            write!(out, "{byte:02x}").map(|()| out)
        })
        .unwrap_or_default()
}

/// Exactly 64 hexadecimal digits, as 32 bytes.
#[must_use]
pub fn from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    bytes_from_hex(text)?.try_into().ok()
}

/// Hexadecimal digits in pairs, as the bytes they spell.
#[must_use]
pub fn bytes_from_hex(text: &str) -> Option<Vec<u8>> {
    // An odd count would leave a last pair of one digit, which parses.
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hashes_what_passes_through_and_passes_it_on() {
        let mut hashing = Hashing::new(Vec::new());
        hashing.write_all(b"ab").unwrap();
        hashing.write_all(b"c").unwrap();
        hashing.flush().unwrap();
        let (abc, written) = hashing.finish();
        assert_eq!(written, b"abc");
        assert_eq!(
            hex(&abc),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(&mut &b""[..]).map(|digest| hex(&digest)).unwrap(),
            EMPTY
        );
    }

    /// A writer that only counts the flushes it is asked for.
    struct Flushes(usize);

    impl Write for Flushes {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0 = self.0.saturating_add(1);
            Ok(())
        }
    }

    #[test]
    fn a_flush_reaches_the_writer_underneath() {
        let mut hashing = Hashing::new(Flushes(0));
        hashing.flush().unwrap();
        assert_eq!(hashing.finish().1.0, 1);
    }

    #[test]
    fn reads_only_exactly_64_hexadecimal_digits() {
        assert_eq!(
            from_hex(EMPTY).map(|digest| hex(&digest)),
            Some(EMPTY.to_owned())
        );
        let upper = EMPTY.to_uppercase();
        assert_eq!(
            from_hex(&upper).map(|digest| hex(&digest)),
            Some(EMPTY.to_owned())
        );
        let short = EMPTY.get(..63).unwrap();
        let long = format!("{EMPTY}0");
        let longer = format!("{EMPTY}00");
        let not_hex = EMPTY.replacen('e', "g", 1);
        let signed = EMPTY.replacen('e', "+", 1);
        for wrong in ["", "e", short, &long, &longer, &not_hex, &signed] {
            assert_eq!(from_hex(wrong), None, "{wrong}");
        }
    }

    #[test]
    fn reads_any_even_count_of_hexadecimal_digits_back_to_its_bytes() {
        for bytes in [&b""[..], b"\x00", b"\xff\x10", b"config.vdf"] {
            assert_eq!(bytes_from_hex(&hex(bytes)).as_deref(), Some(bytes));
        }
        assert_eq!(bytes_from_hex("0A").as_deref(), Some(&b"\n"[..]));
        for wrong in ["0", "abc", "0g", "+1", " 01", "0\u{e9}"] {
            assert_eq!(bytes_from_hex(wrong), None, "{wrong}");
        }
    }
}
