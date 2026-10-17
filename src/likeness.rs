//! Whether the image Steam serves of an achievement icon is the icon's file, pixel by pixel.
//!
//! Steam serves each icon as a JPEG, so hardly a pixel is exactly the file's. On Fantasy Guild
//! Manager's 28 icons that Steam held as imported, no pixel was more than 48 off in any channel,
//! where an icon drawn again since had about 9,700 of its 65,536 pixels more than 64 off. So a
//! pixel more than [`OFF`] off in a channel differs, and an image differs once more than one pixel
//! in [`SHARE`] does: few enough that a changed detail of ten pixels square is caught.

use std::fmt;
use std::io::Cursor;

use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// How far a channel may be off, out of 255, before its pixel differs.
pub const OFF: u8 = 64;

/// One pixel in this many may differ before the image does.
pub const SHARE: u64 = 1000;

/// The longest side either image may have. The editor takes icons of 64 to 256 pixels, so this
/// leaves room while keeping a hostile file from costing more than a few tens of megabytes.
const SIDE: u32 = 4096;

/// How alike the file of an icon and Steam's image of it are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Likeness {
    /// The pixels compared: those the file has opaque, since what Steam puts behind a
    /// see-through one is not known.
    pub compared: u64,
    /// Those of them with a channel more than [`OFF`] off.
    pub differing: u64,
    /// How far off every channel compared is, added up.
    pub total: u64,
}

impl Likeness {
    /// Whether Steam's image is the file's, as far as a JPEG can be.
    #[must_use]
    pub const fn same(&self) -> bool {
        self.differing.saturating_mul(SHARE) <= self.compared
    }

    /// How far off a channel is on average, in tenths of one step out of 255.
    #[must_use]
    pub fn mean_tenths(&self) -> u64 {
        self.total
            .saturating_mul(10)
            .checked_div(self.compared.saturating_mul(3))
            .unwrap_or(0)
    }
}

impl fmt::Display for Likeness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tenths = self.mean_tenths();
        write!(
            formatter,
            "{} of {} pixels more than {OFF} off, {}.{} on average",
            self.differing,
            self.compared,
            tenths.checked_div(10).unwrap_or(0),
            tenths.checked_rem(10).unwrap_or(0)
        )
    }
}

fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage, String> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(SIDE);
    limits.max_image_height = Some(SIDE);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map_err(|error| error.to_string())
}

