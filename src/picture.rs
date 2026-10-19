//! What a picture file is and how large, from its header alone: PNG, JPEG, and Windows and Apple
//! icons, the formats Steamworks takes artwork in.
//!
//! Nothing is decoded; only the few bytes that say the size are read, so a hostile file can
//! cost no more than walking its headers.

use std::fmt;

/// A picture's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A PNG, and whether it can be see-through: an alpha channel, or a transparent colour.
    Png {
        transparent: bool,
    },
    Jpeg,
    /// A Windows icon; its size is its largest image's.
    Ico,
    /// An Apple icon, ICNS; its size is its largest image's.
    Icns,
}

impl Format {
    /// The format as a sentence says it after "is" or "not": "a PNG", "an Apple icon".
    #[must_use]
    pub const fn with_article(self) -> &'static str {
        match self {
            Self::Png { .. } => "a PNG",
            Self::Jpeg => "a JPEG",
            Self::Ico => "a Windows icon",
            Self::Icns => "an Apple icon",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Png { .. } => "PNG",
            Self::Jpeg => "JPEG",
            Self::Ico => "Windows icon",
            Self::Icns => "Apple icon",
        })
    }
}

/// A picture's format and size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picture {
    pub format: Format,
    pub width: u32,
    pub height: u32,
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Reads what `bytes`, a whole file, are.
///
/// # Errors
///
/// When they are none of the four formats, or their header is cut short or says no size.
pub fn read(bytes: &[u8]) -> Result<Picture, String> {
    type Reader = fn(&[u8]) -> Result<Picture, String>;
    let formats: [(&[u8], Reader); 4] = [
        (PNG_SIGNATURE, png),
        (b"\xff\xd8", jpeg),
        (b"\x00\x00\x01\x00", ico),
        (b"icns", icns),
    ];
    formats
        .iter()
        .find_map(|(start, reader)| bytes.strip_prefix(*start).map(reader))
        .unwrap_or_else(|| Err("is not a PNG, a JPEG, a Windows icon or an Apple icon".to_owned()))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(slice.try_into().ok()?))
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes(slice.try_into().ok()?))
}

/// A PNG after its signature: `IHDR` first, then any chunks up to the image data, among which a
/// `tRNS` makes a colour see-through.
fn png(mut rest: &[u8]) -> Result<Picture, String> {
    let cut = || "is a PNG cut short".to_owned();
    let mut header: Option<(u32, u32, u8)> = None;
    let mut transparent_colour = false;
    loop {
        let length = be32(rest, 0)
            .and_then(|length| usize::try_from(length).ok())
            .ok_or_else(cut)?;
        let kind = rest.get(4..8).ok_or_else(cut)?;
        let data = rest
            .get(8..8_usize.checked_add(length).ok_or_else(cut)?)
            .ok_or_else(cut)?;
        match kind {
            b"IHDR" if header.is_none() => {
                let width = be32(data, 0).ok_or_else(cut)?;
                let height = be32(data, 4).ok_or_else(cut)?;
                let colour = *data.get(9).ok_or_else(cut)?;
                header = Some((width, height, colour));
            }
            _ if header.is_none() => {
                return Err("is a PNG that does not start with IHDR".to_owned());
            }
            b"tRNS" => transparent_colour = true,
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        // The chunk's type, data and checksum.
        rest = rest
            .get(length.checked_add(12).ok_or_else(cut)?..)
            .ok_or_else(cut)?;
    }
    let (width, height, colour) = header.ok_or_else(cut)?;
    // Colour types 4 and 6 carry an alpha channel.
    let transparent = matches!(colour, 4 | 6) || transparent_colour;
    sized(Format::Png { transparent }, width, height)
}

/// A JPEG after its start marker: segments up to the frame header that holds the size.
fn jpeg(mut rest: &[u8]) -> Result<Picture, String> {
    let cut = || "is a JPEG cut short".to_owned();
    loop {
        let fill = rest.iter().take_while(|byte| **byte == 0xff).count();
        if fill == 0 {
            return Err("is a JPEG whose segments cannot be read".to_owned());
        }
        let marker = *rest.get(fill).ok_or_else(cut)?;
        rest = rest
            .get(fill.checked_add(1).ok_or_else(cut)?..)
            .ok_or_else(cut)?;
        match marker {
            // Markers that stand alone, with no length after them.
            0x01 | 0xd0..=0xd7 => {}
            0xd9 | 0xda => return Err("is a JPEG with no frame header".to_owned()),
            _ => {
                let length = usize::from(be16(rest, 0).ok_or_else(cut)?);
                // Frame headers: every SOF marker, but not DHT, JPG or DAC among them.
                if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                    let height = be16(rest, 3).ok_or_else(cut)?;
                    let width = be16(rest, 5).ok_or_else(cut)?;
                    return sized(Format::Jpeg, u32::from(width), u32::from(height));
                }
                rest = rest.get(length..).ok_or_else(cut)?;
            }
        }
    }
}

