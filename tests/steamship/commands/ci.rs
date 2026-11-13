//! `steamship ci`.

#[cfg(windows)]
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::common::git::git;
use steamship::ci;
#[cfg(unix)]
use steamship::platform::Platform;
#[cfg(unix)]
use steamship::run;
#[cfg(unix)]
use steamship::steamcmd;

#[cfg(unix)]
use crate::write_program;
use crate::{
    Session, answering, checking, failure, packed_login, project, steamship, steamship_in,
};

/// A packed login without the day it was packed, for one `steamship ci` packed today, which a
/// test cannot know ahead.
fn undated(packed: &str) -> String {
    let rest = packed.strip_prefix("steamship-login-2:").unwrap();
    rest.split_once(':').unwrap().1.to_owned()
}

#[test]
fn ci_with_the_script_named_goes_on_to_the_account() {
    let (_project, root) = github_project();
    let home = tempfile::tempdir().unwrap();
    let script = root.join("steam").join("app_build.vdf");
    let (code, stdout, stderr) = steamship(
        &["ci", script.to_str().unwrap(), "--output", "unused"],
        Some(home.path()),
        &[("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains(&format!("  script    {}\n", script.display())),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "no build account is named. Name it with --account or STEAMSHIP_ACCOUNT"
    );
}

#[test]
fn ci_in_a_repository_without_an_app_build_script_says_so_and_exits_2() {
    let empty = tempfile::tempdir().unwrap();
    git(empty.path(), &["init", "--quiet"]);
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(empty.path(), &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert!(
        failure(&stderr).starts_with("no app build script in "),
        "{stderr}"
    );
    assert!(
        stderr.contains(". Give the script or its app ID as the first word"),
        "{stderr}"
    );
}

#[test]
fn ci_refuses_a_secret_github_would_not_take_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship(&["ci", "--secret", "GITHUB_LOGIN"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32));
    assert!(
        failure(&stderr).starts_with("GITHUB_LOGIN cannot name a GitHub secret"),
        "{stderr}"
    );
}

#[test]
fn ci_outside_a_repository_asks_for_the_script_and_exits_2() {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(outside.path(), &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert_eq!(
        failure(&stderr),
        "this folder is in no Git repository. Run steamship ci in the game's, or give the \
         script or its app ID as the first word"
    );
}

#[test]
fn ci_checks_the_scripts_alone_before_the_build_exists_but_needs_a_github_origin_and_exits_2() {
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    fs::remove_dir_all(root.join("content")).unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = steamship_in(&root, &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    let found = Path::new("steam").join("app_build.vdf");
    assert!(
        stdout.contains(&format!("  script    {}, found\n", found.display())),
        "{stdout}"
    );
    assert!(
        stdout.contains("  app       \u{2713} 1000, 1 depot, scripts checked\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  content   not checked (each upload checks it)\n"),
        "{stdout}"
    );
    assert!(
        failure(&stderr).starts_with("the script's repository has no GitHub origin"),
        "{stderr}"
    );
}

#[test]
fn ci_with_several_scripts_and_nobody_to_ask_names_them_and_exits_2() {
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    let demo = fs::read_to_string(root.join("steam/app_build.vdf"))
        .unwrap()
        .replace("\"1000\"", "\"1001\"");
    fs::write(root.join("steam/app_build_demo.vdf"), demo).unwrap();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(&root, &["ci"], Some(home.path()), &[]);
    assert_eq!(code, Some(2_i32), "{stderr}");
    let steam = Path::new("steam");
    assert_eq!(
        failure(&stderr),
        format!(
            "several app build scripts, {}, {}. Give the script or its app ID as the first word",
            steam.join("app_build.vdf").display(),
            steam.join("app_build_demo.vdf").display()
        )
    );
    let (picked, said, _) = steamship_in(
        &root,
        &["ci", "1001", "--output", "unused"],
        Some(home.path()),
        &[("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!(picked, Some(2_i32), "{said}");
    assert!(
        said.contains(&format!(
            "  script    {}, found\n",
            steam.join("app_build_demo.vdf").display()
        )),
        "{said}"
    );
}

#[test]
fn ci_writes_the_checked_login_to_a_file_for_another_ci() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    let output = project.path().join("login.txt");
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci", "--output", output.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  steam     \u{2713} Steam takes the saved login\n"),
        "{stdout}"
    );
    assert!(stdout.contains("\u{2713} ready for CI\n"), "{stdout}");
    let written = fs::read_to_string(&output).unwrap();
    assert_eq!(undated(&written), undated(&packed_login()));
    assert!(!stdout.contains(&written), "the login is never shown");
    let unpacked = ci::Login::unpack(&written).unwrap();
    assert_eq!(unpacked.account().name(), "build_bot");
    assert!(unpacked.packed_on().is_some());
}

#[test]
fn ci_with_no_saved_login_says_to_log_in_first_and_exits_3() {
    let home = answering("", 0);
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    let output = project.path().join("login.txt");
    let (code, _, stderr) = steamship_in(
        &root,
        &["ci", "--output", output.to_str().unwrap()],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(3_i32), "{stderr}");
    assert!(
        stderr.ends_with("log in here first with steamship login\n"),
        "{stderr}"
    );
    assert!(!output.exists());
}

#[test]
fn ci_says_whether_its_account_was_named_or_remembered() {
    let home = answering("", 0);
    let (project, script) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    let output = project.path().join("login.txt");
    let run = |extra: &[&str]| {
        let mut arguments = vec![
            "ci",
            script.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ];
        arguments.extend_from_slice(extra);
        steamship_in(&root, &arguments, Some(home.path()), &[])
    };
    let (named, named_said, named_why) = run(&["--account", "build_bot"]);
    assert_eq!(named, Some(3_i32), "{named_said}{named_why}");
    assert!(
        named_said.contains("  account   as named\n"),
        "{named_said}"
    );
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (kept, kept_said, kept_why) = run(&[]);
    assert_eq!(kept, Some(3_i32), "{kept_said}{kept_why}");
    assert!(
        kept_said.contains("  account   remembered\n"),
        "{kept_said}"
    );
}

/// The game's repository from [`project`], cloned from GitHub as far as its `origin` says.
fn github_project() -> (tempfile::TempDir, PathBuf) {
    let (project, _) = project(true);
    let root = project.path().join("gate (x86) 1a2b");
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/Aureliolo/some-game.git",
        ],
    );
    (project, root)
}

/// A stand-in for the GitHub command line, first on a `PATH` that is otherwise the test's own.
/// `gh secret set` keeps what it was given and how beside itself, and refuses when
/// `STEAMSHIP_FAKE_GH_REFUSE` is set; `gh api` answers a commit, `STEAMSHIP_FAKE_GH_COMMIT` if
/// set, or fails when `STEAMSHIP_FAKE_GH_OFFLINE` is set. Unix only: Windows would not start
/// the script, and would find the real `gh` behind it.
#[cfg(unix)]
fn fake_gh() -> (tempfile::TempDir, String) {
    use std::env;

    let bin = tempfile::tempdir().unwrap();
    let gh = bin.path().join("gh");
    write_program(
        &gh,
        "#!/bin/sh\n\
         here=\"$(dirname \"$0\")\"\n\
         case \"$1\" in\n\
         secret)\n\
         echo \"$*\" > \"$here/args\"\n\
         cat > \"$here/secret\"\n\
         [ -z \"$STEAMSHIP_FAKE_GH_REFUSE\" ] || { echo 'HTTP 403: Resource not accessible' >&2; exit 1; } ;;\n\
         api)\n\
         [ -z \"$STEAMSHIP_FAKE_GH_OFFLINE\" ] || exit 1\n\
         echo \"${STEAMSHIP_FAKE_GH_COMMIT:-0123456789abcdef0123456789abcdef01234567}\" ;;\n\
         esac\n",
    );
    let path = format!("{}:{}", bin.path().display(), env::var("PATH").unwrap());
    (bin, path)
}

#[cfg(unix)]
#[test]
fn ci_sets_the_secret_through_gh_and_shows_the_step_pinned_by_commit() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) =
        steamship_in(&root, &["ci"], Some(home.path()), &[("PATH", &path)]);
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  repo      Aureliolo/some-game, from origin\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  secret    \u{2713} STEAMSHIP_LOGIN set on Aureliolo/some-game\n"),
        "{stdout}"
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.ends_with(&format!(
            "\n      - name: Upload to Steam\n        \
             uses: Aureliolo/steamship@0123456789abcdef0123456789abcdef01234567 # v{version}\n        \
             with:\n          \
             app: steam/app_build.vdf\n          \
             version: ${{{{ github.ref_name }}}}\n          \
             login: ${{{{ secrets.STEAMSHIP_LOGIN }}}}\n"
        )),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(gh.path().join("args")).unwrap(),
        "secret set STEAMSHIP_LOGIN --repo Aureliolo/some-game\n"
    );
    let secret = fs::read_to_string(gh.path().join("secret")).unwrap();
    assert!(!stdout.contains(&secret), "the login is never shown");
    assert_eq!(undated(&secret), undated(&packed_login()));
    assert!(ci::Login::unpack(&secret).unwrap().packed_on().is_some());
}

