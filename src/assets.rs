//! Steamworks artwork checked against Valve's sizes and formats before it is uploaded by hand.
//!
//! Each file is named for the asset it is: `header_capsule.png`, `library_logo.png`,
//! `screenshot_03.jpg`, `linux_icons.zip`. The sizes are Valve's, from Steamworks' documentation of store, library, community and event
//! assets, and from the Workshop's Branding and the app's Client Images pages.

use std::fs;
use std::io::{self, Cursor, Read as _};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::picture::{self, Format, Picture};

/// How large an asset must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    Exact(u32, u32),
    /// A screenshot: at least 1920 by 1080, and 16:9.
    Widescreen,
    /// The library logo: 1280 wide, 720 tall, or both, and neither larger.
    Logo,
    /// The shortcut icon: 256 by 256, or as a PNG 512 by 512. A Windows icon is as large as its
    /// largest image, and none is larger than 256.
    Shortcut,
    /// Any size, where Valve names none.
    Any,
}

/// The formats an asset may be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Formats {
    /// A PNG or a JPEG, where Valve names neither.
    Picture,
    Png,
    /// A PNG that can be see-through, as the library logo is laid over the hero.
    SeeThroughPng,
    Jpeg,
    /// A Windows icon or a PNG, for the shortcut icon.
    IconOrPng,
    /// An Apple icon, for the Mac.
    AppleIcon,
    /// A zip of square PNGs, one for each size, for the Linux icons.
    PngZip,
}

/// An asset Steamworks takes, the file name it is kept under, and its size and formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// What Steamworks calls it.
    name: &'static str,
    /// The file's name without its extension; screenshots start with it and may go on.
    stem: &'static str,
    size: Size,
    formats: Formats,
}

impl Asset {
    /// What Steamworks calls it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

const fn asset(name: &'static str, stem: &'static str, size: Size, formats: Formats) -> Asset {
    Asset {
        name,
        stem,
        size,
        formats,
    }
}

/// Every asset, in the order Steamworks lists them.
const ASSETS: [Asset; 18] = [
    asset(
        "header capsule",
        "header_capsule",
        Size::Exact(920, 430),
        Formats::Picture,
    ),
    asset(
        "small capsule",
        "small_capsule",
        Size::Exact(462, 174),
        Formats::Picture,
    ),
    asset(
        "main capsule",
        "main_capsule",
        Size::Exact(1232, 706),
        Formats::Picture,
    ),
    asset(
        "vertical capsule",
        "vertical_capsule",
        Size::Exact(748, 896),
        Formats::Picture,
    ),
    asset(
        "screenshot",
        "screenshot",
        Size::Widescreen,
        Formats::Picture,
    ),
    asset(
        "page background",
        "page_background",
        Size::Exact(1438, 810),
        Formats::Picture,
    ),
    asset(
        "bundle header",
        "bundle_header",
        Size::Exact(707, 232),
        Formats::Picture,
    ),
    asset(
        "library capsule",
        "library_capsule",
        Size::Exact(600, 900),
        Formats::Png,
    ),
    asset(
        "library header",
        "library_header",
        Size::Exact(920, 430),
        Formats::Png,
    ),
    asset(
        "library hero",
        "library_hero",
        Size::Exact(3840, 1240),
        Formats::Png,
    ),
    asset(
        "library logo",
        "library_logo",
        Size::Logo,
        Formats::SeeThroughPng,
    ),
    asset(
        "shortcut icon",
        "shortcut_icon",
        Size::Shortcut,
        Formats::IconOrPng,
    ),
    asset("app icon", "app_icon", Size::Exact(184, 184), Formats::Jpeg),
    asset("Mac icon", "mac_icon", Size::Any, Formats::AppleIcon),
    asset("Linux icons", "linux_icons", Size::Any, Formats::PngZip),
    asset(
        "event cover",
        "event_cover",
        Size::Exact(800, 450),
        Formats::Picture,
    ),
    asset(
        "event header",
        "event_header",
        Size::Exact(1920, 622),
        Formats::Picture,
    ),
    // The Workshop's Branding page asks for "948x203px JPG", as its Photoshop template is.
    asset(
        "Workshop branding image",
        "workshop_header",
        Size::Exact(948, 203),
        Formats::Jpeg,
    ),
];

/// Steamworks' least number of screenshots for a store page.
pub const SCREENSHOTS: usize = 5;

/// The extensions of the files looked at; anything else in the folder is left alone.
const LOOKED_AT: [&str; 6] = ["png", "jpg", "jpeg", "ico", "icns", "zip"];

/// The most of each file in the Linux icons' zip that is read. A PNG says its size in its first
/// chunks, and a zip's entry can unpack to far more than the zip holds.
const ZIPPED_LIMIT: u64 = 4 << 20;

/// The asset a file named `stem` (its name without the extension) is kept as.
#[must_use]
pub fn named(stem: &str) -> Option<&'static Asset> {
    let stem = stem.to_ascii_lowercase();
    ASSETS.iter().find(|asset| {
        if asset.size == Size::Widescreen {
            stem.starts_with(asset.stem)
        } else {
            stem == asset.stem
        }
    })
}

