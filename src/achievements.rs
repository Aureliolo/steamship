//! An app's achievements as a file in its repository says they should be, against Steam's.
//!
//! This is what `steamship achievements --check` reports, so that a release can refuse to go out
//! while the two have drifted apart.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value;

use crate::webapi::{Achievement, Stat};

/// An achievement as the file wants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
}

/// A stat as the file wants it.
#[derive(Debug, Clone, PartialEq)]
pub struct WantedStat {
    pub api_name: String,
    pub name: String,
    pub default: f64,
}

/// What the file holds: the app it is for, if it says, the achievements in its order, and the
/// stats when it lists them, which are checked only then.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub app: Option<u64>,
    pub achievements: Vec<Wanted>,
    pub stats: Option<Vec<WantedStat>>,
}

/// Reads the file's `text`: an object with an `achievements` list, and a `stats` list if the
/// app's stats are to be checked too.
///
/// Each achievement has its `api_name` and `name`, and its `description` and `hidden` when they
/// are set; each stat its `api_name`, and its `name` and `default` when they are set. Other keys,
/// such as what a game's own tools keep beside them, are left alone.
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
    let stats = match file.get("stats") {
        None => None,
        Some(Value::Array(listed)) => Some(stats(listed)?),
        Some(_) => return Err("\"stats\" is not a list".to_owned()),
    };
    Ok(Listed {
        app,
        achievements,
        stats,
    })
}

/// The file's `stats` list.
fn stats(entries: &[Value]) -> Result<Vec<WantedStat>, String> {
    let mut seen = BTreeSet::new();
    let mut stats = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let api_name = match entry.get("api_name") {
            Some(Value::String(api_name)) if !api_name.is_empty() => api_name.clone(),
            _ => {
                let place = index.saturating_add(1);
                return Err(format!("stat {place} has no \"api_name\" text"));
            }
        };
        let name = match entry.get("name") {
            None => String::new(),
            Some(Value::String(name)) => name.clone(),
            Some(_) => return Err(format!("stat {api_name}: \"name\" is not text")),
        };
        let default = match entry.get("default") {
            None => 0.0_f64,
            Some(value) => value
                .as_f64()
                .ok_or_else(|| format!("stat {api_name}: \"default\" is not a number"))?,
        };
        if !seen.insert(api_name.clone()) {
            return Err(format!("stat {api_name} is in the file twice"));
        }
        stats.push(WantedStat {
            api_name,
            name,
            default,
        });
    }
    Ok(stats)
}

/// A way Steam's achievements and the file's differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// Steam holds no achievements at all, and the file this many: said once, not once for each.
    NoneOnSteam(usize),
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
            Self::NoneOnSteam(count) => write!(
                formatter,
                "Steam holds none of the file's {count} achievements: they count once entered and \
                 published in Steamworks"
            ),
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

/// `stat` as `steamship achievements` lists it: its name when it has one, and the value it starts
/// at.
#[must_use]
pub fn stat_line(stat: &Stat) -> String {
    if stat.name.is_empty() {
        format!("starts at {}", stat.default)
    } else {
        format!("{}, starts at {}", stat.name, stat.default)
    }
}

/// Every way `held`, Steam's stats, differ from those the file `wants`, each named as a stat:
/// first each of the file's in its order, then those only Steam has, in Steam's.
#[must_use]
pub fn compare_stats(wants: &[WantedStat], held: &[Stat]) -> Vec<Drift> {
    let label = |api_name: &str| format!("stat {api_name}");
    let mut drift = Vec::new();
    for wanted in wants {
        let Some(steam) = held.iter().find(|stat| stat.api_name == wanted.api_name) else {
            drift.push(Drift::NotOnSteam(label(&wanted.api_name)));
            continue;
        };
        if steam.name != wanted.name {
            drift.push(Drift::Differs {
                api_name: label(&wanted.api_name),
                field: "the display name",
                steam: steam.name.clone(),
                file: wanted.name.clone(),
            });
        }
        // Steam and the file both write a whole number with no fraction, or the same fraction,
        // so an exact comparison is what each says.
        #[expect(
            clippy::float_cmp,
            reason = "both are the value as written, not computed"
        )]
        if steam.default != wanted.default {
            drift.push(Drift::Differs {
                api_name: label(&wanted.api_name),
                field: "the default",
                steam: steam.default.to_string(),
                file: wanted.default.to_string(),
            });
        }
    }
    drift.extend(
        held.iter()
            .filter(|steam| !wants.iter().any(|wanted| wanted.api_name == steam.api_name))
            .map(|steam| Drift::NotInFile(label(&steam.api_name))),
    );
    drift
}

