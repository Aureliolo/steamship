//! `steamship settings`.

use std::fs;

use steamship::platform::Platform;
use steamship::steamcmd;

use crate::{
    CUT_OFF, CUT_OFF_SAID, failure, faked_with, packed_login, project, saved_login, steamship,
    web_api, web_api_variables,
};

#[test]
fn app_info_cut_off_before_its_closing_brace_fails_settings_and_promote_saying_it_cannot_be_read() {
    let home = faked_with(CUT_OFF);
    saved_login(home.path());
    let (_project, script) = project(false);
    let script = script.to_str().unwrap();
    let (silent, _unasked) = web_api::<&str>(vec![]);
    // Each names what its command asked for.
    for (args, asked) in [
        (["settings", "1000"].as_slice(), "settings"),
        (
            ["promote", script, "--build", "1234", "--branch", "public"].as_slice(),
            "app info",
        ),
    ] {
        let (code, stdout, stderr) =
            steamship(args, Some(home.path()), &web_api_variables(&silent));
        assert_eq!(code, Some(1_i32), "{args:?}: {stdout}{stderr}");
        assert_eq!(
            failure(&stderr),
            format!(
                "{}. Run the command again",
                CUT_OFF_SAID.replace("app info", asked)
            ),
            "{args:?}: {stdout}"
        );
    }
}

/// A steamcmd that prints app 1000's info as steamcmd 1788292693 printed Falcon Ridge
/// Manager's, trimmed, with the executable `executable`, keeping what it was started with in
/// `args`.
fn app_info(executable: &str) -> String {
    format!(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo 'Logging in using cached credentials.'\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         echo 'AppID : 1000, change number : 5/5, last change : Tue Sep 29 20:07:48 2026 '\n\
         printf '\"1000\"\\n{{\\n'\n\
         printf '\\t\"common\"\\n\\t{{\\n\\t\\t\"name\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"ReleaseState\"\\t\\t\"unavailable\"\\n\\t}}\\n'\n\
         printf '\\t\"config\"\\n\\t{{\\n\\t\\t\"installdir\"\\t\\t\"Spacewar\"\\n'\n\
         printf '\\t\\t\"launch\"\\n\\t\\t{{\\n\\t\\t\\t\"0\"\\n\\t\\t\\t{{\\n'\n\
         printf '\\t\\t\\t\\t\"executable\"\\t\\t\"{executable}\"\\n\\t\\t\\t}}\\n\\t\\t}}\\n\\t}}\\n'\n\
         printf '\\t\"depots\"\\n\\t{{\\n\\t\\t\"1001\"\\n\\t\\t{{\\n'\n\
         printf '\\t\\t\\t\"manifests\"\\n\\t\\t\\t{{\\n\\t\\t\\t}}\\n\\t\\t}}\\n\\t}}\\n}}\\n'\n\
         echo 'Unloading Steam API...OK'\n"
    )
}

