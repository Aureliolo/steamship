//! The settings made in Steamworks that a build depends on, as steamcmd's `app_info_print` shows
//! them to the build account, and a snapshot of them kept beside the build scripts.
//!
//! Steam shows its own bookkeeping beside them (each depot's manifests, the build on each
//! branch), and for a released app store details that change on Valve's side. Only what is set in
//! Steamworks for the build is kept, so that the snapshot differs from Steam when someone changed
//! the app there, and not after every upload.

use std::fmt;

use crate::vdf::{self, Block, Pair, Value};

/// The keys of `common` kept: what the app is, and the systems it runs on.
const COMMON: [&str; 5] = ["name", "type", "oslist", "osarch", "osextended"];

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
/// When steamcmd printed none, what it printed cannot be read, or Steam showed the build account
/// nothing, as for an app it cannot see.
pub fn from_console(console: &str, app: u32) -> Result<Block, String> {
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
    Ok(kept(shown))
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

    /// steamcmd's console as it printed Fantasy Guild Manager's app info, trimmed.
    const CONSOLE: &str = "Logging in user 'ACCOUNT' [U:1:1] to Steam Public...OK\n\
        AppID : 5335950, change number : 39317222/39317222, last change : Tue Sep 29 2026 \n\
        \"5335950\"\n{\n\
        \t\"common\"\n\t{\n\
        \t\t\"name\"\t\t\"Fantasy Guild Manager\"\n\
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
        \t\t\t\t\"executable\"\t\t\"fantasy-guild-manager.exe\"\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\
        \t\"depots\"\n\t{\n\
        \t\t\"5335951\"\n\t\t{\n\
        \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t}\n\
        \t\t\t\"manifests\"\n\t\t\t{\n\t\t\t\t\"public\"\n\t\t\t\t{\n\
        \t\t\t\t\t\"gid\"\t\t\"4987896756238774439\"\n\t\t\t\t}\n\t\t\t}\n\
        \t\t}\n\
        \t\t\"workshopdepot\"\t\t\"5335950\"\n\
        \t\t\"branches\"\n\t\t{\n\t\t\t\"public\"\n\t\t\t{\n\
        \t\t\t\t\"buildid\"\t\t\"25585928\"\n\t\t\t}\n\t\t}\n\
        \t\t\"privatebranches\"\t\t\"1\"\n\
        \t}\n\
        \t\"ufs\"\n\t{\n\t\t\"quota\"\t\t\"1073741824\"\n\t}\n\
        \t\"extended\"\n\t{\n\t\t\"developer\"\t\t\"Someone\"\n\t}\n\
        }\n\
        Unloading Steam API...OK\n";

    #[test]
    fn only_what_is_set_for_the_build_is_kept_from_what_steamcmd_printed() {
        let settings = from_console(CONSOLE, 5_335_950).unwrap();
        let written = snapshot(5_335_950, &settings).unwrap();
        assert_eq!(
            written,
            "// App 5335950's settings in Steamworks, kept by `steamship settings --save`;\n\
             // `steamship settings --check` says where Steam's differ.\n\
             \"5335950\"\n{\n\
             \t\"common\"\n\t{\n\
             \t\t\"name\"\t\t\"Fantasy Guild Manager\"\n\
             \t\t\"type\"\t\t\"Game\"\n\
             \t\t\"oslist\"\t\t\"windows,linux\"\n\
             \t\t\"osarch\"\t\t\"64\"\n\
             \t}\n\
             \t\"config\"\n\t{\n\
             \t\t\"installdir\"\t\t\"FGM\"\n\
             \t\t\"launch\"\n\t\t{\n\
             \t\t\t\"0\"\n\t\t\t{\n\
             \t\t\t\t\"executable\"\t\t\"fantasy-guild-manager.exe\"\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             \t\"depots\"\n\t{\n\
             \t\t\"5335951\"\n\t\t{\n\
             \t\t\t\"config\"\n\t\t\t{\n\t\t\t\t\"oslist\"\t\t\"windows\"\n\t\t\t}\n\
             \t\t}\n\
             \t\t\"workshopdepot\"\t\t\"5335950\"\n\
             \t}\n\
             \t\"ufs\"\n\t{\n\t\t\"quota\"\t\t\"1073741824\"\n\t}\n\
             }\n"
        );
        assert_eq!(
            read(&written, 5_335_950).unwrap(),
            settings,
            "read back as written"
        );
    }

    #[test]
    fn nothing_printed_or_shown_is_said_so() {
        assert_eq!(
            from_console("Logging in...FAILED\n", 5_335_950).unwrap_err(),
            "steamcmd printed no settings for app 5335950"
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
            read("\"480\"\n{\n}\n", 5_335_950).unwrap_err(),
            "is for app 480, not app 5335950"
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
