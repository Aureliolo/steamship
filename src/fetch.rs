//! Fetching an app's build as Steam serves it to players, through steamcmd's `app_update` into a
//! folder of its own, and reading back which build that folder holds.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::platform::Platform;
use crate::terminal::{Event, Reader};
use crate::upload;
use crate::vdf;
use crate::webapi::{self, Branch};

/// The systems a build is fetched for, in the order they are listed.
const SYSTEMS: [Platform; 3] = [Platform::Windows, Platform::Linux, Platform::MacOs];

/// The word steamcmd's `sSteamCmdForcePlatformType` takes for `platform`, which is also how it is
/// given on the command line.
#[must_use]
pub const fn system_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "windows",
        Platform::Linux => "linux",
        Platform::MacOs => "macos",
    }
}

/// Reads a system given as its word, such as `linux`.
///
/// # Errors
///
/// When `text` is none of the words.
pub fn system(text: &str) -> Result<Platform, String> {
    SYSTEMS
        .into_iter()
        .find(|platform| system_name(*platform) == text)
        .ok_or_else(|| "it is windows, linux or macos".to_owned())
}

/// The branch a fetch downloads from `app`'s `branches`: the one `branch` names, or else the one
/// `build` is live on, the default first and one without a password before one with, or else the
/// default.
///
/// # Errors
///
/// When there is no such branch, no build is live on it, or it holds another build than `build`.
pub fn chosen<'branches>(
    branches: &'branches [Branch],
    app: u32,
    branch: Option<&str>,
    build: Option<u64>,
) -> Result<&'branches Branch, String> {
    let found = match (branch, build) {
        (Some(given), _) => branches
            .iter()
            .find(|found| named(given, &found.name))
            .ok_or_else(|| {
                format!(
                    "app {app} has no branch {given}; its branches are {}",
                    listed(branches)
                )
            })?,
        (None, Some(build)) => branches
            .iter()
            .filter(|found| found.build_id == build)
            .min_by_key(|found| (!webapi::is_default(&found.name), found.locked))
            .ok_or_else(|| {
                format!(
                    "build {build} is live on no branch of app {app}, and Steam serves only a \
                     branch's live build: set it live with `steamship promote --build {build} \
                     --branch <branch>`, then fetch that branch"
                )
            })?,
        (None, None) => branches
            .iter()
            .find(|found| webapi::is_default(&found.name))
            .ok_or_else(|| format!("app {app} has no default branch"))?,
    };
    let shown = webapi::shown_name(&found.name);
    if found.build_id == 0 {
        return Err(format!("no build is live on app {app}'s {shown} branch"));
    }
    if let Some(build) = build.filter(|build| *build != found.build_id) {
        return Err(format!(
            "build {build} is not live on app {app}'s {shown} branch, which has build {}",
            found.build_id
        ));
    }
    Ok(found)
}

/// Where a branch's password is read from before it is asked for.
pub const PASSWORD: &str = "STEAMSHIP_BRANCH_PASSWORD";

/// Whether `password` can be typed into steamcmd's console, which splits what is typed at each
/// space and reads quotes as its own.
#[must_use]
pub fn typeable(password: &str) -> bool {
    !password.is_empty()
        && !password
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
}

/// The command typed into steamcmd's console that fetches `app`'s `branch` with its `password`.
#[must_use]
pub fn typed_command(app: u32, branch: &str, password: &str) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "app_update {app} -beta {branch} -betapassword {password} validate"
    ))
}

/// `text` with every `secret` in it hidden, and whatever follows `-betapassword` on each line, as
/// steamcmd's console echoes what is typed into it.
#[must_use]
pub fn hidden(text: &str, secret: &str) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = line.split_once("-betapassword").map_or_else(
                || line.to_owned(),
                |(before, _)| format!("{before}-betapassword <password>"),
            );
            if secret.is_empty() {
                line
            } else {
                line.replace(secret, "<password>")
            }
        })
        .collect();
    lines.join("\n")
}

