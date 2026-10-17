//! An app's leaderboards as a file in its repository says they should be, against Steam's.
//!
//! Steam makes a leaderboard it lacks as the file has it, but keeps an existing one's settings,
//! and its scores with them; so one that differs is reported, never changed or made again.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

use crate::show;
use crate::webapi::Leaderboard;

/// The longest name Steam takes, in bytes.
const LONGEST: usize = 128;

/// What the file holds: the app it is for, if it says, and the leaderboards in its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub app: Option<u64>,
    pub leaderboards: Vec<Leaderboard>,
}

/// Reads the file's `text`: an object with a `leaderboards` list, or a list of games-kit's
/// records, of which those typed `leaderboard` are read by the name in `steam`.
///
/// Each entry has its `name`, `sort` (`ascending` or `descending`) and `display` (`numeric`,
/// `seconds` or `milliseconds`), and `trusted_writes` and `friends_only` when they are set. Other
/// keys, among them a record's `upload` and `details`, which the game sends with each score, are
/// left alone.
///
/// # Errors
///
/// When the text is neither, an entry lacks its name, sort or display or has one Steam does not
/// take, a value has the wrong type, or a name is there twice.
pub fn read(text: &str) -> Result<Listed, String> {
    let file: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let mut leaderboards = Vec::new();
    let app = if let Value::Array(records) = &file {
        for (index, record) in records.iter().enumerate() {
            let label = format!("record {}", index.saturating_add(1));
            let Some(Value::String(kind)) = record.get("type") else {
                return Err(format!("{label} has no \"type\" text"));
            };
            if kind == "leaderboard" {
                leaderboards.push(leaderboard(record, &label, "steam")?);
            }
        }
        None
    } else {
        let entries = file
            .get("leaderboards")
            .and_then(Value::as_array)
            .ok_or("there is no \"leaderboards\" list")?;
        for (index, entry) in entries.iter().enumerate() {
            let label = format!("leaderboard {}", index.saturating_add(1));
            leaderboards.push(leaderboard(entry, &label, "name")?);
        }
        match file.get("app") {
            None => None,
            Some(app) => Some(app.as_u64().ok_or("\"app\" is not an app ID")?),
        }
    };
    let mut seen = BTreeSet::new();
    if let Some(again) = leaderboards
        .iter()
        .find(|leaderboard| !seen.insert(&leaderboard.name))
    {
        return Err(format!("{} is in the file twice", again.name));
    }
    Ok(Listed { app, leaderboards })
}

/// The leaderboard `entry`, its name under `named_by`, called `label` in what is wrong with it
/// until that name is known.
fn leaderboard(entry: &Value, label: &str, named_by: &str) -> Result<Leaderboard, String> {
    let name = match entry.get(named_by) {
        Some(Value::String(name)) if !name.is_empty() => name.clone(),
        _ => return Err(format!("{label} has no \"{named_by}\" text")),
    };
    if name.len() > LONGEST {
        return Err(format!("{name}: longer than Steam's {LONGEST} bytes"));
    }
    let one_of = |key: &str, words: &[&str]| -> Result<String, String> {
        let given = entry.get(key).and_then(Value::as_str).unwrap_or_default();
        words
            .iter()
            .find(|word| word.eq_ignore_ascii_case(given))
            .map(|&word| word.to_owned())
            .ok_or_else(|| {
                let lowered: Vec<String> =
                    words.iter().map(|word| word.to_ascii_lowercase()).collect();
                format!("{name}: \"{key}\" is not {}", or_list(&lowered))
            })
    };
    let flag = |key: &str| match entry.get(key) {
        None => Ok(false),
        Some(Value::Bool(on)) => Ok(*on),
        Some(_) => Err(format!("{name}: \"{key}\" is not true or false")),
    };
    Ok(Leaderboard {
        sort: one_of("sort", &["Ascending", "Descending"])?,
        display: one_of("display", &["Numeric", "Seconds", "MilliSeconds"])?,
        trusted_writes: flag("trusted_writes")?,
        friends_only: flag("friends_only")?,
        entries: 0,
        name,
    })
}

