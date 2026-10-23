//! The settings made in Steamworks that a build depends on, as steamcmd's `app_info_print` shows
//! them to the build account, and a snapshot of them kept beside the build scripts.
//!
//! Steam shows its own bookkeeping beside them (each depot's manifests, the build on each
//! branch), and for a released app store details that change on Valve's side. Only what is set in
//! Steamworks for the build is kept, so that the snapshot differs from Steam when someone changed
//! the app there, and not after every upload.

use std::fmt;

use crate::vdf::{self, Block, Pair, Value};

/// The snapshot's name, beside the build scripts.
pub const SNAPSHOT: &str = "settings.vdf";

/// The keys of `common` kept: what the app is, the app a demo or DLC belongs to, and the systems
/// it runs on.
const COMMON: [&str; 6] = ["name", "type", "parent", "oslist", "osarch", "osextended"];

/// A demo's type, as `app_info_print` shows it.
const DEMO: &str = "Demo";

/// The types of app a player launches, whose every system needs a launch option.
const LAUNCHED: [&str; 2] = ["Game", DEMO];

/// The sections kept whole: how the app installs and launches, and its Steam Cloud.
const WHOLE: [&str; 2] = ["config", "ufs"];

/// Steam's bookkeeping under `depots` and under each depot, which uploads change.
const BOOKKEEPING: [&str; 4] = [
    "branches",
    "privatebranches",
    "manifests",
    "encryptedmanifests",
];

/// `app`'s settings in steamcmd's `console`, where `app_info_print` printed them.
///
/// # Errors
///
/// As [`info_from_console`].
pub fn from_console(console: &str, app: u32) -> Result<Block, String> {
    info_from_console(console, app).map(|shown| kept(&shown))
}

/// All of `app`'s info in steamcmd's `console`, where `app_info_print` printed it, Steam's own
/// bookkeeping and release state among it.
///
/// # Errors
///
/// When steamcmd printed none, what it printed cannot be read, or Steam showed the build account
/// nothing, as for an app it cannot see.
pub fn info_from_console(console: &str, app: u32) -> Result<Block, String> {
    let named = format!("\"{app}\"");
    let mut printed = String::new();
    // Its closing brace is the only one at the start of a line: those within are indented.
    for line in console.lines().skip_while(|line| line.trim_end() != named) {
        printed.push_str(line);
        printed.push('\n');
        if line.trim_end() == "}" {
            break;
        }
    }
    if printed.is_empty() {
        return Err(format!("steamcmd printed no settings for app {app}"));
    }
    let document = vdf::parse(&printed).map_err(|error| {
        format!("steamcmd printed settings for app {app} that cannot be read: {error}")
    })?;
    let shown = document.block(&app.to_string()).unwrap_or(&document);
    if shown.pairs.is_empty() {
        return Err(format!(
            "Steam shows the build account no settings for app {app}: it cannot see the app, or \
             there is no such app"
        ));
    }
    Ok(shown.clone())
}

/// What of `shown` is kept: see the module's description.
fn kept(shown: &Block) -> Block {
    let pairs = shown
        .pairs
        .iter()
        .filter_map(|pair| {
            let value = match (&pair.value, pair.key.to_ascii_lowercase().as_str()) {
                (Value::Block(common), "common") => Value::Block(only(common, &COMMON)),
                (Value::Block(depots), "depots") => Value::Block(Block {
                    pairs: without_bookkeeping(depots)
                        .into_iter()
                        .map(|depot| match depot.value {
                            Value::Block(inner) => Pair {
                                key: depot.key,
                                value: Value::Block(Block {
                                    pairs: without_bookkeeping(&inner),
                                }),
                            },
                            Value::Text(_) => depot,
                        })
                        .collect(),
                }),
                (value, key) if WHOLE.contains(&key) => value.clone(),
                _ => return None,
            };
            Some(Pair {
                key: pair.key.clone(),
                value,
            })
        })
        .collect();
    Block { pairs }
}

/// The pairs of `block` whose keys are among `keys`, in order.
fn only(block: &Block, keys: &[&str]) -> Block {
    Block {
        pairs: block
            .pairs
            .iter()
            .filter(|pair| keys.iter().any(|key| pair.key.eq_ignore_ascii_case(key)))
            .cloned()
            .collect(),
    }
}