/// Types `command` at steamcmd's first `Steam>` prompt and `quit` at each one after, and answers
/// each line of its console with `secret` hidden.
///
/// The console is read from `output` to its end, or to a line holding one of `hopeless`, and typed
/// into through `input`. A prompt is seen again each time more output arrives before the line
/// ends; it is answered once.
///
/// # Errors
///
/// When the output cannot be read or the input written.
pub fn typed<Output, Input>(
    output: &mut Output,
    input: &mut Input,
    command: &str,
    secret: &str,
    hopeless: &[&str],
) -> io::Result<Vec<String>>
where
    Output: Read,
    Input: Write,
{
    let mut reader = Reader::default();
    let mut chunk = [0_u8; 4096];
    let mut lines = Vec::new();
    let mut answered = false;
    let mut next = command;
    loop {
        let length = output.read(&mut chunk)?;
        if length == 0 {
            break;
        }
        for event in reader.read(chunk.get(..length).unwrap_or_default()) {
            match event {
                Event::Line(line) => {
                    answered = false;
                    let given_up = hopeless.iter().any(|words| line.contains(words));
                    lines.push(hidden(&line, secret));
                    if given_up {
                        return Ok(lines);
                    }
                }
                Event::Waiting(prompt) if !answered && prompt.trim_end().ends_with("Steam>") => {
                    answered = true;
                    input.write_all(next.as_bytes())?;
                    input.write_all(b"\r")?;
                    input.flush()?;
                    next = "quit";
                }
                Event::Waiting(_) => {}
            }
        }
    }
    Ok(lines)
}

/// Whether the branch `given` on the command line is the one Steam calls `name`.
fn named(given: &str, name: &str) -> bool {
    if webapi::is_default(given) {
        webapi::is_default(name)
    } else {
        given == name
    }
}

/// The names of `branches`, as Steamworks shows them, in one line.
fn listed(branches: &[Branch]) -> String {
    let names: Vec<&str> = branches
        .iter()
        .map(|branch| webapi::shown_name(&branch.name))
        .collect();
    names.join(", ")
}

/// Where a fetch keeps `app`'s `branch` for `platform` in `home` when no folder is given: one
/// folder each, so that fetching it again updates it in place and the others are left alone.
#[must_use]
pub fn folder(home: &Path, app: u32, branch: &str, platform: Platform) -> PathBuf {
    home.join("fetched").join(app.to_string()).join(format!(
        "{}-{}",
        webapi::shown_name(branch),
        system_name(platform)
    ))
}

/// The file in which steamcmd records which build of `app` it put in `folder`.
#[must_use]
pub fn manifest(folder: &Path, app: u32) -> PathBuf {
    folder
        .join("steamapps")
        .join(format!("appmanifest_{app}.acf"))
}

/// Which build a folder holds, as its app manifest records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub build_id: u64,
    /// The branch, as Steamworks shows it: steamcmd records none for the default.
    pub branch: String,
}

/// Reads an app manifest steamcmd wrote.
///
/// # Errors
///
/// When it is not one, or names no build.
pub fn installed(text: &str) -> Result<Installed, String> {
    let document = vdf::parse(text).map_err(|error| error.to_string())?;
    let state = document
        .block("AppState")
        .ok_or("there is no \"AppState\" block")?;
    let build_id = state
        .text("buildid")
        .and_then(|build| build.parse().ok())
        .filter(|build| *build > 0)
        .ok_or("it names no build")?;
    let branch = ["MountedConfig", "UserConfig"]
        .into_iter()
        .find_map(|config| state.block(config)?.text("BetaKey"))
        .filter(|branch| !branch.is_empty())
        .map_or("default", webapi::shown_name);
    Ok(Installed {
        build_id,
        branch: branch.to_owned(),
    })
}

/// What came of a fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// steamcmd said the app is installed, or was already.
    Fetched,
    /// Steam refused the saved login, for this reason.
    NotLoggedIn(String),
    /// Steam refused the branch's password, as steamcmd said.
    PasswordRefused(Vec<String>),
    /// The fetch failed, for every reason given.
    Failed(Vec<String>),
}

