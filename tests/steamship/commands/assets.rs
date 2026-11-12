//! `steamship assets`.

use std::fs;
use std::io::{self, Write as _};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::{png_header, steamship};

#[test]
fn artwork_is_checked_against_valves_sizes_with_nothing_sent() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("header_capsule.png"),
        png_header(920, 430),
    )
    .unwrap();
    fs::write(
        folder.path().join("small_capsule.png"),
        png_header(462, 174),
    )
    .unwrap();
    fs::write(folder.path().join("notes.txt"), "left alone").unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  header_capsule.png header capsule, 920x430 PNG\n")
            && stdout.contains("  small_capsule.png  small capsule, 462x174 PNG\n")
            && stdout.contains("the artwork is as Steamworks takes it"),
        "lined up however long the names run: {stdout}"
    );

    fs::write(
        folder.path().join("library_hero.png"),
        png_header(1920, 620),
    )
    .unwrap();
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("library_hero.png: the library hero must be 3840x1240, and it is 1920x620"),
        "{why}"
    );

    let empty = tempfile::tempdir().unwrap();
    let (none, _, said) = steamship(&["assets", &empty.path().to_string_lossy()], None, &quiet);
    assert_eq!(none, Some(2_i32), "{said}");
    assert!(
        said.contains("holds no PNG, JPEG, icon or zip, nor a folder for a language with one"),
        "{said}"
    );
}

#[test]
fn a_store_folder_is_checked_with_its_folders_for_each_language_and_the_rest_named() {
    let folder = tempfile::tempdir().unwrap();
    for inner in ["de", "pt_BR", "achievements"] {
        fs::create_dir_all(folder.path().join(inner)).unwrap();
    }
    fs::write(
        folder.path().join("de/header_capsule.png"),
        png_header(920, 430),
    )
    .unwrap();
    fs::write(
        folder.path().join("pt_BR/small_capsule.png"),
        png_header(462, 174),
    )
    .unwrap();
    fs::write(
        folder.path().join("achievements/FIRST_POUR.png"),
        png_header(64, 64),
    )
    .unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  skipped   achievements, named for no language\n")
            && stdout
                .contains("  de/header_capsule.png   header capsule for german, 920x430 PNG\n")
            && stdout
                .contains("  pt_BR/small_capsule.png small capsule for brazilian, 462x174 PNG\n"),
        "{stdout}"
    );

    fs::write(
        folder.path().join("de/small_capsule.png"),
        png_header(460, 174),
    )
    .unwrap();
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("de/small_capsule.png: the small capsule must be 462x174, and it is 460x174"),
        "{why}"
    );
}

#[test]
fn the_mac_and_linux_icons_are_checked_with_every_wrong_file_in_the_zip_named() {
    let folder = tempfile::tempdir().unwrap();
    let zipped = |entries: &[(&str, Vec<u8>)]| {
        let mut zip = ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, contents) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        fs::write(
            folder.path().join("linux_icons.zip"),
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
    };
    zipped(&[
        ("icons/16.png", png_header(16, 16)),
        ("icons/32.png", png_header(32, 32)),
    ]);
    // An Apple icon of one 128x128 image, with a byte of it.
    fs::write(
        folder.path().join("mac_icon.icns"),
        b"icns\x00\x00\x00\x11it32\x00\x00\x00\x09\x00",
    )
    .unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let path = folder.path().to_string_lossy().into_owned();

    let (code, stdout, stderr) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  linux_icons.zip Linux icons, 2 PNGs from 16x16 to 32x32\n")
            && stdout.contains("  mac_icon.icns   Mac icon, 128x128 Apple icon\n"),
        "{stdout}"
    );

    zipped(&[
        ("icons/16.png", png_header(16, 16)),
        ("icons/32.png", png_header(32, 30)),
        ("icons/notes.txt", b"drawn by hand".to_vec()),
    ]);
    let (wrong, _, why) = steamship(&["assets", &path], None, &quiet);
    assert_eq!(wrong, Some(2_i32), "{why}");
    assert!(
        why.contains("linux_icons.zip: the Linux icons must be square, and icons/32.png is 32x30")
            && why.contains(
                "linux_icons.zip: the Linux icons must be PNGs, and icons/notes.txt is not a PNG, \
             a JPEG, a Windows icon or an Apple icon"
            ),
        "{why}"
    );
}
