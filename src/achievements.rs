//! An app's achievements as a file in its repository says they should be, against Steam's.
//!
//! This is what `steamship achievements --check` reports, so that a release can refuse to go out
//! while the two have drifted apart.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value;

use crate::webapi::Achievement;

/// An achievement as the file wants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
}

/// What the file holds: the app it is for, if it says, and the achievements in its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub app: Option<u64>,
    pub achievements: Vec<Wanted>,
}

/// Reads the file's `text`: an object with an `achievements` list.
///
/// Each entry has its `api_name` and `name`, and its `description` and `hidden` when they are
/// set. Other keys, such as what a game's own tools keep beside them, are left alone.
///
/// # Errors
///
/// When the text is not such an object, an entry lacks its API name or name, a value has the
/// wrong type, or an API name is there twice.
pub fn read(text: &str) -> Result<Listed, String> {
    let file: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let app = match file.get("app") {
        None => None,
        Some(app) => Some(app.as_u64().ok_or("\"app\" is not an app ID")?),
    };
    let entries = file
        .get("achievements")
        .and_then(Value::as_array)
        .ok_or("there is no \"achievements\" list")?;
    let mut seen = BTreeSet::new();
    let mut achievements = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let place = index.saturating_add(1);
        let words = |key: &str, needed: bool| -> Result<String, String> {
            match entry.get(key) {
                Some(Value::String(value)) if !(needed && value.is_empty()) => Ok(value.clone()),
                None if !needed => Ok(String::new()),
                _ => Err(format!("achievement {place} has no \"{key}\" text")),
            }
        };
        let api_name = words("api_name", true)?;
        let hidden = match entry.get("hidden") {
            None => false,
            Some(Value::Bool(hidden)) => *hidden,
            Some(_) => return Err(format!("{api_name}: \"hidden\" is not true or false")),
        };
        if !seen.insert(api_name.clone()) {
            return Err(format!("{api_name} is in the file twice"));
        }
        achievements.push(Wanted {
            name: words("name", true)?,
            description: words("description", false)?,
            hidden,
            api_name,
        });
    }
    Ok(Listed { app, achievements })
}

/// A way Steam's achievements and the file's differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// In the file, not on Steam.
    NotOnSteam(String),
    /// On Steam, not in the file.
    NotInFile(String),
    /// A field that is one thing on Steam and another in the file.
    Differs {
        api_name: String,
        field: &'static str,
        steam: String,
        file: String,
    },
    /// An icon Steam has none of.
    NoIcon {
        api_name: String,
        which: &'static str,
    },
}

impl fmt::Display for Drift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOnSteam(api_name) => {
                write!(formatter, "{api_name}: in the file, not on Steam")
            }
            Self::NotInFile(api_name) => write!(formatter, "{api_name}: on Steam, not in the file"),
            Self::Differs {
                api_name,
                field,
                steam,
                file,
            } => write!(
                formatter,
                "{api_name}: {field} is \"{steam}\" on Steam and \"{file}\" in the file"
            ),
            Self::NoIcon { api_name, which } => {
                write!(formatter, "{api_name}: Steam has no {which}")
            }
        }
    }
}

/// `achievement` as `steamship achievements` lists it: its name, whether it is hidden, and its
/// description when it has one.
#[must_use]
pub fn line(achievement: &Achievement) -> String {
    let mut line = achievement.name.clone();
    if achievement.hidden {
        line.push_str(", hidden");
    }
    if !achievement.description.is_empty() {
        line.push_str(": ");
        line.push_str(&achievement.description);
    }
    line
}