/// Reads a fetch of `app` from steamcmd's exit `code`, its `console`, and what it added to its
/// content `log` meanwhile.
#[must_use]
pub fn judge(code: Option<i32>, console: &str, log: &str, app: u32) -> Outcome {
    if let Some(reason) = upload::refused_login(console) {
        return Outcome::NotLoggedIn(reason);
    }
    let finished = [
        format!("Success! App '{app}' fully installed."),
        format!("Success! App '{app}' already up to date."),
    ];
    let said = |line: &str| finished.iter().any(|done| line.trim() == done);
    if code == Some(0_i32) && console.lines().any(said) {
        return Outcome::Fetched;
    }
    let logged = logged(log, app);
    let errors = console.contains("ERROR!");
    if !errors && !logged.is_empty() {
        return Outcome::Failed(logged);
    }
    if code == Some(0_i32) && !errors {
        return Outcome::Failed(vec![format!(
            "steamcmd exited 0 without saying app {app} is installed"
        )]);
    }
    let mut reasons = upload::reasons(code, console, None);
    let checked = format!("Password check for AppId {app} returned error");
    if console.lines().any(|line| line.contains(&checked)) {
        reasons.push("Steam refused the branch's password".to_owned());
        return Outcome::PasswordRefused(reasons);
    }
    reasons.extend(logged);
    if reasons
        .iter()
        .any(|reason| reason.ends_with("(No subscription)"))
    {
        reasons.push(format!(
            "the build account holds no licence for app {app}, so Steam serves it none of its \
             builds"
        ));
    }
    Outcome::Failed(reasons)
}

/// Why steamcmd's content `log` says it gave up on `app`: it says so there alone, as when it
/// cancelled a fetch of a beta branch it remembered whose password it lacked, with an exit code of
/// 8 and nothing on its console.
fn logged(log: &str, app: u32) -> Vec<String> {
    let cancelled = format!("AppID {app} update canceled : ");
    let warned = format!("WARNING! AppID {app} ");
    log.lines()
        .filter_map(|line| {
            if let Some((_, why)) = line.split_once(&cancelled) {
                Some(format!("steamcmd cancelled the download: {}", why.trim()))
            } else {
                line.split_once(&warned)
                    .map(|(_, said)| format!("AppID {app} {}", untokened(said.trim())))
            }
        })
        .collect()
}

/// `said` with the number of each `token` it names hidden, which steamcmd gives beside a branch
/// password it found wrong.
fn untokened(said: &str) -> String {
    let mut shown = String::new();
    let mut rest = said;
    while let Some((before, after)) = rest.split_once("token ") {
        let number = after.trim_start_matches(|character: char| character.is_ascii_digit());
        shown.push_str(before);
        shown.push_str("token ");
        if number.len() < after.len() {
            shown.push_str("<hidden>");
        }
        rest = number;
    }
    shown.push_str(rest);
    shown
}