fn without_bookkeeping(block: &Block) -> Vec<Pair> {
    block
        .pairs
        .iter()
        .filter(|pair| {
            !BOOKKEEPING
                .iter()
                .any(|key| pair.key.eq_ignore_ascii_case(key))
        })
        .cloned()
        .collect()
}

/// Whether `settings` hold Steam Cloud, and for whom, said plainly.
///
/// An app with none set up in Steamworks has no `ufs` section at all, which a snapshot alone
/// cannot tell from one that left it out. Cloud kept to developers passes every comparison while
/// players have none, and Steamworks has ticked that for an app by itself the first time its
/// Cloud was saved.
#[must_use]
pub fn cloud(settings: &Block) -> &'static str {
    match settings.block("ufs") {
        Some(ufs) if ufs.text("hidecloudui") == Some("1") => {
            "Steam Cloud set up in Steamworks for developers only, so players have none: turn \
             it on for everyone on the app's Steam Cloud page"
        }
        Some(ufs) if !ufs.pairs.is_empty() => "Steam Cloud set up in Steamworks",
        _ => "Steam Cloud not set up in Steamworks",
    }
}

/// What `settings` show of the defaults on the app's Steam Input page. An app has none until they
/// are chosen there, and a Steam Deck has given a demo with none no input from its controls.
#[must_use]
pub fn controls(settings: &Block) -> String {
    let template = |key: &str| settings.block("config")?.text(key);
    let [(controller, _), (touch, _)] = CONTROLS;
    let touch = template(touch).map_or_else(
        || "no touch default".to_owned(),
        |touch| format!("touch template {touch}"),
    );
    template(controller).map_or_else(
        || {
            format!(
                "no Default Controller Configuration in Steamworks, and {touch}, so a Steam Deck \
                 may give the game no input from its controls: choose one on the app's Steam \
                 Input page"
            )
        },
        |controller| {
            format!(
                "Steam Input defaults set in Steamworks: controller template {controller}, {touch}"
            )
        },
    )
}

/// `settings` for `app` as a snapshot file: Valve's own format, as `app_info_print` shows it.
///
/// # Errors
///
/// A text that the format cannot hold, as [`vdf::write`] says.
pub fn snapshot(app: u32, settings: &Block) -> Result<String, String> {
    let document = Block {
        pairs: vec![Pair {
            key: app.to_string(),
            value: Value::Block(settings.clone()),
        }],
    };
    let written = vdf::write(&document).map_err(|error| error.to_string())?;
    Ok(format!(
        "// App {app}'s settings in Steamworks, kept by `steamship settings --save`;\n\
         // `steamship settings --check` says where Steam's differ.\n{written}"
    ))
}

/// Why the install folder `settings` names is refused, if it is: a folder of anything but ASCII
/// letters and digits is one a path, a shell or an installer on some system takes otherwise.
#[must_use]
pub fn install_folder(settings: &Block) -> Option<String> {
    let folder = settings.block("config")?.text("installdir")?;
    let plain = !folder.is_empty() && folder.chars().all(|each| each.is_ascii_alphanumeric());
    (!plain).then(|| {
        format!(
            "the install folder is \"{folder}\", and steamship takes letters and digits alone: \
             change it in Steamworks under Installation, General Installation, then save the \
             settings again"
        )
    })
}

/// Why `settings`, app `app`'s, are refused as the demo of the full game `full`: a demo is an app
/// typed Demo whose base game is the full game.
#[must_use]
pub fn demo_of(settings: &Block, app: u32, full: u32) -> Vec<String> {
    let common = settings.block("common");
    let then = "under General Application Settings in Steamworks, then save the settings again";
    let mut wrong = Vec::new();
    match common.and_then(|common| common.text("type")) {
        Some(kind) if kind.eq_ignore_ascii_case(DEMO) => {}
        Some(kind) => wrong.push(format!(
            "app {app} is typed {kind}, not {DEMO}: set its type to {DEMO} {then}"
        )),
        None => wrong.push(format!(
            "app {app}'s settings give it no type: set it to {DEMO} {then}"
        )),
    }
    match common.and_then(|common| common.text("parent")) {
        Some(parent) if parent == full.to_string() => {}
        Some(parent) => wrong.push(format!(
            "app {app}'s base game is app {parent}, not the full game, app {full}: change it {then}"
        )),
        None => wrong.push(format!(
            "app {app}'s settings name no base game: enter the full game, app {full}, {then}"
        )),
    }
    wrong
}