/// Every way `picture` is not what `asset` must be.
#[must_use]
pub fn problems(asset: &Asset, picture: &Picture) -> Vec<String> {
    let mut found = Vec::new();
    let png = matches!(picture.format, Format::Png { .. });
    let (taken, kind) = match asset.formats {
        Formats::Picture => (png || picture.format == Format::Jpeg, "a PNG or a JPEG"),
        Formats::Png => (png, "a PNG"),
        Formats::SeeThroughPng if png => (
            picture.format == Format::Png { transparent: true },
            "a PNG that can be see-through, with an alpha channel",
        ),
        Formats::SeeThroughPng => (false, "a PNG that can be see-through"),
        Formats::Jpeg => (picture.format == Format::Jpeg, "a JPEG"),
        Formats::IconOrPng => (
            png || picture.format == Format::Ico,
            "a Windows icon or a PNG",
        ),
        Formats::AppleIcon => (picture.format == Format::Icns, "an Apple icon (ICNS)"),
        Formats::PngZip => (false, "a zip of PNGs"),
    };
    if !taken {
        found.push(format!(
            "the {} must be {kind}, not {}",
            asset.name,
            picture.format.with_article()
        ));
    }
    let (width, height) = (picture.width, picture.height);
    let size_wanted = match asset.size {
        Size::Exact(wanted_width, wanted_height) => ((width, height)
            != (wanted_width, wanted_height))
            .then(|| format!("{wanted_width}x{wanted_height}")),
        Size::Widescreen => {
            let widescreen =
                u64::from(width).saturating_mul(9) == u64::from(height).saturating_mul(16);
            // At 16:9, below 1920 wide is below 1080 tall as well.
            (width < 1920 || !widescreen).then_some("at least 1920x1080, and 16:9".to_owned())
        }
        Size::Logo => {
            let fits = (width == 1280 && height <= 720) || (height == 720 && width <= 1280);
            (!fits).then_some("1280 wide or 720 tall, and no larger".to_owned())
        }
        Size::Shortcut => {
            let fits = (width, height) == (256, 256) || (png && (width, height) == (512, 512));
            (!fits).then_some("256x256, or 512x512 as a PNG".to_owned())
        }
        Size::Any => None,
    };
    if let Some(wanted) = size_wanted {
        found.push(format!(
            "the {} must be {wanted}, and it is {width}x{height}",
            asset.name
        ));
    }
    found
}

/// A file in the folder, and what came of checking it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub path: PathBuf,
    /// The asset it is and what it holds, as `assets` lists it, such as "header capsule, 920x430
    /// PNG"; or every way it is not what Steamworks takes.
    pub found: Result<String, Vec<String>>,
}

/// Checks every picture, icon and zip in `folder`, in the order of their names.
///
/// # Errors
///
/// When the folder cannot be listed. A file that cannot be read is a problem of that file's.
pub fn check(folder: &Path) -> io::Result<Vec<Checked>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<_>>()?;
    paths.retain(|path| {
        path.is_file()
            && path.extension().is_some_and(|extension| {
                LOOKED_AT
                    .iter()
                    .any(|looked_at| extension.eq_ignore_ascii_case(looked_at))
            })
    });
    paths.sort();
    Ok(paths.into_iter().map(checked).collect())
}