/// A Windows icon after its first four bytes: the count of images, then an entry for each, in
/// which a size of 0 stands for 256.
fn ico(rest: &[u8]) -> Result<Picture, String> {
    let cut = || "is a Windows icon cut short".to_owned();
    let [low, high, ..] = *rest else {
        return Err(cut());
    };
    let count = usize::from(u16::from_le_bytes([low, high]));
    let (entries, _) = rest.get(2..).ok_or_else(cut)?.as_chunks::<16>();
    let listed = entries.get(..count).ok_or_else(cut)?;
    let side = |side: u8| if side == 0 { 256 } else { u32::from(side) };
    let (width, height) = listed
        .iter()
        .map(|&[width, height, ..]| (side(width), side(height)))
        .max_by_key(|&(width, height)| u64::from(width).saturating_mul(u64::from(height)))
        .ok_or_else(|| "is a Windows icon with no images".to_owned())?;
    sized(Format::Ico, width, height)
}

/// The side in pixels of each kind of image an Apple icon can hold, by its type. Masks, the table
/// of contents and the version say no size of their own.
const ICNS_SIDES: [(&[u8; 4], u32); 30] = [
    (b"ics#", 16),
    (b"ics4", 16),
    (b"ics8", 16),
    (b"is32", 16),
    (b"icp4", 16),
    (b"ic04", 16),
    (b"icsb", 18),
    (b"sb24", 24),
    (b"ICN#", 32),
    (b"icl4", 32),
    (b"icl8", 32),
    (b"il32", 32),
    (b"icp5", 32),
    (b"ic05", 32),
    (b"ic11", 32),
    (b"icsB", 36),
    (b"ich#", 48),
    (b"ich4", 48),
    (b"ich8", 48),
    (b"ih32", 48),
    (b"SB24", 48),
    (b"icp6", 64),
    (b"ic12", 64),
    (b"it32", 128),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),
    (b"ic09", 512),
    (b"ic14", 512),
    (b"ic10", 1024),
];

/// An Apple icon after its type: its whole length, then elements that each start with their type
/// and their length, both lengths counting the eight bytes that say them.
fn icns(rest: &[u8]) -> Result<Picture, String> {
    let cut = || "is an Apple icon cut short".to_owned();
    let length = be32(rest, 0)
        .and_then(|length| usize::try_from(length).ok())
        .ok_or_else(cut)?;
    let mut elements = rest
        .get(4..length.checked_sub(4).ok_or_else(cut)?)
        .ok_or_else(cut)?;
    let mut largest = None;
    while !elements.is_empty() {
        let kind = elements.get(..4).ok_or_else(cut)?;
        let element = be32(elements, 4)
            .and_then(|element| usize::try_from(element).ok())
            .ok_or_else(cut)?;
        if element < 8 {
            return Err("is an Apple icon whose elements cannot be read".to_owned());
        }
        if let Some(&(_, side)) = ICNS_SIDES.iter().find(|(known, _)| *known == kind) {
            largest = largest.max(Some(side));
        }
        elements = elements.get(element..).ok_or_else(cut)?;
    }
    let side = largest.ok_or_else(|| "is an Apple icon with no images".to_owned())?;
    sized(Format::Icns, side, side)
}

