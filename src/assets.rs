//! Steamworks artwork checked against Valve's sizes and formats before it is uploaded by hand,
//! each file named for the asset it is: `header_capsule.png`, `library_logo.png`,
//! `screenshot_03.jpg`.
//!
//! The sizes are Valve's, from Steamworks' documentation of store, library, community and event
//! assets.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::picture::{self, Format, Picture};

/// How large an asset must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    Exact(u32, u32),
    /// A screenshot: at least 1920 by 1080, and 16:9.
    Widescreen,
    /// The library logo: 1280 wide, 720 tall, or both, and neither larger.
    Logo,
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
    /// An icon or a PNG, for the shortcut icon.
    IconOrPng,
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
const ASSETS: [Asset; 15] = [
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
        Size::Exact(256, 256),
        Formats::IconOrPng,
    ),
    asset("app icon", "app_icon", Size::Exact(184, 184), Formats::Jpeg),
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
];

/// Steamworks' least number of screenshots for a store page.
pub const SCREENSHOTS: usize = 5;

/// The extensions of the files looked at; anything else in the folder is left alone.
const PICTURES: [&str; 4] = ["png", "jpg", "jpeg", "ico"];

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
    let format_wanted = match (asset.formats, picture.format) {
        (Formats::Picture, Format::Png { .. } | Format::Jpeg)
        | (Formats::Png, Format::Png { .. })
        | (Formats::SeeThroughPng, Format::Png { transparent: true })
        | (Formats::Jpeg, Format::Jpeg)
        | (Formats::IconOrPng, Format::Ico | Format::Png { .. }) => None,
        (Formats::Picture, Format::Ico) => Some("a PNG or a JPEG"),
        (Formats::Png, Format::Jpeg | Format::Ico) => Some("a PNG"),
        (Formats::SeeThroughPng, Format::Png { transparent: false }) => {
            Some("a PNG that can be see-through, with an alpha channel")
        }
        (Formats::SeeThroughPng, Format::Jpeg | Format::Ico) => {
            Some("a PNG that can be see-through")
        }
        (Formats::Jpeg, Format::Png { .. } | Format::Ico) => Some("a JPEG"),
        (Formats::IconOrPng, Format::Jpeg) => Some("an icon or a PNG"),
    };
    if let Some(wanted) = format_wanted {
        found.push(format!(
            "the {} must be {wanted}, not a {}",
            asset.name, picture.format
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
            (width < 1920 || height < 1080 || !widescreen)
                .then_some("at least 1920x1080, and 16:9".to_owned())
        }
        Size::Logo => {
            let fits = (width == 1280 && height <= 720) || (height == 720 && width <= 1280);
            (!fits).then_some("1280 wide or 720 tall, and no larger".to_owned())
        }
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
    /// The asset it is kept as, and its picture, when it is one and could be read.
    pub found: Option<(&'static Asset, Picture)>,
    pub problems: Vec<String>,
}

/// Checks every picture in `folder`, in the order of their names.
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
                PICTURES
                    .iter()
                    .any(|picture| extension.eq_ignore_ascii_case(picture))
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
    let Some(asset) = named(&stem) else {
        return Checked {
            path,
            found: None,
            problems: vec![
                "is named for no asset: name it header_capsule, screenshot_01, library_logo and \
                 so on"
                    .to_owned(),
            ],
        };
    };
    match fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| picture::read(&bytes))
    {
        Ok(picture) => Checked {
            problems: problems(asset, &picture),
            path,
            found: Some((asset, picture)),
        },
        Err(why) => Checked {
            path,
            found: None,
            problems: vec![why],
        },
    }
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
            ("app_icon", of(Format::Jpeg, 184, 184)),
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
                vec!["the header capsule must be a PNG or a JPEG, not a icon"],
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
                "app_icon",
                png(184, 184, false),
                vec!["the app icon must be a JPEG, not a PNG"],
            ),
            (
                "shortcut_icon",
                of(Format::Jpeg, 256, 256),
                vec!["the shortcut icon must be an icon or a PNG, not a JPEG"],
            ),
        ] {
            assert_eq!(problems(asset_named(stem), &picture), said, "{stem}");
        }
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

        let checked = check(folder.path()).unwrap();
        let said: Vec<(String, Vec<String>)> = checked
            .iter()
            .map(|file| {
                (
                    file.path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    file.problems.clone(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                (
                    "capsule.png".to_owned(),
                    vec![
                        "is named for no asset: name it header_capsule, screenshot_01, \
                         library_logo and so on"
                            .to_owned()
                    ]
                ),
                ("header_capsule.png".to_owned(), vec![]),
                ("library_header.PNG".to_owned(), vec![]),
                (
                    "screenshot_1.jpg".to_owned(),
                    vec!["is not a PNG, a JPEG or an icon".to_owned()]
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
            found: None,
            problems: Vec::new(),
        };
        let some: Vec<Checked> = (1..=4).map(screenshot).collect();
        assert_eq!(too_few_screenshots(&some), Some(4));
        let enough: Vec<Checked> = (1..=5).map(screenshot).collect();
        assert_eq!(too_few_screenshots(&enough), None);
        assert_eq!(too_few_screenshots(&[]), None);
    }
}