/// The leaderboards of the `files`, each read and named, of each in turn.
///
/// # Errors
///
/// When a leaderboard is in two of the files, naming both.
pub fn together(files: Vec<(String, Listed)>) -> Result<Vec<Leaderboard>, String> {
    let mut whole = Vec::new();
    let mut from: BTreeMap<String, String> = BTreeMap::new();
    for (file, listed) in files {
        for wanted in listed.leaderboards {
            if let Some(first) = from.insert(wanted.name.clone(), file.clone()) {
                return Err(format!("{} is in both {first} and {file}", wanted.name));
            }
            whole.push(wanted);
        }
    }
    Ok(whole)
}

/// `words` as a sentence gives a choice: `a`, `a or b`, `a, b or c`.
fn or_list(words: &[String]) -> String {
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => words.join(""),
    }
}

/// A way Steam's leaderboards and the file's differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// In the file, not on Steam.
    NotOnSteam(String),
    /// On Steam, not in the file.
    NotInFile(String),
    /// A setting that is one thing on Steam and another in the file.
    Differs {
        name: String,
        field: &'static str,
        steam: String,
        file: String,
    },
}

impl fmt::Display for Drift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOnSteam(name) => {
                write!(
                    formatter,
                    "{name}: in the file, not on Steam; --create makes it"
                )
            }
            Self::NotInFile(name) => write!(formatter, "{name}: on Steam, not in the file"),
            Self::Differs {
                name,
                field,
                steam,
                file,
            } => write!(
                formatter,
                "{name}: {field} is \"{steam}\" on Steam and \"{file}\" in the file"
            ),
        }
    }
}

/// `leaderboard` as `steamship leaderboards` lists it: its sort, its display, its scores, and
/// who may write and read them when that is narrowed.
#[must_use]
pub fn line(leaderboard: &Leaderboard) -> String {
    let mut line = format!(
        "{}, {}, {}",
        leaderboard.sort.to_ascii_lowercase(),
        display(&leaderboard.display),
        show::counted(
            usize::try_from(leaderboard.entries).unwrap_or(usize::MAX),
            "score"
        )
    );
    if leaderboard.trusted_writes {
        line.push_str(", trusted writes only");
    }
    if leaderboard.friends_only {
        line.push_str(", friends only");
    }
    line
}

/// A display type as it is shown: Steam leaves it empty when none was set.
fn display(display: &str) -> String {
    if display.is_empty() {
        "no display type".to_owned()
    } else {
        display.to_ascii_lowercase()
    }
}

