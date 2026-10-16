//! The files Steamworks' achievement editor imports under Bulk Import/Export, written from an
//! app's achievements as [`achievements::together`](crate::achievements::together) reads them.

use std::iter;

use crate::achievements::{Listed, Wanted};

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
        // The editor reads a head's note in brackets as no part of it, so this one needs none.
        let mut names = row(iter::once("setid").chain(set_languages.iter().copied()));
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

/// The name the editor puts the icon of the achievement `api_name` on it by: the one shown once
/// it is unlocked, or while it is `locked`.
#[must_use]
pub fn icon_name(api_name: &str, locked: bool) -> String {
    let state = if locked { "UNACHIEVED" } else { "ACHIEVED" };
    format!("{api_name}_{state}.png")
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
            "setid,english\n1,The Cellar\n"
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
            "setid,english,german\n1,The Cellar,Der Keller\n"
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