#[cfg(unix)]
#[test]
fn ci_refuses_a_login_too_large_for_a_github_secret_before_asking_gh() {
    let home = answering(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, format!("\"token\" \"{}\"", "t".repeat(30_000))).unwrap();
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) =
        steamship_in(&root, &["ci"], Some(home.path()), &[("PATH", &path)]);
    assert_eq!(code, Some(1_i32), "{stdout}{stderr}");
    assert_eq!(
        failure(&stderr),
        "the packed login is too large for a GitHub secret: 59 KB, where a secret holds 48 KB"
    );
    assert!(
        stderr
            .ends_with("write it to a file for another CI with steamship ci --output login.txt\n"),
        "{stderr}"
    );
    assert!(!gh.path().join("args").exists(), "gh is never asked");
}

#[cfg(unix)]
#[test]
fn ci_sets_a_login_just_inside_what_a_github_secret_holds() {
    let home = answering(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let config = steamcmd::saved_login(home.path(), Platform::THIS);
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, format!("\"token\" \"{}\"", "t".repeat(24_000))).unwrap();
    fs::write(home.path().join("account"), "build_bot\n").unwrap();
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) =
        steamship_in(&root, &["ci"], Some(home.path()), &[("PATH", &path)]);
    assert_eq!(code, Some(0_i32), "{stdout}{stderr}");
    let secret = fs::read_to_string(gh.path().join("secret")).unwrap();
    assert!(
        (47_104..=ci::SECRET_LIMIT).contains(&secret.len()),
        "{} bytes set",
        secret.len()
    );
}