/// Every way `held`, Steam's leaderboards, differ from those the file `wants`: first each of the
/// file's in its order, then those only Steam has, in Steam's.
#[must_use]
pub fn compare(wants: &[Leaderboard], held: &[Leaderboard]) -> Vec<Drift> {
    let mut drift = Vec::new();
    for wanted in wants {
        let Some(steam) = held
            .iter()
            .find(|leaderboard| leaderboard.name == wanted.name)
        else {
            drift.push(Drift::NotOnSteam(wanted.name.clone()));
            continue;
        };
        let yes_no = |on: bool| if on { "yes" } else { "no" }.to_owned();
        for (field, on_steam, in_file) in [
            (
                "the sort",
                steam.sort.to_ascii_lowercase(),
                wanted.sort.to_ascii_lowercase(),
            ),
            (
                "the display",
                display(&steam.display),
                display(&wanted.display),
            ),
            (
                "trusted writes only",
                yes_no(steam.trusted_writes),
                yes_no(wanted.trusted_writes),
            ),
            (
                "friends only",
                yes_no(steam.friends_only),
                yes_no(wanted.friends_only),
            ),
        ] {
            if on_steam != in_file {
                drift.push(Drift::Differs {
                    name: wanted.name.clone(),
                    field,
                    steam: on_steam,
                    file: in_file,
                });
            }
        }
    }
    drift.extend(
        held.iter()
            .filter(|steam| !wants.iter().any(|wanted| wanted.name == steam.name))
            .map(|steam| Drift::NotInFile(steam.name.clone())),
    );
    drift
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;

    fn board(name: &str, sort: &str, display: &str) -> Leaderboard {
        Leaderboard {
            name: name.to_owned(),
            sort: sort.to_owned(),
            display: display.to_owned(),
            trusted_writes: false,
            friends_only: false,
            entries: 0,
        }
    }

    #[test]
    fn a_file_is_read_with_steams_words_whatever_their_case() {
        let listed = read(
            r#"{"app": 1000010, "leaderboards": [
                {"name": "fastest_season", "sort": "ascending", "display": "SECONDS",
                 "trusted_writes": true, "icon": "ours"},
                {"name": "gold", "sort": "Descending", "display": "numeric", "friends_only": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(listed.app, Some(1_000_010));
        let mut fastest = board("fastest_season", "Ascending", "Seconds");
        fastest.trusted_writes = true;
        let mut gold = board("gold", "Descending", "Numeric");
        gold.friends_only = true;
        assert_eq!(listed.leaderboards, [fastest, gold]);
        assert_eq!(
            read(r#"{"leaderboards": []}"#).unwrap(),
            Listed {
                app: None,
                leaderboards: Vec::new()
            }
        );
    }

    #[test]
    fn a_file_steam_would_not_take_says_why() {
        let long = "x".repeat(129);
        for (text, why) in [
            ("{}".to_owned(), "there is no \"leaderboards\" list"),
            ("{".to_owned(), "not JSON"),
            (
                r#"{"app": "x", "leaderboards": []}"#.to_owned(),
                "\"app\" is not",
            ),
            (
                r#"{"leaderboards": [{"sort": "ascending"}]}"#.to_owned(),
                "leaderboard 1 has no \"name\"",
            ),
            (
                r#"{"leaderboards": [{"name": ""}]}"#.to_owned(),
                "leaderboard 1 has no \"name\"",
            ),
            (
                format!(r#"{{"leaderboards": [{{"name": "{long}"}}]}}"#),
                &format!("{long}: longer than Steam's 128 bytes"),
            ),
            (
                r#"{"leaderboards": [{"name": "a", "display": "numeric"}]}"#.to_owned(),
                "a: \"sort\" is not ascending or descending",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "up", "display": "numeric"}]}"#
                    .to_owned(),
                "a: \"sort\" is not",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "ascending", "display": "time"}]}"#
                    .to_owned(),
                "a: \"display\" is not numeric, seconds or milliseconds",
            ),
            (
                r#"{"leaderboards": [{"name": "a", "sort": "ascending", "display": "numeric",
                    "friends_only": "yes"}]}"#
                    .to_owned(),
                "a: \"friends_only\" is not true or false",
            ),
            (
                r#"{"leaderboards": [
                    {"name": "a", "sort": "ascending", "display": "numeric"},
                    {"name": "a", "sort": "ascending", "display": "numeric"}]}"#
                    .to_owned(),
                "a is in the file twice",
            ),
        ] {
            let error = read(&text).unwrap_err();
            assert!(error.starts_with(why), "{text}: {error}");
        }
        let longest = "x".repeat(128);
        assert!(
            read(&format!(
                r#"{{"leaderboards": [{{"name": "{longest}", "sort": "ascending", "display": "numeric"}}]}}"#
            ))
            .is_ok(),
            "Steam's longest is taken"
        );
    }

    #[test]
    fn games_kits_leaderboard_records_are_read_by_their_steam_name_and_others_left_alone() {
        let listed = read(
            r#"[{"type": "stat", "id": "base:floors", "steam": "floors"},
                {"type": "leaderboard", "id": "base:fastest_inn", "steam": "FASTEST_INN",
                 "name": "Fastest Inn", "sort": "ascending", "display": "seconds",
                 "upload": "keep_best", "details": 2},
                {"type": "leaderboard", "steam": "GOLD", "sort": "descending",
                 "display": "numeric", "upload": "force", "trusted_writes": true}]"#,
        )
        .unwrap();
        let mut gold = board("GOLD", "Descending", "Numeric");
        gold.trusted_writes = true;
        assert_eq!(
            listed,
            Listed {
                app: None,
                leaderboards: vec![board("FASTEST_INN", "Ascending", "Seconds"), gold],
            }
        );
        assert_eq!(read("[]").unwrap().leaderboards, []);
        for (text, why) in [
            (r#"[{"steam": "A"}]"#, "record 1 has no \"type\" text"),
            (
                r#"[{"type": "stat", "steam": "s"}, {"type": "leaderboard", "name": "A"}]"#,
                "record 2 has no \"steam\" text",
            ),
            (
                r#"[{"type": "leaderboard", "steam": "A", "sort": "ascending"}]"#,
                "A: \"display\" is not numeric, seconds or milliseconds",
            ),
            (
                r#"[{"type": "leaderboard", "steam": "A", "sort": "ascending", "display": "numeric"},
                    {"type": "leaderboard", "steam": "A", "sort": "ascending", "display": "numeric"}]"#,
                "A is in the file twice",
            ),
        ] {
            assert_eq!(read(text).unwrap_err(), why, "{text}");
        }
    }

    #[test]
    fn several_files_are_read_as_one_and_none_may_repeat_another() {
        let listed = |names: &[&str]| Listed {
            app: None,
            leaderboards: names
                .iter()
                .map(|name| board(name, "Ascending", "Numeric"))
                .collect(),
        };
        assert_eq!(
            together(vec![
                ("base.json".to_owned(), listed(&["a", "b"])),
                ("more.json".to_owned(), listed(&["c"])),
            ])
            .unwrap(),
            [
                board("a", "Ascending", "Numeric"),
                board("b", "Ascending", "Numeric"),
                board("c", "Ascending", "Numeric"),
            ]
        );
        assert_eq!(
            together(vec![
                ("base.json".to_owned(), listed(&["a"])),
                ("more.json".to_owned(), listed(&["a"])),
            ])
            .unwrap_err(),
            "a is in both base.json and more.json"
        );
    }

    #[test]
    fn a_choice_reads_as_a_sentence() {
        let words =
            |list: &[&str]| -> Vec<String> { list.iter().map(|&word| word.to_owned()).collect() };
        assert_eq!(or_list(&words(&[])), "");
        assert_eq!(or_list(&words(&["a"])), "a");
        assert_eq!(or_list(&words(&["a", "b"])), "a or b");
        assert_eq!(or_list(&words(&["a", "b", "c"])), "a, b or c");
    }

    #[test]
    fn a_leaderboard_is_listed_by_its_settings_and_scores() {
        let mut fastest = board("fastest_season", "Ascending", "Seconds");
        fastest.entries = 1;
        assert_eq!(line(&fastest), "ascending, seconds, 1 score");
        fastest.trusted_writes = true;
        fastest.friends_only = true;
        fastest.entries = 12;
        assert_eq!(
            line(&fastest),
            "ascending, seconds, 12 scores, trusted writes only, friends only"
        );
        assert_eq!(
            line(&board("gold", "Descending", "")),
            "descending, no display type, 0 scores"
        );
    }

    #[test]
    fn drift_is_every_setting_that_differs_and_every_leaderboard_on_one_side_only() {
        let fastest = board("fastest_season", "Ascending", "Seconds");
        let mut changed = board("fastest_season", "Descending", "");
        changed.trusted_writes = true;
        changed.friends_only = true;
        changed.entries = 40;
        let gold = board("gold", "Descending", "Numeric");
        let old = board("old", "Descending", "Numeric");
        assert_eq!(
            compare(
                &[fastest.clone(), gold.clone()],
                &[fastest.clone(), gold.clone()]
            ),
            [],
            "the scores are not the file's to say"
        );
        let mut shown_differently = fastest.clone();
        shown_differently.sort = "ASCENDING".to_owned();
        assert_eq!(compare(slice::from_ref(&fastest), &[shown_differently]), []);
        let drift = compare(&[fastest, gold], &[old, changed]);
        let said: Vec<String> = drift.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "fastest_season: the sort is \"descending\" on Steam and \"ascending\" in the file",
                "fastest_season: the display is \"no display type\" on Steam and \"seconds\" in \
                 the file",
                "fastest_season: trusted writes only is \"yes\" on Steam and \"no\" in the file",
                "fastest_season: friends only is \"yes\" on Steam and \"no\" in the file",
                "gold: in the file, not on Steam; --create makes it",
                "old: on Steam, not in the file",
            ]
        );
    }
}