fn checked(path: PathBuf) -> Checked {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let found = named(&stem)
        .ok_or_else(|| {
            vec![
                "is named for no asset: name it header_capsule, screenshot_01, library_logo and \
                 so on"
                    .to_owned(),
            ]
        })
        .and_then(|asset| {
            let bytes = fs::read(&path).map_err(|error| vec![error.to_string()])?;
            let held = if asset.formats == Formats::PngZip {
                linux_icons(&bytes)?
            } else {
                let picture = picture::read(&bytes).map_err(|why| vec![why])?;
                let wrong = problems(asset, &picture);
                if !wrong.is_empty() {
                    return Err(wrong);
                }
                format!("{}x{} {}", picture.width, picture.height, picture.format)
            };
            Ok(format!("{}, {held}", asset.name))
        });
    Checked { path, found }
}

/// What the Linux icons' zip `bytes` hold, such as "5 PNGs from 16x16 to 256x256", or every way
/// they are not square PNGs.
///
/// # Errors
///
/// Every file in the zip that is not a square PNG, and a zip that cannot be read or holds none.
pub fn linux_icons(bytes: &[u8]) -> Result<String, Vec<String>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| vec![format!("cannot be read as a zip: {error}")])?;
    let mut wrong = Vec::new();
    let mut sides = Vec::new();
    for index in 0..zip.len() {
        let mut entry = match zip.by_index(index) {
            Ok(entry) => entry,
            Err(error) => {
                wrong.push(format!("cannot be read as a zip: {error}"));
                continue;
            }
        };
        let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
        if entry.is_dir() || from_macos(&name) {
            continue;
        }
        let mut start = Vec::new();
        if let Err(error) = entry.by_ref().take(ZIPPED_LIMIT).read_to_end(&mut start) {
            wrong.push(format!("{name} cannot be read from the zip: {error}"));
            continue;
        }
        match picture::read(&start) {
            Ok(Picture {
                format: Format::Png { .. },
                width,
                height,
            }) if width == height => sides.push(width),
            Ok(Picture {
                format: Format::Png { .. },
                width,
                height,
            }) => wrong.push(format!(
                "the Linux icons must be square, and {name} is {width}x{height}"
            )),
            Ok(picture) => wrong.push(format!(
                "the Linux icons must be PNGs, and {name} is {}",
                picture.format.with_article()
            )),
            Err(why) => wrong.push(format!("the Linux icons must be PNGs, and {name} {why}")),
        }
    }
    if !wrong.is_empty() {
        return Err(wrong);
    }
    sides.sort_unstable();
    match (sides.first(), sides.last()) {
        (Some(smallest), Some(largest)) if sides.len() > 1 => Ok(format!(
            "{} PNGs from {smallest}x{smallest} to {largest}x{largest}",
            sides.len()
        )),
        (Some(only), _) => Ok(format!("a {only}x{only} PNG")),
        (None, _) => Err(vec![
            "the Linux icons must be PNGs, and the zip holds none".to_owned(),
        ]),
    }
}

/// Whether `name`, a path in a zip, is what macOS adds to a zip it makes: Steam's own Linux icons
/// for app 8930 hold a `__MACOSX` folder beside their PNG.
fn from_macos(name: &str) -> bool {
    name.starts_with("__MACOSX/")
        || name
            .rsplit('/')
            .next()
            .is_some_and(|file| file.starts_with('.'))
}

/// How many screenshots `checked` holds, when that is some but fewer than Steamworks needs.
#[must_use]
pub fn too_few_screenshots(checked: &[Checked]) -> Option<usize> {
    let screenshots = checked
        .iter()
        .filter(|file| {
            file.path
                .file_stem()
                .and_then(|stem| named(&stem.to_string_lossy()))
                .is_some_and(|asset| asset.size == Size::Widescreen)
        })
        .count();
    (1..SCREENSHOTS)
        .contains(&screenshots)
        .then_some(screenshots)
}

#[cfg(test)]
mod tests {
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    const fn png(width: u32, height: u32, transparent: bool) -> Picture {
        Picture {
            format: Format::Png { transparent },
            width,
            height,
        }
    }

    const fn of(format: Format, width: u32, height: u32) -> Picture {
        Picture {
            format,
            width,
            height,
        }
    }

