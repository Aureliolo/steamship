//! The files Steamworks' achievement editor imports under Bulk Import/Export, written from an
//! app's achievements as [`achievements::together`](crate::achievements::together) reads them.

use std::collections::BTreeMap;
use std::iter;

use serde_json::{Map, Value, json};

use crate::achievements::{Drift, IconChange, Listed, Wanted};
use crate::digest;
use crate::webapi::Achievement;

/// The file in the folder of an import that records, once Steam has taken it, each icon it took.
/// The editor imports nothing of this name, so `--write` leaves it alone.
pub const RECEIPT: &str = "steam-icons.json";

/// The definitions' column heads, worded as the editor exports them.
const DEFINED: [&str; 8] = [
    "api_name (required, unique; the name your game passes to the API)",
    "setid (optional; the id of a set that already exists)",
    "permission (one of Client, GameServer, OfficialGameServer)",
    "spoiler (true or false)",
    "archived (true or false)",
    "progress_stat_name (optional; the name of a stat that already exists, blank for no progress \
     bar)",
    "progress_stat_min (number, no lower than the stat's own min)",
    "progress_stat_max (number, above min and no higher than the stat's own max)",
];

/// The localisation's first two column heads, worded as the editor exports them.
const LOCALISED: [&str; 2] = [
    "api_name (required; must match an api_name from the definitions CSV)",
    "field (name or description)",
];

/// The set localisation's first column head, worded as the editor exports it.
const SET: &str = "setid (required; the id of a set that already exists)";

/// One file the editor imports: its name, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub text: String,
}

/// The editor's three tables for `app`, from `listed`: the definitions, the achievements' names
/// and descriptions in each language, and the sets' names when there are sets.
///
/// Without translations the texts are the English ones alone; a set's name is English alone
/// unless its translations were read.
#[must_use]
pub fn tables(app: u32, listed: &Listed) -> Vec<Table> {
    let mut definitions = row(DEFINED);
    for wanted in &listed.achievements {
        definitions.push_str(&definition(wanted));
    }
    let languages = if listed.translations.is_empty() {
        vec!["english"]
    } else {
        listed.languages()
    };
    let mut localisation = row(LOCALISED.iter().copied().chain(languages.iter().copied()));
    for wanted in &listed.achievements {
        let translated = listed
            .translations
            .iter()
            .find(|translated| translated.api_name == wanted.api_name);
        for field in ["name", "description"] {
            let texts = languages.iter().map(|language| {
                let found = translated.and_then(|translated| {
                    translated
                        .texts
                        .iter()
                        .find(|text| text.language == *language)
                });
                match (found, field) {
                    (Some(text), "name") => text.name.as_str(),
                    (Some(text), _) => text.description.as_str(),
                    (None, "name") => wanted.name.as_str(),
                    (None, _) => wanted.description.as_str(),
                }
            });
            localisation.push_str(&row([wanted.api_name.as_str(), field]
                .into_iter()
                .chain(texts)));
        }
    }
    let mut written = vec![
        Table {
            name: format!("{app}-achievements-definitions.csv"),
            text: definitions,
        },
        Table {
            name: format!("{app}-achievements-localization.csv"),
            text: localisation,
        },
    ];
    if !listed.sets.is_empty() {
        let set_languages: Vec<&str> = listed.set_names.first().map_or_else(
            || vec!["english"],
            |first| {
                first
                    .names
                    .iter()
                    .map(|(language, _)| language.as_str())
                    .collect()
            },
        );
        let mut names = row(iter::once(SET).chain(set_languages.iter().copied()));
        for set in &listed.sets {
            let translated = listed
                .set_names
                .iter()
                .find(|translated| translated.setid == set.setid);
            let texts = set_languages.iter().map(|language| {
                translated
                    .and_then(|translated| {
                        translated.names.iter().find(|(each, _)| each == language)
                    })
                    .map_or(set.name.as_str(), |(_, name)| name.as_str())
            });
            names.push_str(&row(iter::once(set.setid.as_str()).chain(texts)));
        }
        written.push(Table {
            name: format!("{app}-achievement-sets-localization.csv"),
            text: names,
        });
    }
    written
}