/// Every way `held`, Steam's achievements, differ from those the file `wants`: first each of the
/// file's in its order, then those only Steam has, in Steam's; or, when Steam holds none, that.
#[must_use]
pub fn compare(wants: &[Wanted], held: &[Achievement]) -> Vec<Drift> {
    if held.is_empty() && !wants.is_empty() {
        return vec![Drift::NoneOnSteam(wants.len())];
    }
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
        assert_eq!(listed.stats, None, "no list, so the stats are not checked");
        assert_eq!(read(r#"{"achievements": []}"#).unwrap().app, None);
    }

    #[test]
    fn stats_are_read_when_the_file_lists_them() {
        let listed = read(
            r#"{"achievements": [], "stats": [
                {"api_name": "NumGames", "name": "Games played", "default": 0},
                {"api_name": "AverageSpeed", "default": 1.5, "unit": "ours"},
                {"api_name": "Bare"}
            ]}"#,
        )
        .unwrap();
        let stat = |api_name: &str, name: &str, default: f64| WantedStat {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            default,
        };
        assert_eq!(
            listed.stats,
            Some(vec![
                stat("NumGames", "Games played", 0.0_f64),
                stat("AverageSpeed", "", 1.5_f64),
                stat("Bare", "", 0.0_f64),
            ])
        );
        assert_eq!(
            read(r#"{"achievements": [], "stats": []}"#).unwrap().stats,
            Some(Vec::new()),
            "an empty list checks that Steam holds none"
        );
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
            (
                r#"{"achievements": [], "stats": {}}"#,
                "\"stats\" is not a list",
            ),
            (
                r#"{"achievements": [], "stats": [{"name": "N"}]}"#,
                "stat 1 has no \"api_name\" text",
            ),
            (
                r#"{"achievements": [], "stats": [{"api_name": ""}]}"#,
                "stat 1 has no \"api_name\" text",
            ),
            (
                r#"{"achievements": [], "stats": [{"api_name": "n", "name": 1}]}"#,
                "stat n: \"name\" is not text",
            ),
            (
                r#"{"achievements": [], "stats": [{"api_name": "n", "default": "0"}]}"#,
                "stat n: \"default\" is not a number",
            ),
            (
                r#"{"achievements": [], "stats": [{"api_name": "n"}, {"api_name": "n"}]}"#,
                "stat n is in the file twice",
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
    fn steam_holding_none_is_said_once_and_an_empty_file_against_none_is_no_drift() {
        // As for Fantasy Guild Manager before its fifteen were entered in Steamworks.
        let wants = [wanted("a", "A"), wanted("b", "B")];
        let drift = compare(&wants, &[]);
        assert_eq!(drift, [Drift::NoneOnSteam(2)]);
        assert_eq!(
            drift.first().unwrap().to_string(),
            "Steam holds none of the file's 2 achievements: they count once entered and \
             published in Steamworks"
        );
        assert_eq!(compare(&[], &[]), Vec::<Drift>::new());
        assert_eq!(
            compare(&[], &[held("a", "A")]),
            [Drift::NotInFile("a".to_owned())]
        );
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

    #[test]
    fn a_stat_is_listed_by_its_name_and_default() {
        let stat = |name: &str, default: f64| Stat {
            api_name: "NumGames".to_owned(),
            name: name.to_owned(),
            default,
        };
        assert_eq!(
            stat_line(&stat("Games played", 0.0_f64)),
            "Games played, starts at 0"
        );
        assert_eq!(stat_line(&stat("", 1.5_f64)), "starts at 1.5");
    }

    #[test]
    fn every_stat_difference_is_named_as_a_stat_the_files_first_then_steams() {
        let wanted = |api_name: &str, name: &str, default: f64| WantedStat {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            default,
        };
        let held = |api_name: &str, name: &str, default: f64| Stat {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            default,
        };
        assert_eq!(
            compare_stats(
                &[wanted("NumGames", "Games played", 0.0_f64)],
                &[held("NumGames", "Games played", 0.0_f64)]
            ),
            []
        );
        let drift = compare_stats(
            &[
                wanted("NumGames", "Games played", 0.0_f64),
                wanted("Missing", "", 0.0_f64),
            ],
            &[
                held("Extra", "", 0.0_f64),
                held("NumGames", "Games", 1.5_f64),
            ],
        );
        let said: Vec<String> = drift.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "stat NumGames: the display name is \"Games\" on Steam and \"Games played\" in \
                 the file",
                "stat NumGames: the default is \"1.5\" on Steam and \"0\" in the file",
                "stat Missing: in the file, not on Steam",
                "stat Extra: on Steam, not in the file",
            ]
        );
    }
}