    fn asset_named(stem: &str) -> &'static Asset {
        named(stem).unwrap()
    }

    #[test]
    fn a_file_is_known_by_its_name_whatever_its_case_and_screenshots_by_how_they_start() {
        assert_eq!(asset_named("Header_Capsule").name, "header capsule");
        assert_eq!(asset_named("screenshot_07").name, "screenshot");
        assert_eq!(asset_named("screenshot").name, "screenshot");
        assert_eq!(named("header_capsule_old"), None);
        assert_eq!(named("capsule"), None);
    }

    #[test]
    fn the_right_size_and_format_is_no_problem() {
        for (stem, picture) in [
            ("header_capsule", png(920, 430, false)),
            ("header_capsule", of(Format::Jpeg, 920, 430)),
            ("screenshot_1", png(1920, 1080, false)),
            ("screenshot_2", of(Format::Jpeg, 3840, 2160)),
            ("library_logo", png(1280, 400, true)),
            ("library_logo", png(900, 720, true)),
            ("library_hero", png(3840, 1240, false)),
            ("shortcut_icon", of(Format::Ico, 256, 256)),
            ("shortcut_icon", png(256, 256, true)),
            ("shortcut_icon", png(512, 512, false)),
            ("app_icon", of(Format::Jpeg, 184, 184)),
            ("mac_icon", of(Format::Icns, 128, 128)),
            ("mac_icon", of(Format::Icns, 1024, 1024)),
            ("workshop_header", of(Format::Jpeg, 948, 203)),
        ] {
            assert_eq!(
                problems(asset_named(stem), &picture),
                Vec::<String>::new(),
                "{stem}"
            );
        }
    }

    #[test]
    fn every_wrong_size_and_format_is_named() {
        for (stem, picture, said) in [
            (
                "header_capsule",
                png(921, 430, false),
                vec!["the header capsule must be 920x430, and it is 921x430"],
            ),
            (
                "header_capsule",
                of(Format::Ico, 920, 430),
                vec!["the header capsule must be a PNG or a JPEG, not a Windows icon"],
            ),
            (
                "screenshot_01",
                png(1920, 1200, false),
                vec!["the screenshot must be at least 1920x1080, and 16:9, and it is 1920x1200"],
            ),
            (
                "screenshot_01",
                png(1280, 720, false),
                vec!["the screenshot must be at least 1920x1080, and 16:9, and it is 1280x720"],
            ),
            (
                "library_logo",
                png(1280, 721, false),
                vec![
                    "the library logo must be a PNG that can be see-through, with an alpha \
                     channel, not a PNG",
                    "the library logo must be 1280 wide or 720 tall, and no larger, and it is \
                     1280x721",
                ],
            ),
            (
                "library_logo",
                of(Format::Jpeg, 1000, 500),
                vec![
                    "the library logo must be a PNG that can be see-through, not a JPEG",
                    "the library logo must be 1280 wide or 720 tall, and no larger, and it is \
                     1000x500",
                ],
            ),
            (
                "library_capsule",
                of(Format::Jpeg, 600, 900),
                vec!["the library capsule must be a PNG, not a JPEG"],
            ),
            (
                "library_logo",
                of(Format::Icns, 1280, 1280),
                vec![
                    "the library logo must be a PNG that can be see-through, not an Apple icon",
                    "the library logo must be 1280 wide or 720 tall, and no larger, and it is \
                     1280x1280",
                ],
            ),
            (
                "library_capsule",
                of(Format::Ico, 600, 900),
                vec!["the library capsule must be a PNG, not a Windows icon"],
            ),
            (
                "workshop_header",
                png(948, 204, false),
                vec![
                    "the Workshop branding image must be a JPEG, not a PNG",
                    "the Workshop branding image must be 948x203, and it is 948x204",
                ],
            ),
        ] {
            assert_eq!(problems(asset_named(stem), &picture), said, "{stem}");
        }
    }

    #[test]
    fn every_wrong_size_and_format_of_an_icon_is_named() {
        for (stem, picture, said) in [
            (
                "app_icon",
                png(184, 184, false),
                vec!["the app icon must be a JPEG, not a PNG"],
            ),
            (
                "app_icon",
                of(Format::Ico, 184, 184),
                vec!["the app icon must be a JPEG, not a Windows icon"],
            ),
            (
                "shortcut_icon",
                of(Format::Jpeg, 256, 256),
                vec!["the shortcut icon must be a Windows icon or a PNG, not a JPEG"],
            ),
            (
                "shortcut_icon",
                of(Format::Ico, 128, 128),
                vec!["the shortcut icon must be 256x256, or 512x512 as a PNG, and it is 128x128"],
            ),
            (
                "shortcut_icon",
                png(384, 384, true),
                vec!["the shortcut icon must be 256x256, or 512x512 as a PNG, and it is 384x384"],
            ),
            (
                "shortcut_icon",
                png(512, 256, true),
                vec!["the shortcut icon must be 256x256, or 512x512 as a PNG, and it is 512x256"],
            ),
            (
                "mac_icon",
                png(512, 512, true),
                vec!["the Mac icon must be an Apple icon (ICNS), not a PNG"],
            ),
            (
                "linux_icons",
                png(16, 16, true),
                vec!["the Linux icons must be a zip of PNGs, not a PNG"],
            ),
        ] {
            assert_eq!(problems(asset_named(stem), &picture), said, "{stem}");
        }
    }

    /// The start of a PNG `width` by `height`, as much of it as is read.
    fn png_file(width: u32, height: u32) -> Vec<u8> {
        let mut file = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        file.extend_from_slice(&width.to_be_bytes());
        file.extend_from_slice(&height.to_be_bytes());
        file.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        file.extend_from_slice(b"\x00\x00\x00\x00IEND\x00\x00\x00\x00");
        file
    }

    /// A zip of `entries`, deflated as macOS and Windows zip them; a name ending in `/` is a folder.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for (name, contents) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                io::Write::write_all(&mut zip, contents).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn linux_icons_are_square_pngs_of_any_size_and_names_in_a_zip() {
        // As Steam's own for apps 203160 and 8930 hold them.
        let (sixteen, large) = (png_file(16, 16), png_file(1024, 1024));
        assert_eq!(
            linux_icons(&zip_of(&[
                ("Linux/", b""),
                ("Linux/1024.png", &large),
                ("Linux/16.png", &sixteen),
                ("Linux/24.png", &png_file(24, 24)),
            ])),
            Ok("3 PNGs from 16x16 to 1024x1024".to_owned())
        );
        assert_eq!(
            linux_icons(&zip_of(&[
                ("civ5.png", &png_file(256, 256)),
                ("__MACOSX/", b""),
                ("__MACOSX/._civ5.png", b"\x00\x05\x16\x07\x00\x02\x00\x00"),
                ("icons/.DS_Store", b"Bud1"),
            ])),
            Ok("a 256x256 PNG".to_owned())
        );
    }

    #[test]
    fn every_file_in_the_linux_icons_that_is_no_square_png_is_named() {
        assert_eq!(
            linux_icons(&zip_of(&[
                ("16.png", &png_file(16, 16)),
                ("32.png", &png_file(32, 16)),
                ("readme.txt", b"icons"),
                (
                    "48.jpg",
                    b"\xff\xd8\xff\xc0\x00\x11\x08\x00\x30\x00\x30\x03"
                ),
            ])),
            Err(vec![
                "the Linux icons must be square, and 32.png is 32x16".to_owned(),
                "the Linux icons must be PNGs, and readme.txt is not a PNG, a JPEG, a Windows \
                 icon or an Apple icon"
                    .to_owned(),
                "the Linux icons must be PNGs, and 48.jpg is a JPEG".to_owned(),
            ])
        );
        assert_eq!(
            linux_icons(&zip_of(&[("icons/", b""), ("__MACOSX/._16.png", b"")])),
            Err(vec![
                "the Linux icons must be PNGs, and the zip holds none".to_owned()
            ])
        );
        let refused = linux_icons(&png_file(16, 16)).unwrap_err();
        assert!(
            matches!(refused.as_slice(), [only] if only.starts_with("cannot be read as a zip: ")),
            "{refused:?}"
        );
    }

    #[test]
    fn a_damaged_file_in_the_linux_icons_is_named_with_the_rest() {
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["16.png", "32.png", "48.png"] {
            zip.start_file(name, stored).unwrap();
            io::Write::write_all(&mut zip, &png_file(16, 16)).unwrap();
        }
        let mut bytes = zip.finish().unwrap().into_inner();
        let at = |pattern: &[u8], nth: usize| {
            bytes
                .windows(pattern.len())
                .enumerate()
                .filter(|(_, window)| *window == pattern)
                .nth(nth)
                .unwrap()
                .0
        };
        // 32.png's PNG signature no longer matches its checksum.
        let damaged = at(b"\x89PNG", 1).checked_add(1).unwrap();
        // 48.png's entry in the central directory names bzip2, which steamship does not read.
        let method = at(b"PK\x01\x02", 2).checked_add(10).unwrap();
        *bytes.get_mut(damaged).expect("a byte of the PNG") = b'Q';
        *bytes.get_mut(method).expect("the method's byte") = 12;

        let said = linux_icons(&bytes).unwrap_err();
        assert!(
            matches!(
                said.as_slice(),
                [checksum, unsupported]
                    if checksum.starts_with("32.png cannot be read from the zip: ")
                        && unsupported.starts_with("cannot be read as a zip: ")
            ),
            "{said:?}"
        );
    }

    #[test]
    fn a_folder_is_checked_file_by_file_and_other_files_are_left_alone() {
        let folder = tempfile::tempdir().unwrap();
        // A 920x430 PNG's signature and header, and nothing after: enough to know its size.
        let mut header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        header.extend_from_slice(&920_u32.to_be_bytes());
        header.extend_from_slice(&430_u32.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        header.extend_from_slice(b"\x00\x00\x00\x00IEND\x00\x00\x00\x00");
        fs::write(folder.path().join("header_capsule.png"), &header).unwrap();
        fs::write(folder.path().join("library_header.PNG"), &header).unwrap();
        fs::write(folder.path().join("capsule.png"), &header).unwrap();
        fs::write(folder.path().join("screenshot_1.jpg"), "not a picture").unwrap();
        fs::write(folder.path().join("notes.txt"), "left alone").unwrap();
        fs::create_dir_all(folder.path().join("old.png")).unwrap();

        fs::write(
            folder.path().join("linux_icons.zip"),
            zip_of(&[("16.png", &header)]),
        )
        .unwrap();
        fs::write(folder.path().join("mac_icon.icns"), "not an icon").unwrap();

        let checked = check(folder.path()).unwrap();
        let said: Vec<(String, Result<String, Vec<String>>)> = checked
            .iter()
            .map(|file| {
                (
                    file.path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    file.found.clone(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                (
                    "capsule.png".to_owned(),
                    Err(vec![
                        "is named for no asset: name it header_capsule, screenshot_01, \
                         library_logo and so on"
                            .to_owned()
                    ])
                ),
                (
                    "header_capsule.png".to_owned(),
                    Ok("header capsule, 920x430 PNG".to_owned())
                ),
                (
                    "library_header.PNG".to_owned(),
                    Ok("library header, 920x430 PNG".to_owned())
                ),
                (
                    "linux_icons.zip".to_owned(),
                    Err(vec![
                        "the Linux icons must be square, and 16.png is 920x430".to_owned()
                    ])
                ),
                (
                    "mac_icon.icns".to_owned(),
                    Err(vec![
                        "is not a PNG, a JPEG, a Windows icon or an Apple icon".to_owned()
                    ])
                ),
                (
                    "screenshot_1.jpg".to_owned(),
                    Err(vec![
                        "is not a PNG, a JPEG, a Windows icon or an Apple icon".to_owned()
                    ])
                ),
            ]
        );
        assert_eq!(too_few_screenshots(&checked), Some(1));
        assert!(
            check(&folder.path().join("absent"))
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn screenshots_are_too_few_only_when_there_are_some_but_not_five() {
        let screenshot = |number: usize| Checked {
            path: PathBuf::from(format!("screenshot_{number}.png")),
            found: Err(Vec::new()),
        };
        let some: Vec<Checked> = (1..=4).map(screenshot).collect();
        assert_eq!(too_few_screenshots(&some), Some(4));
        let enough: Vec<Checked> = (1..=5).map(screenshot).collect();
        assert_eq!(too_few_screenshots(&enough), None);
        assert_eq!(too_few_screenshots(&[]), None);
    }
}