/// How alike `file`, an icon's PNG, and `served`, the JPEG Steam serves of it, are, with the file
/// scaled to the size Steam serves.
///
/// # Errors
///
/// When either cannot be decoded, saying which.
pub fn compare(file: &[u8], served: &[u8]) -> Result<Likeness, String> {
    let file = decode(file, ImageFormat::Png)
        .map_err(|why| format!("the file: {why}"))?
        .to_rgba8();
    let served = decode(served, ImageFormat::Jpeg)
        .map_err(|why| format!("Steam's image: {why}"))?
        .to_rgb8();
    let file = if file.dimensions() == served.dimensions() {
        file
    } else {
        imageops::resize(&file, served.width(), served.height(), FilterType::Lanczos3)
    };
    let mut likeness = Likeness::default();
    for (theirs, ours) in served.pixels().zip(file.pixels()) {
        let [red, green, blue, alpha] = ours.0;
        if alpha < u8::MAX {
            continue;
        }
        let offs = [red, green, blue]
            .into_iter()
            .zip(theirs.0)
            .map(|(our, their)| our.abs_diff(their));
        let mut most = 0_u8;
        for off in offs {
            most = most.max(off);
            likeness.total = likeness.total.saturating_add(u64::from(off));
        }
        likeness.compared = likeness.compared.saturating_add(1);
        if most > OFF {
            likeness.differing = likeness.differing.saturating_add(1);
        }
    }
    Ok(likeness)
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgb, RgbImage, Rgba, RgbaImage};

    use super::*;

    /// The pixels of a 256-pixel icon.
    const ALL: u64 = 256 * 256;

    /// A 256-pixel icon: a dark field with a light square of `side` pixels in its middle.
    fn drawn(side: u32) -> RgbaImage {
        let start = 128_u32.saturating_sub(side.checked_div(2).unwrap_or(0));
        let end = start.saturating_add(side);
        ImageBuffer::from_fn(256, 256, |x, y| {
            if (start..end).contains(&x) && (start..end).contains(&y) {
                Rgba([240, 220, 160, 255])
            } else {
                Rgba([30, 40, 90, 255])
            }
        })
    }

    fn png(picture: &RgbaImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn jpeg(picture: &RgbaImage) -> Vec<u8> {
        let flat: RgbImage = DynamicImage::ImageRgba8(picture.clone()).to_rgb8();
        let mut bytes = Cursor::new(Vec::new());
        flat.write_to(&mut bytes, ImageFormat::Jpeg).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn an_icon_steam_serves_as_imported_is_the_same_despite_the_jpeg() {
        let icon = drawn(96);
        let likeness = compare(&png(&icon), &jpeg(&icon)).unwrap();
        assert!(likeness.same(), "{likeness}");
        assert_eq!(likeness.compared, ALL);
    }

    #[test]
    fn a_detail_of_ten_pixels_square_drawn_since_makes_the_icon_differ() {
        let likeness = compare(&png(&drawn(10)), &jpeg(&drawn(0))).unwrap();
        assert!(!likeness.same(), "{likeness}");
        assert!(
            likeness
                .to_string()
                .starts_with("100 of 65536 pixels more than 64 off, "),
            "{likeness}"
        );
    }

    #[test]
    fn an_icon_drawn_again_is_named_with_how_far_off_it_is() {
        let likeness = compare(&png(&drawn(200)), &jpeg(&drawn(0))).unwrap();
        assert_eq!(likeness.differing, 40_000);
        assert_eq!(likeness.mean_tenths(), 935);
        assert_eq!(
            likeness.to_string(),
            "40000 of 65536 pixels more than 64 off, 93.5 on average"
        );
    }

    #[test]
    fn a_file_larger_than_steams_image_is_scaled_to_it() {
        let large: RgbaImage = ImageBuffer::from_pixel(512, 512, Rgba([30, 40, 90, 255]));
        let likeness = compare(&png(&large), &jpeg(&drawn(0))).unwrap();
        assert_eq!((likeness.compared, likeness.differing), (ALL, 0));
    }

    #[test]
    fn a_see_through_pixel_is_left_out() {
        let mut icon = drawn(0);
        icon.put_pixel(0, 0, Rgba([255, 255, 255, 0]));
        icon.put_pixel(1, 0, Rgba([255, 255, 255, 254]));
        let likeness = compare(&png(&icon), &jpeg(&drawn(0))).unwrap();
        assert_eq!(
            (likeness.compared, likeness.differing),
            (ALL.saturating_sub(2), 0)
        );
    }

    #[test]
    fn what_cannot_be_decoded_is_named() {
        let icon = png(&drawn(0));
        let unread_file = compare(b"not a PNG", &icon).unwrap_err();
        assert!(unread_file.starts_with("the file: "), "{unread_file}");
        let unread_image = compare(&icon, &icon).unwrap_err();
        assert!(
            unread_image.starts_with("Steam's image: "),
            "{unread_image}"
        );
        let wide: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(SIDE.saturating_add(1), 1);
        let mut bytes = Cursor::new(Vec::new());
        wide.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let too_wide = compare(&bytes.into_inner(), &icon).unwrap_err();
        assert!(too_wide.starts_with("the file: "), "{too_wide}");
    }

    #[test]
    fn nothing_compared_is_nothing_off() {
        assert_eq!(Likeness::default().mean_tenths(), 0);
        assert!(Likeness::default().same());
    }
}