/// Every way `held`, Steam's achievements, differ from those the file `wants`: first each of the
/// file's in its order, then those only Steam has, in Steam's.
#[must_use]
pub fn compare(wants: &[Wanted], held: &[Achievement]) -> Vec<Drift> {
    let mut drift = Vec::new();
    for wanted in wants {
        let Some(steam) = held
            .iter()
            .find(|achievement| achievement.api_name == wanted.api_name)
        else {
            drift.push(Drift::NotOnSteam(wanted.api_name.clone()));
            continue;
        };
        let yes_no = |hidden: bool| if hidden { "yes" } else { "no" }.to_owned();
        for (field, on_steam, in_file) in [
            ("the display name", steam.name.clone(), wanted.name.clone()),
            (
                "the description",
                steam.description.clone(),
                wanted.description.clone(),
            ),
            ("hidden", yes_no(steam.hidden), yes_no(wanted.hidden)),
        ] {
            if on_steam != in_file {
                drift.push(Drift::Differs {
                    api_name: wanted.api_name.clone(),
                    field,
                    steam: on_steam,
                    file: in_file,
                });
            }
        }
        for (which, icon) in [("icon", &steam.icon), ("locked icon", &steam.icon_locked)] {
            if icon.is_empty() {
                drift.push(Drift::NoIcon {
                    api_name: wanted.api_name.clone(),
                    which,
                });
            }
        }
    }
    drift.extend(
        held.iter()
            .filter(|steam| !wants.iter().any(|wanted| wanted.api_name == steam.api_name))
            .map(|steam| Drift::NotInFile(steam.api_name.clone())),
    );
    drift
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(api_name: &str, name: &str) -> Wanted {
        Wanted {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            description: format!("{name}, described"),
            hidden: false,
        }
    }

    fn held(api_name: &str, name: &str) -> Achievement {
        Achievement {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            description: format!("{name}, described"),
            hidden: false,
            icon: format!("https://cdn/{api_name}.jpg"),
            icon_locked: format!("https://cdn/{api_name}_bw.jpg"),
        }
    }

    #[test]
    fn a_file_is_read_as_a_game_keeps_it_with_its_own_keys_left_alone() {
        // Shaped as Fantasy Guild Manager's steam/achievements.json.
        let listed = read(
            r#"{"app": 5335950, "achievements": [
                {"api_name": "first_job", "name": "Open for business",
                 "description": "A guild's first job came home done.", "hidden": false,
                 "glyph": "scroll-unfurled"},
                {"api_name": "secret", "name": "Hush", "hidden": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(listed.app, Some(5_335_950));
        assert_eq!(
            listed.achievements,
            [
                Wanted {
                    api_name: "first_job".to_owned(),
                    name: "Open for business".to_owned(),
                    description: "A guild's first job came home done.".to_owned(),
                    hidden: false,
                },
                Wanted {
                    api_name: "secret".to_owned(),
                    name: "Hush".to_owned(),
                    description: String::new(),
                    hidden: true,
                },
            ]
        );
        assert_eq!(read(r#"{"achievements": []}"#).unwrap().app, None);
    }

    #[test]
    fn a_file_that_is_not_one_says_why() {
        for (text, why) in [
            ("[", "not JSON: "),
            ("{}", "there is no \"achievements\" list"),
            (
                r#"{"app": "x", "achievements": []}"#,
                "\"app\" is not an app ID",
            ),
            (
                r#"{"achievements": [{"name": "A"}]}"#,
                "achievement 1 has no \"api_name\" text",
            ),
            (
                r#"{"achievements": [{"api_name": "a", "name": ""}]}"#,
                "achievement 1 has no \"name\" text",
            ),
            (
                r#"{"achievements": [{"api_name": "a", "name": "A", "description": 3}]}"#,
                "achievement 1 has no \"description\" text",
            ),
            (
                r#"{"achievements": [{"api_name": "a", "name": "A", "hidden": 1}]}"#,
                "a: \"hidden\" is not true or false",
            ),
            (
                r#"{"achievements": [{"api_name": "a", "name": "A"}, {"api_name": "a", "name": "B"}]}"#,
                "a is in the file twice",
            ),
        ] {
            let error = read(text).unwrap_err();
            assert!(error.starts_with(why), "{text}: {error}");
        }
    }

    #[test]
    fn an_achievement_is_listed_by_name_hidden_flag_and_description() {
        let mut orbiter = held("ACH_TRAVEL_FAR_SINGLE", "Orbiter");
        assert_eq!(line(&orbiter), "Orbiter: Orbiter, described");
        orbiter.hidden = true;
        orbiter.description = String::new();
        assert_eq!(line(&orbiter), "Orbiter, hidden");
    }

    #[test]
    fn what_matches_is_no_drift() {
        assert_eq!(
            compare(&[wanted("a", "A")], &[held("a", "A")]),
            Vec::<Drift>::new()
        );
    }

    #[test]
    fn every_difference_is_named_the_files_first_then_steams() {
        let mut renamed = held("charter_town", "Town charter");
        renamed.description = "Old words".to_owned();
        renamed.hidden = true;
        renamed.icon_locked = String::new();
        let mut no_icon = held("b", "B");
        no_icon.icon = String::new();
        let drift = compare(
            &[
                wanted("charter_town", "Charter of the town"),
                wanted("missing", "M"),
                wanted("b", "B"),
            ],
            &[renamed, held("extra", "E"), no_icon],
        );
        let said: Vec<String> = drift.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "charter_town: the display name is \"Town charter\" on Steam and \"Charter of the \
                 town\" in the file",
                "charter_town: the description is \"Old words\" on Steam and \"Charter of the \
                 town, described\" in the file",
                "charter_town: hidden is \"yes\" on Steam and \"no\" in the file",
                "charter_town: Steam has no locked icon",
                "missing: in the file, not on Steam",
                "b: Steam has no icon",
                "extra: on Steam, not in the file",
            ]
        );
    }
}