/// The build `folder` holds, as `fetch --json` prints it.
#[must_use]
pub fn json(app: u32, installed: &Installed, platform: Platform, folder: &Path) -> Value {
    json!({
        "app": app,
        "branch": installed.branch,
        "build_id": installed.build_id,
        "system": system_name(platform),
        "folder": folder.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, build_id: u64, locked: bool) -> Branch {
        Branch {
            name: name.to_owned(),
            build_id,
            description: String::new(),
            locked,
        }
    }

    fn branches() -> Vec<Branch> {
        vec![
            branch("beta", 7, true),
            branch("public", 7, false),
            branch("staging", 0, false),
            branch("testing", 8, false),
        ]
    }

    #[test]
    fn a_system_is_given_by_the_word_steamcmd_takes_for_it() {
        for platform in SYSTEMS {
            assert_eq!(system(system_name(platform)), Ok(platform));
        }
        assert_eq!(
            system("osx"),
            Err("it is windows, linux or macos".to_owned())
        );
    }

    #[test]
    fn the_branch_fetched_is_the_one_named_or_the_default() {
        let all = branches();
        let name = |found: Result<&Branch, String>| found.map(|found| found.name.clone());
        assert_eq!(
            name(chosen(&all, 480, Some("testing"), None)),
            Ok("testing".to_owned())
        );
        assert_eq!(name(chosen(&all, 480, None, None)), Ok("public".to_owned()));
        assert_eq!(
            name(chosen(&all, 480, Some("default"), Some(7))),
            Ok("public".to_owned())
        );
        assert_eq!(
            chosen(&all, 480, Some("nightly"), None),
            Err(
                "app 480 has no branch nightly; its branches are beta, default, staging, testing"
                    .to_owned()
            )
        );
        assert_eq!(
            chosen(&[branch("beta", 7, false)], 480, None, None),
            Err("app 480 has no default branch".to_owned())
        );
    }

    #[test]
    fn a_build_is_fetched_from_a_branch_it_is_live_on_the_default_first() {
        let all = branches();
        let name = |found: Result<&Branch, String>| found.map(|found| found.name.clone());
        assert_eq!(
            name(chosen(&all, 480, None, Some(7))),
            Ok("public".to_owned())
        );
        assert_eq!(
            name(chosen(&all, 480, None, Some(8))),
            Ok("testing".to_owned())
        );
        let open_first = [branch("beta", 7, true), branch("open", 7, false)];
        assert_eq!(
            name(chosen(&open_first, 480, None, Some(7))),
            Ok("open".to_owned())
        );
        assert_eq!(
            chosen(&all, 480, None, Some(9)),
            Err(
                "build 9 is live on no branch of app 480, and Steam serves only a branch's live \
                 build: set it live with `steamship promote --build 9 --branch <branch>`, then \
                 fetch that branch"
                    .to_owned()
            )
        );
        assert_eq!(
            chosen(&all, 480, Some("testing"), Some(7)),
            Err("build 7 is not live on app 480's testing branch, which has build 8".to_owned())
        );
    }

    #[test]
    fn a_branch_with_no_build_is_refused_and_one_with_a_password_chosen() {
        let all = branches();
        assert_eq!(
            chosen(&all, 480, Some("staging"), None),
            Err("no build is live on app 480's staging branch".to_owned())
        );
        assert!(chosen(&all, 480, Some("beta"), None).is_ok_and(|found| found.locked));
    }

    #[test]
    fn the_command_typed_names_the_branch_and_its_password() {
        assert_eq!(
            typed_command(480, "friends", "open sesame").as_str(),
            "app_update 480 -beta friends -betapassword open sesame validate"
        );
        assert!(typeable("Open-Sesame!42"));
        for split in ["", "open sesame", "open\tsesame", "open\"sesame"] {
            assert!(!typeable(split), "{split}");
        }
    }

    #[test]
    fn a_password_is_hidden_wherever_it_shows_and_after_its_option() {
        assert_eq!(
            hidden(
                "Steam>app_update 480 -beta friends -betapassword open sesame validate\nopen \
                 sesame\nfine",
                "open sesame"
            ),
            "Steam>app_update 480 -beta friends -betapassword <password>\n<password>\nfine"
        );
        assert_eq!(
            hidden("-betapassword x\nfine", ""),
            "-betapassword <password>\nfine"
        );
        assert_eq!(hidden("fine\n", ""), "fine\n");
    }

    /// Hands out one chunk on each read, as a terminal does when the program pauses.
    struct Chunks(Vec<&'static [u8]>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            let chunk = self.0.remove(0);
            buf.get_mut(..chunk.len())
                .ok_or_else(|| io::Error::other("chunk too large"))?
                .copy_from_slice(chunk);
            Ok(chunk.len())
        }
    }

    #[test]
    fn the_fetch_is_typed_at_the_first_prompt_and_quit_at_the_next_with_the_password_hidden() {
        let mut output = Chunks(vec![
            b"Waiting for user info...OK\r\n",
            b"\r\nSteam>",
            b"app_update 480 -beta",
            b" friends -betapassword open sesame validate\r\n",
            b"Success! App '480' fully installed.\r\n\r\nSteam>",
            b"quit\r\n",
        ]);
        let mut input = Vec::new();
        let command = typed_command(480, "friends", "open sesame");
        let lines = typed(&mut output, &mut input, &command, "open sesame", &[]).unwrap();
        assert_eq!(
            String::from_utf8(input).unwrap(),
            "app_update 480 -beta friends -betapassword open sesame validate\rquit\r"
        );
        assert_eq!(
            lines,
            [
                "Waiting for user info...OK",
                "",
                "Steam>app_update 480 -beta friends -betapassword <password>",
                "Success! App '480' fully installed.",
                "",
                "Steam>quit"
            ]
        );
    }

    /// steamcmd 1792617903's console refusing a branch's password typed at it.
    const WRONG_PASSWORD: &str = "ERROR! Password check for AppId 480 returned error Failure.\n";

    #[test]
    fn a_refused_password_is_told_apart_from_a_failed_fetch() {
        for code in [Some(0_i32), Some(8_i32)] {
            assert_eq!(
                judge(code, WRONG_PASSWORD, "", 480),
                Outcome::PasswordRefused(vec![
                    "Password check for AppId 480 returned error Failure.".to_owned(),
                    "Steam refused the branch's password".to_owned(),
                ]),
                "{code:?}"
            );
        }
        assert!(matches!(
            judge(Some(8_i32), WRONG_PASSWORD, "", 481),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn each_app_branch_and_system_has_its_own_folder() {
        let home = Path::new("home");
        assert_eq!(
            folder(home, 480, "public", Platform::Linux),
            home.join("fetched").join("480").join("default-linux")
        );
        assert_eq!(
            folder(home, 480, "testing", Platform::Windows),
            home.join("fetched").join("480").join("testing-windows")
        );
        assert_eq!(
            manifest(home, 480),
            home.join("steamapps").join("appmanifest_480.acf")
        );
    }

    /// An app manifest laid out as steamcmd 1792617903 wrote one for a beta branch, its values
    /// those of a sample app.
    const ON_TESTING: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"480\"\n\t\"StateFlags\"\t\t\"4\"\n\
        \t\"installdir\"\t\t\"Spacewar\"\n\t\"buildid\"\t\t\"1234567\"\n\
        \t\"TargetBuildID\"\t\t\"1234567\"\n\t\"InstalledDepots\"\n\t{\n\t\t\"481\"\n\t\t{\n\
        \t\t\t\"manifest\"\t\t\"42\"\n\t\t\t\"size\"\t\t\"1024\"\n\t\t}\n\t}\n\
        \t\"UserConfig\"\n\t{\n\t\t\"BetaKey\"\t\t\"testing\"\n\t}\n\
        \t\"MountedConfig\"\n\t{\n\t\t\"BetaKey\"\t\t\"testing\"\n\t}\n}\n";

    #[test]
    fn a_folder_holds_the_build_and_branch_its_app_manifest_records() {
        assert_eq!(
            installed(ON_TESTING),
            Ok(Installed {
                build_id: 1_234_567,
                branch: "testing".to_owned()
            })
        );
        let on_default = "\"AppState\"\n{\n\"buildid\" \"1234567\"\n}\n";
        assert_eq!(
            installed(on_default),
            Ok(Installed {
                build_id: 1_234_567,
                branch: "default".to_owned()
            })
        );
        let public =
            "\"AppState\"\n{\n\"buildid\" \"5\"\n\"UserConfig\"\n{\n\"BetaKey\" \"public\"\n}\n}\n";
        assert_eq!(
            installed(public).map(|found| found.branch),
            Ok("default".to_owned())
        );
        for (text, why) in [
            ("\"Other\"\n{\n}\n", "there is no \"AppState\" block"),
            (
                "\"AppState\"\n{\n\"buildid\" \"0\"\n}\n",
                "it names no build",
            ),
            (
                "\"AppState\"\n{\n\"buildid\" \"x\"\n}\n",
                "it names no build",
            ),
        ] {
            assert_eq!(installed(text), Err(why.to_owned()), "{text}");
        }
        drop(installed("\"AppState\"\n{").unwrap_err());
    }

    /// steamcmd 1792617903's console fetching a build, then fetching it again.
    const FETCHED: &str = "Logging in user 'build_bot' [U:1:1] to Steam Public...OK\n\
        Waiting for user info...OK\n\
        \u{20}Update state (0x61) downloading, progress: 62.17 (979400546 / 1575439958)\n\
        \u{20}Update state (0x81) verifying update, progress: 88.50 (1394319538 / 1575439958)\n\
        Success! App '480' fully installed.\n\
        Unloading Steam API...OK\n";
    const UP_TO_DATE: &str = "Waiting for user info...OK\n\
        Success! App '480' already up to date.\n\
        Unloading Steam API...OK\n";

    #[test]
    fn a_fetch_is_done_only_when_steamcmd_says_the_app_is_installed_and_exits_0() {
        assert_eq!(judge(Some(0_i32), FETCHED, "", 480), Outcome::Fetched);
        assert_eq!(judge(Some(0_i32), UP_TO_DATE, "", 480), Outcome::Fetched);
        assert_eq!(
            judge(Some(0_i32), FETCHED, "", 481),
            Outcome::Failed(vec![
                "steamcmd exited 0 without saying app 481 is installed".to_owned()
            ])
        );
        assert!(matches!(
            judge(Some(8_i32), FETCHED, "", 480),
            Outcome::Failed(_)
        ));
        assert!(matches!(judge(None, FETCHED, "", 480), Outcome::Failed(_)));
    }

    #[test]
    fn a_failed_fetch_says_steamcmds_reason_and_what_no_licence_means() {
        assert_eq!(
            judge(
                Some(8_i32),
                "ERROR! Failed to install app '480' (No subscription)\n",
                "",
                480
            ),
            Outcome::Failed(vec![
                "Failed to install app '480' (No subscription)".to_owned(),
                "the build account holds no licence for app 480, so Steam serves it none of \
                 its builds"
                    .to_owned(),
            ])
        );
        for code in [Some(8_i32), Some(0_i32)] {
            assert_eq!(
                judge(code, "ERROR! Failed to set beta 'testing'\n", "", 480),
                Outcome::Failed(vec!["Failed to set beta 'testing'".to_owned()]),
                "{code:?}"
            );
        }
        assert_eq!(
            judge(Some(8_i32), "Unloading Steam API...OK\n", "", 480),
            Outcome::Failed(vec!["steamcmd exited 8 without saying why".to_owned()])
        );
    }

    /// What steamcmd 1792617903 added to its content log, and nothing to its console, cancelling a
    /// fetch of the default branch while it remembered a beta branch whose password it lacked;
    /// another app's lines among them.
    const CANCELLED: &str = "[2026-10-28 16:33:50] AppID 480 App update changed : Running Update,Reconfiguring,\n\
        [2026-10-28 16:33:50] WARNING! AppID 480 has missing cached password for private beta \
        \"testing\" set, falling back to public\n\
        [2026-10-28 16:33:50] AppID 480 beta changed: public\n\
        [2026-10-28 16:33:50] AppID 480 update canceled : Missing app info for 480 (Missing \
        configuration)\n\
        [2026-10-28 16:33:50] AppID 481 update canceled : Missing app info for 481 (Missing \
        configuration)\n";

    #[test]
    fn a_fetch_steamcmd_gave_up_on_in_its_log_alone_says_why_from_there() {
        let gave_up = Outcome::Failed(vec![
            "AppID 480 has missing cached password for private beta \"testing\" set, falling back \
             to public"
                .to_owned(),
            "steamcmd cancelled the download: Missing app info for 480 (Missing configuration)"
                .to_owned(),
        ]);
        assert_eq!(
            judge(Some(8_i32), "Unloading Steam API...OK\n", CANCELLED, 480),
            gave_up
        );
        assert_eq!(
            judge(Some(0_i32), "Unloading Steam API...OK\n", CANCELLED, 480),
            gave_up
        );
        assert!(matches!(
            judge(
                Some(8_i32),
                "ERROR! Failed to set beta 'testing'\n",
                CANCELLED,
                480,
            ),
            Outcome::Failed(reasons) if reasons.len() == 3
        ));
    }

    #[test]
    fn the_token_steamcmd_logs_beside_a_wrong_password_is_hidden() {
        let log = "[2026-10-28 17:05:27] WARNING! AppID 480 has missing info (Invalid Password, \
                   token 1234567890123456789) for private beta \"testing\" set, falling back to \
                   public\n";
        assert_eq!(
            judge(Some(8_i32), "", log, 480),
            Outcome::Failed(vec![
                "AppID 480 has missing info (Invalid Password, token <hidden>) for private beta \
                 \"testing\" set, falling back to public"
                    .to_owned()
            ])
        );
        assert_eq!(
            judge(Some(0_i32), WRONG_PASSWORD, log, 480),
            Outcome::PasswordRefused(vec![
                "Password check for AppId 480 returned error Failure.".to_owned(),
                "Steam refused the branch's password".to_owned(),
            ])
        );
        assert_eq!(
            untokened("a token, token 12 and token 3x"),
            "a token, token <hidden> and token <hidden>x"
        );
    }

    #[test]
    fn a_refused_login_is_not_a_failed_fetch() {
        assert!(matches!(
            judge(
                Some(5_i32),
                "Logging in user 'build_bot' to Steam Public...FAILED (Invalid Password)\n",
                CANCELLED,
                480
            ),
            Outcome::NotLoggedIn(_)
        ));
    }

    #[test]
    fn json_gives_the_build_its_branch_system_and_folder() {
        let found = Installed {
            build_id: 7,
            branch: "testing".to_owned(),
        };
        assert_eq!(
            json(480, &found, Platform::Linux, Path::new("fetched")),
            json!({
                "app": 480_u32,
                "branch": "testing",
                "build_id": 7_u64,
                "system": "linux",
                "folder": "fetched",
            })
        );
    }
}
