//! An app's leaderboards as a file in its repository says they should be, against Steam's.
//!
//! Steam makes a leaderboard it lacks as the file has it, but keeps an existing one's settings,
//! and its scores with them; so one that differs is reported, never changed or made again.

use std::collections::BTreeSet;
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

/// Reads the file's `text`: an object with a `leaderboards` list.
///
/// Each entry has its `name`, `sort` (`ascending` or `descending`) and `display` (`numeric`,
/// `seconds` or `milliseconds`), and `trusted_writes` and `friends_only` when they are set. Other
/// keys are left alone.
///
/// # Errors
///
/// When the text is not such an object, an entry lacks its name, sort or display or has one Steam
/// does not take, a value has the wrong type, or a name is there twice.
pub fn read(text: &str) -> Result<Listed, String> {
    let file: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let app = match file.get("app") {
        None => None,
        Some(app) => Some(app.as_u64().ok_or("\"app\" is not an app ID")?),
    };
    let entries = file
        .get("leaderboards")
        .and_then(Value::as_array)
        .ok_or("there is no \"leaderboards\" list")?;
    let mut seen = BTreeSet::new();
    let mut leaderboards = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let place = index.saturating_add(1);
        let name = match entry.get("name") {
            Some(Value::String(name)) if !name.is_empty() => name.clone(),
            _ => return Err(format!("leaderboard {place} has no \"name\" text")),
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
        let leaderboard = Leaderboard {
            sort: one_of("sort", &["Ascending", "Descending"])?,
            display: one_of("display", &["Numeric", "Seconds", "MilliSeconds"])?,
            trusted_writes: flag("trusted_writes")?,
            friends_only: flag("friends_only")?,
            entries: 0,
            name,
        };
        if !seen.insert(leaderboard.name.clone()) {
            return Err(format!("{} is in the file twice", leaderboard.name));
        }
        leaderboards.push(leaderboard);
    }
    Ok(Listed { app, leaderboards })
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
            r#"{"app": 5335950, "leaderboards": [
                {"name": "fastest_season", "sort": "ascending", "display": "SECONDS",
                 "trusted_writes": true, "icon": "ours"},
                {"name": "gold", "sort": "Descending", "display": "numeric", "friends_only": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(listed.app, Some(5_335_950));
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
            ("[]".to_owned(), "there is no \"leaderboards\" list"),
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