/// The defaults the Steam Input page sets, by their key in `config` and their name on the page.
const CONTROLS: [(&str, &str); 2] = [
    (
        "steamcontrollertemplateindex",
        "Default Controller Configuration",
    ),
    (
        "steamcontrollertouchtemplateindex",
        "Default Touch Configuration",
    ),
];

/// Each Steam Input default the full game's settings `game` have that `settings` lack or set
/// otherwise, which refuses them as the demo's.
///
/// `settings` are app `app`'s, and `game` app `full`'s. Without its defaults a Steam Deck gives a
/// demo no input from its controls at all.
#[must_use]
pub fn controls_unlike(settings: &Block, app: u32, game: &Block, full: u32) -> Vec<String> {
    let config = |of: &Block, key: &str| {
        of.block("config")
            .and_then(|config| config.text(key))
            .map(ToOwned::to_owned)
    };
    let then = format!(
        "choose the full game's on app {app}'s Steam Input page in Steamworks, then save the \
         settings again"
    );
    CONTROLS
        .iter()
        .filter_map(|(key, name)| {
            let wanted = config(game, key)?;
            match config(settings, key) {
                Some(set) if set == wanted => None,
                Some(set) => Some(format!(
                    "app {app}'s {name} is template {set} and the full game's, app {full}'s, is \
                     template {wanted}: {then}"
                )),
                None => Some(format!(
                    "app {app} has no {name}, and the full game, app {full}, has template \
                     {wanted}: {then}"
                )),
            }
        })
        .collect()
}