fn sized(format: Format, width: u32, height: u32) -> Result<Picture, String> {
    if width == 0 || height == 0 {
        return Err(format!(
            "is {} that says it has no size",
            format.with_article()
        ));
    }
    Ok(Picture {
        format,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::*;

    fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0; 4]);
        chunk
    }

    fn png_with(width: u32, height: u32, colour: u8, before_data: &[Vec<u8>]) -> Vec<u8> {
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, colour, 0, 0, 0]);
        let mut file = PNG_SIGNATURE.to_vec();
        file.extend(chunk(b"IHDR", &header));
        for extra in before_data {
            file.extend_from_slice(extra);
        }
        file.extend(chunk(b"IDAT", &[1, 2, 3]));
        file.extend(chunk(b"IEND", &[]));
        file
    }

    #[test]
    fn a_png_is_read_with_whether_it_can_be_see_through() {
        for (colour, extra, transparent) in [
            (2, vec![], false),
            (6, vec![], true),
            (4, vec![], true),
            (3, vec![chunk(b"PLTE", &[0; 3]), chunk(b"tRNS", &[0])], true),
            (3, vec![chunk(b"PLTE", &[0; 3])], false),
        ] {
            assert_eq!(
                read(&png_with(1280, 720, colour, &extra)).unwrap(),
                Picture {
                    format: Format::Png { transparent },
                    width: 1280,
                    height: 720
                },
                "colour type {colour}"
            );
        }
    }

    #[test]
    fn a_jpeg_is_read_from_its_frame_header_past_what_comes_first() {
        let mut file = b"\xff\xd8".to_vec();
        // An EXIF segment, fill bytes, a standalone marker, then a progressive frame header.
        file.extend_from_slice(b"\xff\xe1\x00\x06Exif");
        file.extend_from_slice(b"\xff\xff\xd0");
        file.extend_from_slice(b"\xff\xc4\x00\x03\x00");
        file.extend_from_slice(b"\xff\xc2\x00\x11\x08\x04\x38\x07\x80\x03");
        assert_eq!(
            read(&file).unwrap(),
            Picture {
                format: Format::Jpeg,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn a_png_is_as_large_as_its_first_header_says() {
        let mut second = 64_u32.to_be_bytes().to_vec();
        second.extend_from_slice(&64_u32.to_be_bytes());
        second.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(
            read(&png_with(920, 430, 2, &[chunk(b"IHDR", &second)])).unwrap(),
            Picture {
                format: Format::Png { transparent: false },
                width: 920,
                height: 430
            }
        );
    }

    #[test]
    fn an_icon_of_one_small_image_is_that_size() {
        let mut file = b"\x00\x00\x01\x00\x01\x00".to_vec();
        file.extend_from_slice(&[48, 32, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            read(&file).unwrap(),
            Picture {
                format: Format::Ico,
                width: 48,
                height: 32
            }
        );
    }

    #[test]
    fn an_icon_is_as_large_as_its_largest_image_and_0_is_256() {
        let mut file = b"\x00\x00\x01\x00\x02\x00".to_vec();
        file.extend_from_slice(&[32, 32, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        file.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            read(&file).unwrap(),
            Picture {
                format: Format::Ico,
                width: 256,
                height: 256
            }
        );
    }

    /// An Apple icon of elements of these types and lengths, each filled with zeros.
    fn icns_of(elements: &[(&[u8; 4], u32)]) -> Vec<u8> {
        let mut body = Vec::new();
        for &(kind, length) in elements {
            body.extend_from_slice(kind);
            body.extend_from_slice(&length.to_be_bytes());
            let filled = usize::try_from(length.checked_sub(8).unwrap()).unwrap();
            body.extend(iter::repeat_n(0, filled));
        }
        let mut file = b"icns".to_vec();
        let whole = body.len().checked_add(8).unwrap();
        file.extend_from_slice(&u32::try_from(whole).unwrap().to_be_bytes());
        file.extend(body);
        file
    }

    #[test]
    fn an_apple_icon_is_as_large_as_its_largest_image_whatever_else_it_holds() {
        // The kinds of element Steam's own Mac icon for app 203160 holds, the largest image not
        // last.
        let icon = icns_of(&[
            (b"TOC ", 80),
            (b"is32", 782),
            (b"s8mk", 264),
            (b"il32", 3096),
            (b"l8mk", 1032),
            (b"it32", 4549),
            (b"t8mk", 1639),
            (b"ic08", 1129),
            (b"ic10", 1240),
            (b"ic09", 3809),
            (b"icnV", 12),
        ]);
        assert_eq!(
            read(&icon).unwrap(),
            Picture {
                format: Format::Icns,
                width: 1024,
                height: 1024
            }
        );
        assert_eq!(
            read(&icns_of(&[(b"it32", 9), (b"t8mk", 8)])).unwrap(),
            Picture {
                format: Format::Icns,
                width: 128,
                height: 128
            }
        );
    }

    #[test]
    fn what_is_no_picture_or_is_cut_short_says_so() {
        let whole = png_with(10, 10, 6, &[]);
        for (bytes, why) in [
            (
                b"GIF89a".to_vec(),
                "is not a PNG, a JPEG, a Windows icon or an Apple icon",
            ),
            (
                whole.iter().take(20).copied().collect(),
                "is a PNG cut short",
            ),
            (
                [PNG_SIGNATURE, &chunk(b"IDAT", &[])].concat(),
                "is a PNG that does not start with IHDR",
            ),
            (png_with(0, 10, 6, &[]), "is a PNG that says it has no size"),
            (b"\xff\xd8\xff\xe1\x00".to_vec(), "is a JPEG cut short"),
            (
                b"\xff\xd8\xff\xd9".to_vec(),
                "is a JPEG with no frame header",
            ),
            (
                b"\xff\xd8\x00".to_vec(),
                "is a JPEG whose segments cannot be read",
            ),
            (
                b"\x00\x00\x01\x00\x02\x00".to_vec(),
                "is a Windows icon cut short",
            ),
            (
                b"\x00\x00\x01\x00\x00\x00".to_vec(),
                "is a Windows icon with no images",
            ),
            (b"icns\x00\x00".to_vec(), "is an Apple icon cut short"),
            (icns_of(&[]), "is an Apple icon with no images"),
            (
                icns_of(&[(b"TOC ", 8), (b"icnV", 12)]),
                "is an Apple icon with no images",
            ),
            // A whole length too short to count its own eight bytes.
            (
                b"icns\x00\x00\x00\x07".to_vec(),
                "is an Apple icon cut short",
            ),
            (
                b"icns\x00\x00\x00\x03".to_vec(),
                "is an Apple icon cut short",
            ),
            (
                b"icns\x00\x00\x00\x10\x00\x00\x00\x00".to_vec(),
                "is an Apple icon cut short",
            ),
            (
                b"icns\x00\x00\x00\x0dis32\x00".to_vec(),
                "is an Apple icon cut short",
            ),
            (
                b"icns\x00\x00\x00\x10is32\x00\x00\x00\x07".to_vec(),
                "is an Apple icon whose elements cannot be read",
            ),
            (
                b"icns\x00\x00\x00\x10is32\x00\x00\x00\x09".to_vec(),
                "is an Apple icon cut short",
            ),
        ] {
            assert_eq!(read(&bytes).unwrap_err(), why, "{bytes:?}");
        }
    }

    #[test]
    fn a_length_as_large_as_it_can_be_is_cut_short_not_an_overflow() {
        let mut file = PNG_SIGNATURE.to_vec();
        file.extend_from_slice(&[0xff; 8]);
        assert_eq!(read(&file).unwrap_err(), "is a PNG cut short");
    }
}
