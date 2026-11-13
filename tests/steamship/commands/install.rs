//! `steamship install`.

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::Path;
use std::thread;
use std::time::Duration;

use steamship::account::REMEMBERED;
use steamship::platform::Platform;
use steamship::update::KEPT;
use steamship::upload::APPS;
use steamship::workshop::ITEMS;
use steamship::{fetch, install, ledger, steamcmd};

use crate::{
    failure, faked_with, project, recorded, saved_login, steamship, upload, workshop_item,
};

/// `steamship install` with `STEAMSHIP_HOME` set to `home`, or with no environment at all.
fn run(home: Option<&Path>) -> (Option<i32>, String, String) {
    steamship(&["install"], home, &[])
}

#[test]
fn install_says_when_the_install_is_as_pinned() {
    let home = recorded("");
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(stdout.starts_with("steamship install\n"), "{stdout}");
    assert!(stdout.contains(", pinned and verified\n"), "{stdout}");
    let folder = home.path().join(install::FOLDER);
    assert!(
        stdout.ends_with(&format!("  folder    {}\n", folder.display())),
        "{stdout}"
    );
}

#[test]
fn with_times_each_step_ends_with_how_long_it_took() {
    let home = recorded("");
    let (code, stdout, stderr) = steamship(&["install", "--times"], Some(home.path()), &[]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    let verified = stdout
        .lines()
        .find(|line| line.contains(", pinned and verified"))
        .unwrap_or_else(|| panic!("{stdout}"));
    let took = verified
        .strip_suffix(" s)")
        .and_then(|line| line.rsplit_once(", pinned and verified ("))
        .map(|(_, took)| took);
    assert!(
        took.is_some_and(|took| took.parse::<f64>().is_ok() && took.contains('.')),
        "{verified}"
    );
}

#[test]
fn install_names_what_changed_and_exits_4() {
    let home = recorded(&format!("file {} steamcmd.exe\n", "0".repeat(64)));
    let (code, stdout, stderr) = run(Some(home.path()));
    assert_eq!(code, Some(4_i32));
    assert!(
        stdout.ends_with("\u{2717} differs from the pinned version\n"),
        "{stdout}"
    );
    assert!(
        stderr.trim_end().ends_with("steamcmd.exe is missing"),
        "{stderr}"
    );
}

#[test]
fn install_waits_for_another_steamship_using_the_home_then_goes_on() {
    let home = recorded("");
    let held = File::create(home.path().join("steamship.lock")).unwrap();
    held.try_lock().unwrap();
    let path = home.path().to_path_buf();
    let second = thread::spawn(move || run(Some(&path)));
    thread::sleep(Duration::from_secs(2));
    assert!(!second.is_finished(), "it waits while the home is held");
    held.unlock().unwrap();
    let (code, stdout, stderr) = second.join().unwrap();
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""));
    assert!(
        stdout.contains(&format!(
            "  steamcmd  waiting for another steamship using {} to finish...\n",
            home.path().display()
        )) && stdout.contains("\u{2713} free after ")
            && stdout.contains(", pinned and verified\n"),
        "{stdout}"
    );
}

#[test]
fn install_says_where_it_could_not_find_a_home_and_exits_1() {
    let (code, _, stderr) = run(None);
    assert_eq!(code, Some(1_i32));
    assert!(
        failure(&stderr).starts_with("neither STEAMSHIP_HOME nor "),
        "{stderr}"
    );
}

/// The page that describes the home, as it is in the repository.
fn install_page() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/install.md")).unwrap()
}

/// A steamcmd that builds an upload as `BuildID` 4242 and publishes a Workshop item as 777.
const BUILDING_AND_PUBLISHING: &str = "#!/bin/sh\n\
    case \" $* \" in\n\
    *' +run_app_build '*)\n\
    output=\"$HOME/apps/1000/output\"\n\
    mkdir -p \"$output\"\n\
    echo 'Successfully finished AppID 1000 build (BuildID 4242).' > \"$output/app_build_1000.log\";;\n\
    *' +workshop_build_item '*)\n\
    while [ $# -gt 0 ]; do [ \"$1\" = +workshop_build_item ] && script=\"$2\"; shift; done\n\
    printf '\"workshopitem\"\\n{\\n\"appid\" \"480\"\\n\"publishedfileid\" \"777\"\\n}\\n' > \"$script\"\n\
    echo 'Success.';;\n\
    esac\n\
    echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n";

#[test]
fn the_install_page_names_everything_a_run_leaves_in_the_home() {
    let home = faked_with(BUILDING_AND_PUBLISHING);
    saved_login(home.path());
    let (_project, script) = project(true);
    let (_item, item) = workshop_item("");
    let item = item.to_str().unwrap();
    let runs = [
        steamship(&["status"], Some(home.path()), &[]),
        upload(&script, home.path(), &["--version", "1.4.0"], &[]),
        steamship(
            &["workshop", item, "--account", "build_bot"],
            Some(home.path()),
            &[],
        ),
    ];
    for (code, stdout, stderr) in runs {
        assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    }
    let page = install_page();
    for entry in fs::read_dir(home.path()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            page.contains(&format!("`{name}`")) || page.contains(&format!("`{name}/")),
            "docs/install.md does not name {name}, which a run left in the home"
        );
    }
}

#[test]
fn the_install_page_names_each_place_the_code_does() {
    let page = install_page();
    let named = [
        install::FOLDER,
        install::STAGING,
        install::PREVIOUS,
        install::DOWNLOADS,
        KEPT,
        APPS,
        ITEMS,
        ledger::FOLDER,
        REMEMBERED,
    ];
    for name in named {
        assert!(
            page.contains(&format!("`{name}`")) || page.contains(&format!("`{name}/")),
            "docs/install.md does not name {name}"
        );
    }
    let home = Path::new("home");
    let fetched = fetch::folder(home, 1000, "testing", Platform::Linux);
    let fetched = fetched
        .strip_prefix(home)
        .unwrap()
        .components()
        .next()
        .unwrap();
    let fetched = fetched.as_os_str().to_string_lossy();
    assert!(page.contains(&format!("`{fetched}/")), "{fetched}");
    for platform in [Platform::Windows, Platform::Linux, Platform::MacOs] {
        let token = steamcmd::saved_login(home, platform);
        let token = token
            .strip_prefix(home)
            .unwrap()
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        assert!(
            page.contains(&token),
            "docs/install.md does not name {token}"
        );
    }
    let defaults = [
        (
            Platform::Windows,
            "%LOCALAPPDATA%\\steamship",
            "LOCALAPPDATA",
        ),
        (Platform::Linux, "$XDG_DATA_HOME/steamship", "XDG_DATA_HOME"),
        (Platform::Linux, "~/.local/share/steamship", "HOME"),
        (
            Platform::MacOs,
            "~/Library/Application Support/steamship",
            "HOME",
        ),
    ];
    for (platform, written, variable) in defaults {
        assert!(
            page.contains(written),
            "docs/install.md does not hold {written}"
        );
        let parts: Vec<&str> = written.split(['/', '\\']).collect();
        let base = parts.first().copied().unwrap();
        let found = platform
            .home(|name| (name == variable).then(|| OsString::from(base)))
            .unwrap();
        let found: Vec<String> = found
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, parts, "the default home on {platform:?}");
    }
    let caching = page
        .split("\n\n")
        .find(|paragraph| paragraph.contains("keep `apps`"))
        .unwrap();
    assert!(caching.contains("`apps/*/output/*.log`"), "{caching}");
}