/// Each system an app the player launches has a depot of its own for, and no launch option runs
/// on, by its name in `oslist`; none for an app of another type, such as a tool.
#[must_use]
pub fn unlaunched(settings: &Block) -> Vec<String> {
    let played = settings
        .block("common")
        .and_then(|common| common.text("type"))
        .is_some_and(|kind| LAUNCHED.iter().any(|each| kind.eq_ignore_ascii_case(each)));
    if !played {
        return Vec::new();
    }
    let mut systems: Vec<String> = Vec::new();
    for (_, depot) in own_depots(settings) {
        for system in systems_of(depot.block("config")) {
            if !systems.contains(&system) {
                systems.push(system);
            }
        }
    }
    let options: Vec<&Block> = settings
        .block("config")
        .and_then(|config| config.block("launch"))
        .map(|launch| {
            launch
                .pairs
                .iter()
                .filter_map(|pair| match &pair.value {
                    Value::Block(option) => Some(option),
                    Value::Text(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    // A launch option that names no system runs on every one.
    if options
        .iter()
        .any(|option| systems_of(option.block("config")).is_empty())
    {
        return Vec::new();
    }
    systems.retain(|system| {
        !options
            .iter()
            .any(|option| systems_of(option.block("config")).contains(system))
    });
    systems
}

/// The depots of `depots`, an app build script's, that `settings` do not show as the app's own;
/// none when they show no depot of its own at all, as before any is published.
#[must_use]
pub fn foreign(settings: &Block, depots: &[u32]) -> Vec<u32> {
    let own: Vec<u32> = own_depots(settings).into_iter().map(|(id, _)| id).collect();
    if own.is_empty() {
        return Vec::new();
    }
    depots
        .iter()
        .copied()
        .filter(|depot| !own.contains(depot))
        .collect()
}

/// The app's own depots by their IDs, without those another app's, such as a Common
/// Redistributable, mounts.
fn own_depots(settings: &Block) -> Vec<(u32, &Block)> {
    settings
        .block("depots")
        .map(|listed| {
            listed
                .pairs
                .iter()
                .filter_map(|pair| match &pair.value {
                    Value::Block(depot) if depot.text("depotfromapp").is_none() => {
                        Some((pair.key.parse().ok()?, depot))
                    }
                    Value::Block(_) | Value::Text(_) => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The systems a `config` block's `oslist` names, such as `windows` and `linux`.
fn systems_of(config: Option<&Block>) -> Vec<String> {
    config
        .and_then(|config| config.text("oslist"))
        .map(|list| {
            list.split(',')
                .map(|system| system.trim().to_ascii_lowercase())
                .filter(|system| !system.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Reads a snapshot's `text`, which must be `app`'s.
///
/// # Errors
///
/// When the text is not one block named by an app ID, or names another app.
pub fn read(text: &str, app: u32) -> Result<Block, String> {
    let document = vdf::parse(text).map_err(|error| error.to_string())?;
    let [
        Pair {
            key,
            value: Value::Block(settings),
        },
    ] = document.pairs.as_slice()
    else {
        return Err("is not one block named by the app's ID".to_owned());
    };
    let named: u32 = key
        .parse()
        .ok()
        .ok_or_else(|| format!("\"{key}\" is not an app's ID"))?;
    if named != app {
        return Err(format!("is for app {named}, not app {app}"));
    }
    Ok(settings.clone())
}

/// A way Steam's settings and the file's differ, each by the path of keys to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// In the file, not on Steam.
    NotOnSteam(String),
    /// On Steam, not in the file.
    NotInFile(String),
    /// One thing on Steam and another in the file.
    Differs {
        path: String,
        steam: String,
        file: String,
    },
}

impl fmt::Display for Drift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOnSteam(path) => write!(formatter, "{path}: in the file, not on Steam"),
            Self::NotInFile(path) => write!(formatter, "{path}: on Steam, not in the file"),
            Self::Differs { path, steam, file } => {
                write!(
                    formatter,
                    "{path} is {steam} on Steam and {file} in the file"
                )
            }
        }
    }
}

/// Every way `steam` differs from the `file`: each of the file's keys in its order, then those
/// only Steam has, in Steam's. Keys match regardless of case, as Valve's reader matches them.
#[must_use]
pub fn compare(file: &Block, steam: &Block) -> Vec<Drift> {
    let mut drift = Vec::new();
    walk("", file, steam, &mut drift);
    drift
}

fn walk(path: &str, file: &Block, steam: &Block, drift: &mut Vec<Drift>) {
    let at = |key: &str| {
        if path.is_empty() {
            key.to_owned()
        } else {
            format!("{path}/{key}")
        }
    };
    for pair in &file.pairs {
        match (&pair.value, steam.get(&pair.key)) {
            (_, None) => drift.push(Drift::NotOnSteam(at(&pair.key))),
            (Value::Block(inner), Some(Value::Block(held))) => {
                walk(&at(&pair.key), inner, held, drift);
            }
            (Value::Text(wanted), Some(Value::Text(held))) if wanted == held => {}
            (wanted, Some(held)) => drift.push(Drift::Differs {
                path: at(&pair.key),
                steam: shown(held),
                file: shown(wanted),
            }),
        }
    }
    drift.extend(
        steam
            .pairs
            .iter()
            .filter(|pair| file.get(&pair.key).is_none())
            .map(|pair| Drift::NotInFile(at(&pair.key))),
    );
}

fn shown(value: &Value) -> String {
    match value {
        Value::Text(text) => format!("\"{text}\""),
        Value::Block(_) => "a block".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// steamcmd's console as it printed Falcon Ridge Overseer's app info, trimmed.
    const CONSOLE: &str = "Logging in user 'ACCOUNT' [U:1:1] to Steam Public...OK\n\
        AppID : 1000010, change number : 39317222/39317222, last change : Tue Sep 29 2026 \n\
        \"1000010\"\n{\n\
        \t\"common\"\n\t{\n\
        \t\t\"name\"\t\t\"Falcon Ridge Overseer\"\n\
        \t\t\"section_type\"\t\t\"ownersonly\"\n\
        \t\t\"type\"\t\t\"Game\"\n\
        \t\t\"ReleaseState\"\t\t\"unavailable\"\n\
        \t\t\"oslist\"\t\t\"windows,linux\"\n\
        \t\t\"osarch\"\t\t\"64\"\n\
        \t\t\"store_tags\"\n\t\t{\n\t\t}\n\
        \t}\n\
        \t\"config\"\n\t{\n\
        \t\t\"installdir\"\t\t\"FGM\"\n\
        \t\t\"launch\"\n\t\t{\n\
        \t\t\t\"0\"\n\t\t\t{\n\
        \t\t\t\t\"executable\"\t\t\"falcon-ridge-overseer.exe\"\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\
        \t\"depots\"\n\t{\n\
        \t\t\"1000011\"\n\t\t{\n\
        \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t}\n\
        \t\t\t\"manifests\"\n\t\t\t{\n\t\t\t\t\"public\"\n\t\t\t\t{\n\
        \t\t\t\t\t\"gid\"\t\t\"4987896756238774439\"\n\t\t\t\t}\n\t\t\t}\n\
        \t\t}\n\
        \t\t\"workshopdepot\"\t\t\"1000010\"\n\
        \t\t\"branches\"\n\t\t{\n\t\t\t\"public\"\n\t\t\t{\n\
        \t\t\t\t\"buildid\"\t\t\"25585928\"\n\t\t\t}\n\t\t}\n\
        \t\t\"privatebranches\"\t\t\"1\"\n\
        \t}\n\
        \t\"ufs\"\n\t{\n\t\t\"quota\"\t\t\"1073741824\"\n\t}\n\
        \t\"extended\"\n\t{\n\t\t\"developer\"\t\t\"Someone\"\n\t}\n\
        }\n\
        Unloading Steam API...OK\n";

    /// A demo's settings, each section shaped as steamcmd printed a public demo's and a released
    /// game's: typed Demo with its base game as `parent`, a launch option for Windows, and depots
    /// for Windows and Linux beside a Common Redistributable's.
    const DEMO: &str = "\"1000070\"\n{\n\
        \t\"common\"\n\t{\n\
        \t\t\"name\"\t\t\"Sample Demo\"\n\
        \t\t\"type\"\t\t\"Demo\"\n\
        \t\t\"parent\"\t\t\"1000050\"\n\
        \t\t\"oslist\"\t\t\"windows\"\n\
        \t\t\"ReleaseState\"\t\t\"prerelease\"\n\
        \t}\n\
        \t\"config\"\n\t{\n\
        \t\t\"installdir\"\t\t\"SampleDemo\"\n\
        \t\t\"launch\"\n\t\t{\n\
        \t\t\t\"0\"\n\t\t\t{\n\
        \t\t\t\t\"executable\"\t\t\"sample.exe\"\n\
        \t\t\t\t\"type\"\t\t\"default\"\n\
        \t\t\t\t\"config\"\n\t\t\t\t{\n\t\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t\t}\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\
        \t\"depots\"\n\t{\n\
        \t\t\"baselanguages\"\t\t\"\"\n\
        \t\t\"228988\"\n\t\t{\n\
        \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t}\n\
        \t\t\t\"depotfromapp\"\t\t\"228980\"\n\
        \t\t\t\"sharedinstall\"\t\t\"1\"\n\
        \t\t}\n\
        \t\t\"1000071\"\n\t\t{\n\
        \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t\t\"language\"\t\t\"\"\n\t\t\t}\n\
        \t\t}\n\
        \t\t\"1000072\"\n\t\t{\n\
        \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"linux\"\n\t\t\t\t\"language\"\t\t\"\"\n\t\t\t}\n\
        \t\t}\n\
        \t}\n\
        }\n";

    fn demo(text: &str) -> Block {
        from_console(text, 1_000_070).unwrap()
    }

    /// `DEMO` with the Steam Input defaults `controls` in its `config`, written as a snapshot
    /// holds them.
    fn controlled(controls: &[(&str, &str)]) -> Block {
        let installdir = "\t\t\"installdir\"\t\t\"SampleDemo\"\n";
        let mut lines = installdir.to_owned();
        for (key, value) in controls {
            lines.push_str(&["\t\t\"", key, "\"\t\t\"", value, "\"\n"].concat());
        }
        demo(&DEMO.replace(installdir, &lines))
    }

    #[test]
    fn the_steam_input_defaults_are_said_and_a_missing_controller_one_is_named() {
        assert_eq!(
            controls(&controlled(&[
                ("steamcontrollertouchtemplateindex", "8"),
                ("steamcontrollertemplateindex", "4"),
            ])),
            "Steam Input defaults set in Steamworks: controller template 4, touch template 8"
        );
        assert_eq!(
            controls(&controlled(&[("steamcontrollertemplateindex", "4")])),
            "Steam Input defaults set in Steamworks: controller template 4, no touch default"
        );
        assert_eq!(
            controls(&controlled(&[("steamcontrollertouchtemplateindex", "8")])),
            "no Default Controller Configuration in Steamworks, and touch template 8, so a Steam \
             Deck may give the game no input from its controls: choose one on the app's Steam \
             Input page"
        );
    }

    #[test]
    fn a_demo_without_the_full_games_steam_input_defaults_is_told_to_choose_them() {
        let both = [
            ("steamcontrollertouchtemplateindex", "8"),
            ("steamcontrollertemplateindex", "4"),
        ];
        let game = controlled(&both);
        let then = "choose the full game's on app 1000070's Steam Input page in Steamworks, then \
                    save the settings again";
        assert_eq!(
            controls_unlike(&demo(DEMO), 1_000_070, &game, 1_000_050),
            [
                format!(
                    "app 1000070 has no Default Controller Configuration, and the full game, app \
                     1000050, has template 4: {then}"
                ),
                format!(
                    "app 1000070 has no Default Touch Configuration, and the full game, app \
                     1000050, has template 8: {then}"
                ),
            ]
        );
        assert_eq!(
            controls_unlike(
                &controlled(&[
                    ("steamcontrollertouchtemplateindex", "8"),
                    ("steamcontrollertemplateindex", "1"),
                ]),
                1_000_070,
                &game,
                1_000_050
            ),
            [format!(
                "app 1000070's Default Controller Configuration is template 1 and the full \
                 game's, app 1000050's, is template 4: {then}"
            )]
        );
        assert_eq!(
            controls_unlike(&controlled(&both), 1_000_070, &game, 1_000_050),
            Vec::<String>::new()
        );
        assert_eq!(
            controls_unlike(&controlled(&both), 1_000_070, &demo(DEMO), 1_000_050),
            Vec::<String>::new(),
            "a game with no default asks none of its demo"
        );
    }

    #[test]
    fn a_demo_typed_demo_whose_base_game_is_the_full_game_passes_and_any_other_is_named() {
        let settings = demo(DEMO);
        assert_eq!(
            settings
                .block("common")
                .and_then(|common| common.text("parent")),
            Some("1000050"),
            "the base game is kept"
        );
        assert_eq!(
            demo_of(&settings, 1_000_070, 1_000_050),
            Vec::<String>::new()
        );
        let then = "under General Application Settings in Steamworks, then save the settings again";
        assert_eq!(
            demo_of(&settings, 1_000_070, 1_000_030),
            [format!(
                "app 1000070's base game is app 1000050, not the full game, app 1000030: change \
                 it {then}"
            )]
        );
        assert_eq!(
            demo_of(
                &demo(
                    &DEMO
                        .replace("\"Demo\"", "\"Game\"")
                        .replace("\t\t\"parent\"\t\t\"1000050\"\n", "")
                ),
                1_000_070,
                1_000_050
            ),
            [
                format!("app 1000070 is typed Game, not Demo: set its type to Demo {then}"),
                format!(
                    "app 1000070's settings name no base game: enter the full game, app 1000050, \
                     {then}"
                ),
            ]
        );
        assert_eq!(
            demo_of(&Block { pairs: Vec::new() }, 1_000_070, 1_000_050).first(),
            Some(&format!(
                "app 1000070's settings give it no type: set it to Demo {then}"
            ))
        );
    }

    #[test]
    fn a_system_an_app_s_own_depots_install_on_with_no_launch_option_is_named() {
        assert_eq!(unlaunched(&demo(DEMO)), ["linux"]);
        let both = DEMO.replace(
            "\"oslist\"\t\t\"windows\"\n\t\t\t\t}",
            "\"oslist\"\t\t\"windows,linux\"\n\t\t\t\t}",
        );
        assert!(unlaunched(&demo(&both)).is_empty(), "{both}");
        let anywhere = DEMO.replace(
            "\t\t\t\t\"config\"\n\t\t\t\t{\n\t\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t\t}\n",
            "",
        );
        assert!(unlaunched(&demo(&anywhere)).is_empty(), "{anywhere}");
        let tool = DEMO.replace("\"Demo\"", "\"Tool\"");
        assert!(unlaunched(&demo(&tool)).is_empty(), "{tool}");
        let unlaunchable = DEMO.replace("\t\t\"launch\"", "\t\t\"launched\"");
        assert_eq!(unlaunched(&demo(&unlaunchable)), ["windows", "linux"]);
    }

    #[test]
    fn a_depot_the_app_does_not_own_is_named_once_it_owns_any() {
        let settings = demo(DEMO);
        assert_eq!(
            foreign(&settings, &[1_000_071, 1_000_072, 1_000_051]),
            [1_000_051]
        );
        assert_eq!(
            foreign(&settings, &[228_988]),
            [228_988],
            "a redistributable's is not its own"
        );
        let none = DEMO
            .split("\t\t\"1000071\"")
            .next()
            .map(|head| format!("{head}\t}}\n}}\n"))
            .unwrap();
        assert!(foreign(&demo(&none), &[1_000_071]).is_empty(), "{none}");
    }

    #[test]
    fn only_what_is_set_for_the_build_is_kept_from_what_steamcmd_printed() {
        let settings = from_console(CONSOLE, 1_000_010).unwrap();
        let written = snapshot(1_000_010, &settings).unwrap();
        assert_eq!(
            written,
            "// App 1000010's settings in Steamworks, kept by `steamship settings --save`;\n\
             // `steamship settings --check` says where Steam's differ.\n\
             \"1000010\"\n{\n\
             \t\"common\"\n\t{\n\
             \t\t\"name\"\t\t\"Falcon Ridge Overseer\"\n\
             \t\t\"type\"\t\t\"Game\"\n\
             \t\t\"oslist\"\t\t\"windows,linux\"\n\
             \t\t\"osarch\"\t\t\"64\"\n\
             \t}\n\
             \t\"config\"\n\t{\n\
             \t\t\"installdir\"\t\t\"FGM\"\n\
             \t\t\"launch\"\n\t\t{\n\
             \t\t\t\"0\"\n\t\t\t{\n\
             \t\t\t\t\"executable\"\t\t\"falcon-ridge-overseer.exe\"\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             \t\"depots\"\n\t{\n\
             \t\t\"1000011\"\n\t\t{\n\
             \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t}\n\
             \t\t}\n\
             \t\t\"workshopdepot\"\t\t\"1000010\"\n\
             \t}\n\
             \t\"ufs\"\n\t{\n\t\t\"quota\"\t\t\"1073741824\"\n\t}\n\
             }\n"
        );
        assert_eq!(
            read(&written, 1_000_010).unwrap(),
            settings,
            "read back as written"
        );
    }

    #[test]
    fn steam_cloud_is_said_to_be_set_up_for_everyone_for_developers_only_or_not_at_all() {
        let with = from_console(CONSOLE, 1_000_010).unwrap();
        assert_eq!(cloud(&with), "Steam Cloud set up in Steamworks");
        for (hidden, said) in [
            (
                "1",
                "Steam Cloud set up in Steamworks for developers only, so players have none: \
                 turn it on for everyone on the app's Steam Cloud page",
            ),
            ("0", "Steam Cloud set up in Steamworks"),
        ] {
            let text = format!(
                "\"ufs\" {{ \"quota\" \"100000000\" \"maxnumfiles\" \"200\" \
                 \"hidecloudui\" \"{hidden}\" }}"
            );
            assert_eq!(cloud(&vdf::parse(&text).unwrap()), said, "{text}");
        }
        for text in [
            "\"config\" { \"installdir\" \"Nocturne\" }",
            "\"ufs\" { }",
            "\"ufs\" \"0\"",
        ] {
            assert_eq!(
                cloud(&vdf::parse(text).unwrap()),
                "Steam Cloud not set up in Steamworks",
                "{text}"
            );
        }
    }

    #[test]
    fn an_install_folder_of_anything_but_letters_and_digits_is_refused() {
        let folder = |name: &str| {
            install_folder(
                &vdf::parse(&format!("\"config\" {{ \"installdir\" \"{name}\" }}")).unwrap(),
            )
        };
        assert_eq!(folder("TidalMillInn"), None);
        assert_eq!(folder("FGM2"), None);
        assert_eq!(
            install_folder(&from_console(CONSOLE, 1_000_010).unwrap()),
            None
        );
        assert_eq!(install_folder(&vdf::parse("\"ufs\" { }").unwrap()), None);
        for name in [
            "Tidal Mill Inn",
            "Tidal-Mill",
            "Brewhouse_2",
            "Inn\u{e9}",
            "",
        ] {
            assert_eq!(
                folder(name),
                Some(format!(
                    "the install folder is \"{name}\", and steamship takes letters and digits \
                     alone: change it in Steamworks under Installation, General Installation, \
                     then save the settings again"
                )),
                "{name}"
            );
        }
    }

    #[test]
    fn nothing_printed_or_shown_is_said_so() {
        assert_eq!(
            from_console("Logging in...FAILED\n", 1_000_010).unwrap_err(),
            "steamcmd printed no settings for app 1000010"
        );
        // As steamcmd printed an app ID the build account holds nothing for.
        assert!(
            from_console(
                "AppID : 3999999, change number : 1/1, last change : Thu Jan  1 01:00:00 1970 \n\
                 \"3999999\"\n{\n}\nUnloading Steam API...OK\n",
                3_999_999
            )
            .unwrap_err()
            .starts_with("Steam shows the build account no settings for app 3999999: ")
        );
        assert!(
            from_console("\"1\"\n{\n\t\"a\"\n}\n", 1)
                .unwrap_err()
                .starts_with("steamcmd printed settings for app 1 that cannot be read: ")
        );
    }

    #[test]
    fn a_snapshot_for_another_app_or_of_another_shape_is_refused() {
        assert_eq!(
            read("\"480\"\n{\n}\n", 1_000_010).unwrap_err(),
            "is for app 480, not app 1000010"
        );
        for text in ["\"480\"\t\"x\"\n", "", "\"1\"\n{\n}\n\"2\"\n{\n}\n"] {
            assert_eq!(
                read(text, 1).unwrap_err(),
                "is not one block named by the app's ID",
                "{text:?}"
            );
        }
        assert_eq!(
            read("\"app\"\n{\n}\n", 1).unwrap_err(),
            "\"app\" is not an app's ID"
        );
        assert!(
            read("\"1\"\n{\n", 1).unwrap_err().starts_with("line "),
            "the reader's own reason"
        );
    }

    #[test]
    fn drift_is_named_by_its_path_the_files_first_then_steams() {
        let file = vdf::parse(
            "\"config\" { \"installdir\" \"FGM\" \"launch\" { \"0\" { \"executable\" \"a.exe\" } } \
             \"gone\" \"1\" } \"ufs\" { \"quota\" \"1\" }",
        )
        .unwrap();
        let steam = vdf::parse(
            "\"CONFIG\" { \"installdir\" \"FGM\" \"launch\" { \"0\" { \"executable\" \"b.exe\" } \
             \"1\" { } } } \"ufs\" \"none\" \"extra\" \"x\"",
        )
        .unwrap();
        let said: Vec<String> = compare(&file, &steam)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            said,
            [
                "config/launch/0/executable is \"b.exe\" on Steam and \"a.exe\" in the file",
                "config/launch/1: on Steam, not in the file",
                "config/gone: in the file, not on Steam",
                "ufs is \"none\" on Steam and a block in the file",
                "extra: on Steam, not in the file",
            ]
        );
        assert_eq!(compare(&file, &file), []);
    }
}