#[test]
fn settings_are_shown_saved_and_checked_leaving_steams_bookkeeping_out() {
    let home = faked_with(&app_info("spacewar"));
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];

    let (code, stdout, stderr) = steamship(&["settings", "1000"], Some(home.path()), &quiet);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("\"1000\"\n{\n\t\"common\"\n\t{\n\t\t\"name\"\t\t\"Spacewar\"\n\t}\n")
            && stdout.contains("\t\t\t\t\"executable\"\t\t\"spacewar\"\n")
            && stdout.contains("\t\"depots\"\n\t{\n\t\t\"1001\"\n\t\t{\n\t\t}\n\t}\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("ReleaseState") && !stdout.contains("manifests"));
    assert!(
        stdout.contains("  cloud     Steam Cloud not set up in Steamworks\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "  input     no Default Controller Configuration in Steamworks, and no touch default"
        ),
        "{stdout}"
    );
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(
        args.contains("+login build_bot +app_info_update 1 +app_info_print 1000 +quit"),
        "{args}"
    );

    let kept = files.path().join("settings.vdf");
    let kept_text = kept.to_string_lossy().into_owned();
    let (saved, said, _) = steamship(
        &["settings", "1000", "--save", &kept_text],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(saved, Some(0_i32), "{said}");
    assert!(said.contains("app 1000: settings saved to "), "{said}");
    let (matched, checked, _) = steamship(
        &["settings", "1000", "--check", &kept_text],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(matched, Some(0_i32), "{checked}");
    assert!(
        checked.contains("Steam's settings match the file"),
        "{checked}"
    );

    // Someone changed the executable in Steamworks.
    let changed = faked_with(&app_info("spacewar.x86_64"));
    saved_login(changed.path());
    let (drifted, shown, why) = steamship(
        &["settings", "1000", "--check", &kept_text],
        Some(changed.path()),
        &quiet,
    );
    assert_eq!(drifted, Some(2_i32), "{shown}{why}");
    assert!(
        why.contains(
            "config/launch/0/executable is \"spacewar.x86_64\" on Steam and \"spacewar\" in the \
             file"
        ),
        "{why}"
    );
    assert!(
        shown.contains("or run steamship settings --save again if the change was meant"),
        "{shown}"
    );
}

#[test]
fn a_snapshot_is_named_after_the_app() {
    let home = faked_with(&app_info("spacewar"));
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let kept = files.path().join("kept.vdf");
    let kept = kept.to_string_lossy().into_owned();
    let (saved, said, why) = steamship(
        &["settings", "1000", &kept, "--save"],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(saved, Some(0_i32), "{said}{why}");
    for line in [
        ["settings", "1000", &kept, "--check"],
        ["settings", "1000", "--check", &kept],
    ] {
        let (code, checked, stderr) = steamship(&line, Some(home.path()), &quiet);
        assert_eq!(code, Some(0_i32), "{line:?}: {checked}{stderr}");
        assert!(
            checked.contains(&format!("  file      {kept}\n"))
                && checked.contains("Steam's settings match the file"),
            "{line:?}: {checked}"
        );
    }
}

#[test]
fn settings_in_ci_are_asked_for_as_the_account_handed_over() {
    // No login saved and no account remembered: only what `steamship ci` packed.
    let home = faked_with(&app_info("spacewar"));
    let (code, stdout, stderr) = steamship(
        &["settings", "1000"],
        Some(home.path()),
        &[
            ("STEAMSHIP_LOGIN", &packed_login()),
            ("STEAMSHIP_NO_UPDATE_CHECK", "1"),
        ],
    );
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let args = fs::read_to_string(home.path().join("args")).unwrap();
    assert!(args.contains("+login build_bot "), "{args}");
    assert!(steamcmd::saved_login(home.path(), Platform::THIS).exists());
}

#[test]
fn a_snapshot_for_another_app_is_refused_before_steamcmd_and_an_app_steam_hides_is_a_failure() {
    let home = faked_with(
        "#!/bin/sh\n\
         echo \"$*\" > \"$HOME/args\"\n\
         echo \"Logging in user 'build_bot' [U:1:1] to Steam Public...OK\"\n\
         printf '\"1000\"\\n{\\n}\\n'\n",
    );
    saved_login(home.path());
    let files = tempfile::tempdir().unwrap();
    let quiet = [("STEAMSHIP_NO_UPDATE_CHECK", "1")];
    let other = files.path().join("other.vdf");
    fs::write(&other, "\"480\"\n{\n}\n").unwrap();
    let (code, _, stderr) = steamship(
        &["settings", "1000", "--check", &other.to_string_lossy()],
        Some(home.path()),
        &quiet,
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(stderr.contains("is for app 480, not app 1000"), "{stderr}");
    assert!(!home.path().join("args").exists(), "steamcmd was not run");

    let (hidden, _, why) = steamship(&["settings", "1000"], Some(home.path()), &quiet);
    assert_eq!(hidden, Some(1_i32), "{why}");
    // Running it again would show the same, so the step is the one the message gives.
    assert!(
        why.contains("Steam shows the build account no settings for app 1000: ")
            && why.contains("permission group that holds the app")
            && !why.contains("Run the command again"),
        "{why}"
    );
}