/// `wanted`'s row of the definitions.
fn definition(wanted: &Wanted) -> String {
    let defined = &wanted.defined;
    let (stat, most) = defined.progress.as_ref().map_or_else(
        || (String::new(), "0".to_owned()),
        |progress| (progress.stat.clone(), progress.at.to_string()),
    );
    row([
        wanted.api_name.as_str(),
        defined.set.as_deref().unwrap_or_default(),
        defined.permission.as_deref().unwrap_or("Client"),
        if wanted.hidden { "true" } else { "false" },
        if defined.archived { "true" } else { "false" },
        &stat,
        "0",
        &most,
    ])
}

/// Each achievement whose progress bar the definitions `after` give otherwise than `before` did,
/// both as [`tables`] writes them: its API name, and its bar in each as the editor shows it.
#[must_use]
pub fn rebarred(before: &str, after: &str) -> Vec<(String, String, String)> {
    let was = progress_bars(before);
    progress_bars(after)
        .into_iter()
        .filter_map(|(api_name, bar)| {
            let (_, old) = was.iter().find(|(each, _)| *each == api_name)?;
            (*old != bar).then(|| (api_name, old.clone(), bar))
        })
        .collect()
}

/// Each achievement in `definitions` and its progress bar, as the editor shows it: the stat it
/// counts and the values it runs between, or "no progress bar".
fn progress_bars(definitions: &str) -> Vec<(String, String)> {
    definitions
        .lines()
        .skip(1)
        .filter_map(|line| {
            // An API name, a set ID, a permission, two flags, a stat's name and two numbers: no
            // field of a row has a comma or a quote to escape, as the header's do.
            let fields: Vec<&str> = line.split(',').collect();
            let [api_name, _, _, _, _, stat, least, most] = fields.as_slice() else {
                return None;
            };
            let bar = if stat.is_empty() {
                "no progress bar".to_owned()
            } else {
                format!("{stat} {least} - {most}")
            };
            Some(((*api_name).to_owned(), bar))
        })
        .collect()
}

/// The name the editor puts the icon of the achievement `api_name` on it by: the one shown once
/// it is unlocked, or while it is `locked`.
#[must_use]
pub fn icon_name(api_name: &str, locked: bool) -> String {
    let state = if locked { "UNACHIEVED" } else { "ACHIEVED" };
    format!("{api_name}_{state}.png")
}

/// An achievement's icon as the files give it, named as the editor imports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub api_name: String,
    /// Whether it is the icon shown before the achievement is unlocked.
    pub locked: bool,
    pub name: String,
    pub bytes: Vec<u8>,
}

impl Icon {
    /// Which of its achievement's two icons it is, as a person reads it.
    #[must_use]
    pub const fn which(&self) -> &'static str {
        if self.locked { "locked icon" } else { "icon" }
    }

    /// The SHA-256 of the file, in hexadecimal.
    #[must_use]
    pub fn sha256(&self) -> String {
        let mut bytes = self.bytes.as_slice();
        digest::sha256(&mut bytes).map_or_else(|_| String::new(), |sum| digest::hex(&sum))
    }

    /// The address Steam gives this icon in `held`, its achievements; empty when it has none.
    #[must_use]
    pub fn on_steam<'held>(&self, held: &'held [Achievement]) -> &'held str {
        held.iter()
            .find(|achievement| achievement.api_name == self.api_name)
            .map_or("", |achievement| {
                if self.locked {
                    achievement.icon_locked.as_str()
                } else {
                    achievement.icon.as_str()
                }
            })
    }
}

/// The image Steam made of an icon, by the name it serves it under at the end of its `address`.
/// Steam serves one image from more than one host, so the name alone tells one from another.
#[must_use]
pub fn steam_image(address: &str) -> &str {
    address.rsplit_once('/').map_or(address, |(_, name)| name)
}