#[cfg(unix)]
#[test]
fn ci_takes_the_repository_and_secret_named_and_pins_the_tag_when_github_cannot_be_asked() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (gh, path) = fake_gh();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &[
            "ci",
            "--repo",
            "Aureliolo/other-game",
            "--secret",
            "STEAM_UPLOAD",
        ],
        Some(home.path()),
        &[("PATH", &path), ("STEAMSHIP_FAKE_GH_OFFLINE", "1")],
    );
    assert_eq!((code, stderr.as_str()), (Some(0_i32), ""), "{stdout}");
    assert!(
        stdout.contains("  repo      Aureliolo/other-game, as named\n"),
        "{stdout}"
    );
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(&format!(
            "uses: Aureliolo/steamship@v{version} # v{version}\n"
        )),
        "{stdout}"
    );
    assert!(
        stdout.contains("login: ${{ secrets.STEAM_UPLOAD }}\n"),
        "{stdout}"
    );
    assert_eq!(
        fs::read_to_string(gh.path().join("args")).unwrap(),
        "secret set STEAM_UPLOAD --repo Aureliolo/other-game\n"
    );
}

#[cfg(unix)]
#[test]
fn ci_says_what_gh_answered_when_it_cannot_set_the_secret_and_exits_1() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (_gh, path) = fake_gh();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", &path), ("STEAMSHIP_FAKE_GH_REFUSE", "1")],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} not set\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "gh could not set the secret: HTTP 403: Resource not accessible"
    );
    assert!(
        stderr.ends_with("if gh is not logged in, run gh auth login\n"),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn ci_without_gh_says_where_to_get_it_and_exits_1() {
    use std::os::unix::fs::symlink;

    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let bin = tempfile::tempdir().unwrap();
    let git = run::output(run::command("sh").args(["-c", "command -v git"])).unwrap();
    let git = String::from_utf8(git.stdout).unwrap();
    symlink(git.trim(), bin.path().join("git")).unwrap();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", bin.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} gh is not installed\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "gh, the GitHub command line, is needed to set the secret"
    );
    assert!(
        stderr.ends_with(
            "install it from https://cli.github.com, or write the login to a file with --output\n"
        ),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn ci_with_a_gh_that_cannot_start_says_so_and_exits_1() {
    use std::os::unix::fs::symlink;

    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let bin = tempfile::tempdir().unwrap();
    let git = run::output(run::command("sh").args(["-c", "command -v git"])).unwrap();
    symlink(
        String::from_utf8(git.stdout).unwrap().trim(),
        bin.path().join("git"),
    )
    .unwrap();
    fs::write(bin.path().join("gh"), "not a program").unwrap();
    let (code, stdout, stderr) = steamship_in(
        &root,
        &["ci"],
        Some(home.path()),
        &[("PATH", bin.path().to_str().unwrap())],
    );
    assert_eq!(code, Some(1_i32), "{stdout}");
    assert!(
        stdout.ends_with("  secret    \u{2717} gh could not start\n"),
        "{stdout}"
    );
    assert!(failure(&stderr).starts_with("gh: "), "{stderr}");
}

#[cfg(unix)]
#[test]
fn ci_pins_the_tag_when_github_answers_something_that_is_not_a_commit() {
    let home = checking(
        "Logging in user 'build_bot' [U:1:0] to Steam Public...OK",
        0,
    );
    let (_project, root) = github_project();
    let (_gh, path) = fake_gh();
    let version = env!("CARGO_PKG_VERSION");
    for answer in ["0123abc", &"g".repeat(40)] {
        let (code, stdout, _) = steamship_in(
            &root,
            &["ci"],
            Some(home.path()),
            &[("PATH", &path), ("STEAMSHIP_FAKE_GH_COMMIT", answer)],
        );
        assert_eq!(code, Some(0_i32), "{stdout}");
        assert!(
            stdout.contains(&format!(
                "uses: Aureliolo/steamship@v{version} # v{version}\n"
            )),
            "{answer}: {stdout}"
        );
    }
}

#[test]
fn ci_run_outside_the_repository_reads_origin_where_the_script_is() {
    let (_project, root) = github_project();
    let outside = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let script = root.join("steam").join("app_build.vdf");
    let (code, stdout, stderr) = steamship_in(
        outside.path(),
        &["ci", script.to_str().unwrap()],
        Some(home.path()),
        &[("STEAMSHIP_ACCOUNT", "")],
    );
    assert_eq!(code, Some(2_i32), "{stdout}{stderr}");
    assert!(
        stdout.contains("  repo      Aureliolo/some-game, from origin\n"),
        "{stdout}"
    );
    assert_eq!(
        failure(&stderr),
        "no build account is named. Name it with --account or STEAMSHIP_ACCOUNT"
    );
}

#[test]
fn ci_refuses_a_repository_that_is_not_an_owner_and_name_and_exits_2() {
    let (_project, root) = github_project();
    let home = tempfile::tempdir().unwrap();
    let (code, _, stderr) = steamship_in(
        &root,
        &["ci", "--repo", "Aureliolo/some-game/extra"],
        Some(home.path()),
        &[],
    );
    assert_eq!(code, Some(2_i32), "{stderr}");
    assert_eq!(
        failure(&stderr),
        "Aureliolo/some-game/extra is not a repository's owner/name"
    );
}

#[test]
fn ci_on_a_terminal_asks_which_script_when_there_are_several() {
    let (_project, root) = github_project();
    let _: u64 = fs::copy(
        root.join("steam/app_build.vdf"),
        root.join("steam/app_build_demo.vdf"),
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let steamship = env!("CARGO_BIN_EXE_steamship");
    // As in login_then_look: `cmd` reads no quoting, so the path is written without.
    #[cfg(windows)]
    let line = format!(
        "cd /d {} & {steamship} ci --account build_bot",
        root.display()
    );
    #[cfg(unix)]
    let line = format!(
        "cd '{}' && '{steamship}' ci --account build_bot",
        root.display()
    );
    let mut session = Session::start(&line, home.path());
    session.wait_for("script");
    session.wait_for("2");
    session.type_in("2\r");
    session.wait_for("log in here first");
    let _: Option<i32> = session.end().wait().unwrap();
    let seen = session.seen();
    let demo = Path::new("steam").join("app_build_demo.vdf");
    assert!(
        seen.contains(&format!("  2         {}\n", demo.display())),
        "{seen}"
    );
    assert!(
        seen.contains("\u{2713} 1000, 1 depot, scripts checked"),
        "{seen}"
    );
}