/// One icon as an import left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The SHA-256 of the file imported, in hexadecimal.
    pub sha256: String,
    /// The image Steam made of it, as [`steam_image`] names it.
    pub steam: String,
}

/// What an import left on Steam of each icon, by the name of its file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Receipt {
    pub app: u32,
    pub icons: BTreeMap<String, Imported>,
}

impl Receipt {
    /// The `icons` as Steam took them for `app`, with the address each has in `held`.
    #[must_use]
    pub fn of(app: u32, icons: &[Icon], held: &[Achievement]) -> Self {
        Self {
            app,
            icons: icons
                .iter()
                .map(|icon| {
                    (
                        icon.name.clone(),
                        Imported {
                            sha256: icon.sha256(),
                            steam: steam_image(icon.on_steam(held)).to_owned(),
                        },
                    )
                })
                .collect(),
        }
    }

    /// The receipt as it is kept: JSON, a key to a line in a set order, so that a change to one
    /// icon changes its lines alone.
    #[must_use]
    pub fn text(&self) -> String {
        let icons: Map<String, Value> = self
            .icons
            .iter()
            .map(|(name, imported)| {
                (
                    name.clone(),
                    json!({"sha256": imported.sha256, "steam": imported.steam}),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({"app": self.app, "icons": icons}))
            .unwrap_or_default();
        format!("{text}\n")
    }

    /// The receipt kept as `text`.
    ///
    /// # Errors
    ///
    /// When it is not one.
    pub fn read(text: &str) -> Result<Self, String> {
        let kept: Value =
            serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        let app = kept
            .get("app")
            .and_then(Value::as_u64)
            .and_then(|app| u32::try_from(app).ok())
            .ok_or("\"app\" is not an app ID")?;
        let icons = kept
            .get("icons")
            .and_then(Value::as_object)
            .ok_or("there is no \"icons\" object")?
            .iter()
            .map(|(name, imported)| {
                let field = |key: &str| {
                    imported
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("{name} has no \"{key}\" text"))
                };
                Ok((
                    name.clone(),
                    Imported {
                        sha256: field("sha256")?,
                        steam: field("steam")?,
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { app, icons })
    }

    /// How each of the `icons` differs from what the receipt says Steam took of it, as Steam's
    /// achievements, `held`, now give it. An icon Steam has none of is left to the check that
    /// names it so.
    #[must_use]
    pub fn drift(&self, icons: &[Icon], held: &[Achievement]) -> Vec<Drift> {
        icons
            .iter()
            .filter_map(|icon| {
                let address = icon.on_steam(held);
                if address.is_empty() {
                    return None;
                }
                let change = match self.icons.get(&icon.name) {
                    None => IconChange::Unrecorded,
                    Some(imported) if imported.sha256 != icon.sha256() => {
                        IconChange::ChangedInFiles
                    }
                    Some(imported) if steam_image(address) != imported.steam => {
                        IconChange::ChangedOnSteam
                    }
                    Some(_) => return None,
                };
                Some(Drift::IconImport {
                    api_name: icon.api_name.clone(),
                    which: icon.which(),
                    change,
                })
            })
            .collect()
    }
}

/// Whether the file `name` is of a kind written for `app`, so that one left in the folder from
/// an achievement since removed is found.
#[must_use]
pub fn written_for(app: u32, name: &str) -> bool {
    [
        "achievements-definitions.csv",
        "achievements-localization.csv",
        "achievement-sets-localization.csv",
    ]
    .iter()
    .any(|table| name == format!("{app}-{table}"))
        || name.ends_with("_ACHIEVED.png")
        || name.ends_with("_UNACHIEVED.png")
}

/// The `fields` as a line of CSV, each quoted only when it holds a comma, a quote or a line
/// break, as the editor's own export does.
fn row<'field>(fields: impl IntoIterator<Item = &'field str>) -> String {
    let mut line = String::new();
    for (index, field) in fields.into_iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        if field.contains([',', '"', '\n', '\r']) {
            line.push('"');
            line.push_str(&field.replace('"', "\"\""));
            line.push('"');
        } else {
            line.push_str(field);
        }
    }
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievements::{self, Catalogued};
    use crate::po;

    const INN: &str = r#"[
        {"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"},
        {"type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar"},
        {"type": "achievement", "id": "base:five_floors", "steam": "FIVE_FLOORS",
         "name": "Five Floors Up", "description": "Stand an inn five floors above its taproom.",
         "stat": "base:floors", "at": 5},
        {"type": "achievement", "id": "base:first_cellar", "steam": "FIRST_CELLAR",
         "name": "Down in the Cellar", "description": "Dig an inn's \"first\" cellar, at last.",
         "set": "base:cellar", "archived": true, "hidden": true, "permission": "game_server"}
    ]"#;

    /// What the table of `written` named `kind` after the app holds.
    fn table<'written>(written: &'written [Table], kind: &str) -> &'written str {
        &written
            .iter()
            .find(|table| table.name == format!("5440760{kind}"))
            .unwrap()
            .text
    }

    /// An achievement on Steam whose icons are at `icon` and `icon_locked`.
    fn held(api_name: &str, icon: &str, icon_locked: &str) -> Achievement {
        Achievement {
            api_name: api_name.to_owned(),
            name: String::new(),
            description: String::new(),
            hidden: false,
            icon: icon.to_owned(),
            icon_locked: icon_locked.to_owned(),
        }
    }

    fn icon(api_name: &str, locked: bool, bytes: &[u8]) -> Icon {
        Icon {
            api_name: api_name.to_owned(),
            locked,
            name: icon_name(api_name, locked),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn a_receipt_records_each_icons_file_and_steams_image_and_reads_back_the_same() {
        let icons = [icon("A", false, b"a"), icon("A", true, b"b")];
        let steam = [held("A", "https://cdn/1/aa.jpg", "https://cdn/1/bb.jpg")];
        let receipt = Receipt::of(5_440_760, &icons, &steam);
        let text = receipt.text();
        assert_eq!(
            text,
            "{\n  \"app\": 5440760,\n  \"icons\": {\n    \"A_ACHIEVED.png\": {\n      \
             \"sha256\": \"ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb\",\n      \
             \"steam\": \"aa.jpg\"\n    },\n    \"A_UNACHIEVED.png\": {\n      \
             \"sha256\": \"3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d\",\n      \
             \"steam\": \"bb.jpg\"\n    }\n  }\n}\n"
        );
        assert_eq!(Receipt::read(&text), Ok(receipt));
    }

    #[test]
    fn a_receipt_that_is_not_one_is_refused_saying_why() {
        for (text, why) in [
            ("[", "not JSON: EOF while parsing a list at line 1 column 1"),
            ("{\"icons\": {}}", "\"app\" is not an app ID"),
            (
                "{\"app\": 4294967296, \"icons\": {}}",
                "\"app\" is not an app ID",
            ),
            ("{\"app\": 1}", "there is no \"icons\" object"),
            (
                "{\"app\": 1, \"icons\": {\"A_ACHIEVED.png\": {\"sha256\": \"00\"}}}",
                "A_ACHIEVED.png has no \"steam\" text",
            ),
            (
                "{\"app\": 1, \"icons\": {\"A_ACHIEVED.png\": {\"steam\": \"a.jpg\"}}}",
                "A_ACHIEVED.png has no \"sha256\" text",
            ),
        ] {
            assert_eq!(Receipt::read(text), Err(why.to_owned()), "{text}");
        }
    }

    #[test]
    fn an_icon_moved_from_the_recorded_import_is_named_and_one_steam_lacks_is_left_alone() {
        let imported = [
            icon("SAME", false, b"same"),
            icon("FILE", false, b"old"),
            icon("STEAM", true, b"steam"),
            icon("BARE", false, b"bare"),
        ];
        let before = [
            held("SAME", "https://cdn/s.jpg", ""),
            held("FILE", "https://cdn/f.jpg", ""),
            held("STEAM", "", "https://cdn/t.jpg"),
            held("BARE", "https://cdn/b.jpg", ""),
        ];
        let receipt = Receipt::of(1, &imported, &before);
        let now = [
            icon("SAME", false, b"same"),
            icon("FILE", false, b"new"),
            icon("STEAM", true, b"steam"),
            icon("BARE", false, b"bare"),
            icon("NEW", false, b"new"),
            icon("UNIMPORTED", false, b"unimported"),
        ];
        let after = [
            held("SAME", "https://other-host/s.jpg", ""),
            held("FILE", "https://cdn/f.jpg", ""),
            held("STEAM", "", "https://cdn/u.jpg"),
            held("BARE", "", ""),
            held("NEW", "https://cdn/n.jpg", ""),
        ];
        let named: Vec<String> = receipt
            .drift(&now, &after)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            named,
            [
                "FILE: the icon changed in the files since the import steam-icons.json records: \
                 import it, publish, then run --imported",
                "STEAM: the locked icon on Steam changed since the import steam-icons.json \
                 records, and not from the files: import it again, publish, then run --imported",
                "NEW: the icon has no import in steam-icons.json: import it, publish, then run \
                 --imported",
            ]
        );
    }

    #[test]
    fn steams_image_is_named_by_the_end_of_its_address() {
        assert_eq!(steam_image("https://cdn/apps/1/ab.jpg"), "ab.jpg");
        assert_eq!(steam_image("ab.jpg"), "ab.jpg");
    }

    fn inn() -> Listed {
        achievements::together(vec![(
            "achievements.json".to_owned(),
            achievements::read(INN).unwrap(),
        )])
        .unwrap()
    }

    #[test]
    fn the_definitions_give_each_achievement_its_set_flags_and_progress_bar() {
        let written = tables(5_440_760, &inn());
        let lines: Vec<&str> = table(&written, "-achievements-definitions.csv")
            .lines()
            .collect();
        assert_eq!(
            lines.first().copied(),
            Some(
                "\"api_name (required, unique; the name your game passes to the API)\",setid \
                 (optional; the id of a set that already exists),\"permission (one of Client, \
                 GameServer, OfficialGameServer)\",spoiler (true or false),archived (true or \
                 false),\"progress_stat_name (optional; the name of a stat that already exists, \
                 blank for no progress bar)\",\"progress_stat_min (number, no lower than the \
                 stat's own min)\",\"progress_stat_max (number, above min and no higher than the \
                 stat's own max)\""
            )
        );
        assert_eq!(
            lines.get(1..),
            Some(
                [
                    "FIVE_FLOORS,,Client,false,false,floors,0,5",
                    "FIRST_CELLAR,1,GameServer,true,true,,0,0",
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn a_progress_bar_changed_since_the_last_definitions_is_named_with_both_ranges() {
        let head = "\"api_name (required, unique)\",setid\n";
        let before = format!(
            "{head}FULL_GRIMOIRE,,Client,false,false,REACTIONS_FOUND,0,12\n\
             FIRST_DAY,,Client,false,false,,0,0\n\
             STAYED_ON,,Client,false,false,DAYS,0,30\n"
        );
        let after = format!(
            "{head}FULL_GRIMOIRE,,Client,false,false,REACTIONS_FOUND,0,11\n\
             FIRST_DAY,,Client,false,false,DAYS,0,1\n\
             STAYED_ON,,Client,true,false,DAYS,0,30\n\
             NEW_ONE,,Client,false,false,DAYS,0,2\n"
        );
        assert_eq!(
            rebarred(&before, &after),
            [
                (
                    "FULL_GRIMOIRE".to_owned(),
                    "REACTIONS_FOUND 0 - 12".to_owned(),
                    "REACTIONS_FOUND 0 - 11".to_owned()
                ),
                (
                    "FIRST_DAY".to_owned(),
                    "no progress bar".to_owned(),
                    "DAYS 0 - 1".to_owned()
                ),
            ],
            "neither a flag changed nor an achievement new to the definitions"
        );
        assert_eq!(rebarred("", &after), []);
    }

    #[test]
    fn the_texts_are_english_alone_without_translations_and_quoted_where_csv_needs_it() {
        let written = tables(5_440_760, &inn());
        assert_eq!(
            table(&written, "-achievements-localization.csv"),
            "api_name (required; must match an api_name from the definitions CSV),field (name or \
             description),english\n\
             FIVE_FLOORS,name,Five Floors Up\n\
             FIVE_FLOORS,description,Stand an inn five floors above its taproom.\n\
             FIRST_CELLAR,name,Down in the Cellar\n\
             FIRST_CELLAR,description,\"Dig an inn's \"\"first\"\" cellar, at last.\"\n"
        );
        assert_eq!(
            table(&written, "-achievement-sets-localization.csv"),
            "setid (required; the id of a set that already exists),english\n1,The Cellar\n"
        );
    }

    #[test]
    fn each_language_is_a_column_of_both_texts_and_of_the_sets_names() {
        let mut listed = inn();
        let german = "msgctxt \"achievement.name\"\nmsgid \"base:five_floors\"\n\
            msgstr \"F\u{fc}nf Stock hoch\"\n\
            msgctxt \"achievement.description\"\nmsgid \"base:five_floors\"\n\
            msgstr \"Stelle eine Herberge f\u{fc}nf Stock \u{fc}ber ihre Schankstube.\"\n\
            msgctxt \"achievement.name\"\nmsgid \"base:first_cellar\"\nmsgstr \"Unten im Keller\"\n\
            msgctxt \"achievement.description\"\nmsgid \"base:first_cellar\"\n\
            msgstr \"Grabe den ersten Keller einer Herberge.\"\n\
            msgctxt \"achievement_set.name\"\nmsgid \"base:cellar\"\nmsgstr \"Der Keller\"\n";
        achievements::translate(
            &mut listed,
            &[Catalogued {
                file: "de.po".to_owned(),
                language: "german",
                catalogue: po::read(german).unwrap(),
            }],
        )
        .unwrap();
        let written = tables(5_440_760, &listed);
        let localisation: Vec<&str> = table(&written, "-achievements-localization.csv")
            .lines()
            .collect();
        assert!(
            localisation.first().unwrap().ends_with(",english,german"),
            "{localisation:?}"
        );
        assert_eq!(
            localisation.get(1).copied(),
            Some("FIVE_FLOORS,name,Five Floors Up,F\u{fc}nf Stock hoch")
        );
        assert_eq!(localisation.len(), 5);
        assert_eq!(
            table(&written, "-achievement-sets-localization.csv"),
            "setid (required; the id of a set that already exists),english,german\n\
             1,The Cellar,Der Keller\n"
        );
    }

    #[test]
    fn no_sets_write_no_file_of_their_names() {
        let mut listed = inn();
        listed.sets.clear();
        assert_eq!(tables(480, &listed).len(), 2);
    }

    #[test]
    fn icons_are_named_for_their_achievement_and_only_the_kinds_written_are_counted_as_written() {
        assert_eq!(
            icon_name("FIRST_CELLAR", false),
            "FIRST_CELLAR_ACHIEVED.png"
        );
        assert_eq!(
            icon_name("FIRST_CELLAR", true),
            "FIRST_CELLAR_UNACHIEVED.png"
        );
        for (name, written) in [
            ("480-achievements-definitions.csv", true),
            ("480-achievements-localization.csv", true),
            ("480-achievement-sets-localization.csv", true),
            ("A_ACHIEVED.png", true),
            ("A_UNACHIEVED.png", true),
            ("481-achievements-definitions.csv", false),
            ("notes.md", false),
            ("A.png", false),
        ] {
            assert_eq!(written_for(480, name), written, "{name}");
        }
    }
}
