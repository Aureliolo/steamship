//! An app's achievements as a file in its repository says they should be, against Steam's.
//!
//! This is what `steamship achievements --check` reports, so that a release can refuse to go out
//! while the two have drifted apart, and what `--write` turns into the files Steamworks'
//! achievement editor imports.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use serde_json::Value;

use crate::po::Catalogue;
use crate::presence;
use crate::show::counted;
use crate::webapi::{Achievement, Stat};

/// The most achievements Steam takes for an app, archived ones included.
pub const MOST_ACHIEVEMENTS: usize = 5_000;
/// The most it takes before Valve has vetted the app.
pub const MOST_UNVETTED: usize = 100;
/// The most sets it takes, the base set included.
pub const MOST_SETS: usize = 40;

/// Who may unlock an achievement, as the achievement editor words it and as godot-kit's records
/// do.
const PERMISSIONS: [(&str, &str); 3] = [
    ("Client", "client"),
    ("GameServer", "game_server"),
    ("OfficialGameServer", "official_game_server"),
];

/// An achievement as the file wants it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    /// What its translations are kept under: the record's `id`, or else its API name.
    pub id: String,
    pub defined: Definition,
}

/// What the achievement editor defines of an achievement beside its texts and whether it is
/// hidden, none of which Steam's Web API answers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Definition {
    /// The set it is in, none for the base set: as the file names it, until [`together`] makes it
    /// the set's ID on Steam.
    pub set: Option<String>,
    /// It can no longer be earned.
    pub archived: bool,
    /// Who may unlock it, none for `Client`.
    pub permission: Option<String>,
    pub progress: Option<Progress>,
    /// Its icon and the one shown while it is locked, where the file gives them.
    pub icon: Option<PathBuf>,
    pub icon_locked: Option<PathBuf>,
}

/// The stat an achievement's progress bar counts, from 0, and the value of it that unlocks it.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// As the file names it, until [`together`] makes it the stat's API name.
    pub stat: String,
    pub at: f64,
}

/// A stat as the file wants it, its name and default checked only when the file gives them.
#[derive(Debug, Clone, PartialEq)]
pub struct WantedStat {
    pub api_name: String,
    pub name: Option<String>,
    pub default: Option<f64>,
    /// The record's `id`, which an achievement may name it by.
    pub id: Option<String>,
    /// Whether it counts in whole steps, when the file says.
    pub whole: Option<bool>,
}

/// A set of achievements as the file wants it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WantedSet {
    /// Its ID on Steam, which it is given when it is made in the achievement editor.
    pub setid: String,
    /// Its name in English.
    pub name: String,
    /// What achievements name it by and its translations are kept under: the record's `id`, or
    /// else its set ID.
    pub id: String,
}

/// A set's name in each language, in the order of its localisation's columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTranslated {
    pub setid: String,
    /// Each language by Steam's name for it, with the set's name in it.
    pub names: Vec<(String, String)>,
}

/// A gettext catalogue, from the file it was read from, in the Steam language it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogued {
    pub file: String,
    pub language: &'static str,
    pub catalogue: Catalogue,
}

/// An achievement's name and description in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    /// Steam's name for the language, such as `german` or `schinese`.
    pub language: String,
    pub name: String,
    pub description: String,
}

/// An achievement's name and description in each language a localisation file gives them, in
/// its column order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    pub api_name: String,
    pub texts: Vec<Translation>,
}

/// What the file holds, and what in it goes unchecked.
///
/// That is the app it is for, if it says, the achievements in its order, the stats when it lists
/// them, which are checked only then, the sets, what it sets that Steam's Web API does not
/// answer, and the achievements' translations when it is a localisation; and, once
/// [`translate`] has read them from catalogues, the sets' too.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listed {
    pub app: Option<u64>,
    pub achievements: Vec<Wanted>,
    pub stats: Option<Vec<WantedStat>>,
    pub sets: Vec<WantedSet>,
    pub unchecked: Vec<&'static str>,
    pub translations: Vec<Translated>,
    pub set_names: Vec<SetTranslated>,
}

impl Listed {
    /// Every language the translations are in, in the order the files first give them.
    #[must_use]
    pub fn languages(&self) -> Vec<&str> {
        let mut languages: Vec<&str> = Vec::new();
        for translation in self.translations.iter().flat_map(|each| &each.texts) {
            if !languages.contains(&translation.language.as_str()) {
                languages.push(&translation.language);
            }
        }
        languages
    }
}

/// Reads the file's `text`: an object with an `achievements` list, a `stats` list if the app's
/// stats are to be checked too, and a `sets` list if its achievements are grouped; or a list of
/// godot-kit's records.
///
/// In the object, each achievement has its `api_name` and `name`, and its `description` and
/// `hidden` when they are set; each stat its `api_name`, and its `name` and `default`, none and 0
/// when left out; each set its `setid` and its `name` in English. An achievement may also give
/// its `set`, whether it is `archived`, who has `permission` to unlock it, the `stat` its
/// progress bar counts with the value it unlocks `at`, and its `icon` and `icon_locked`. Other
/// keys, such as what a game's own tools keep beside them, are left alone.
///
/// In the list, the records typed `achievement`, `stat` and `achievement_set` are read the same
/// way with the API name or set ID in `steam`, and those of other types are left alone. The list
/// holds all of a game's stats, so they are always checked, but a stat's name and default only
/// when its record sets them, since a record need not.
///
/// # Errors
///
/// When the text is neither, an entry lacks its API name, name or type, a value has the wrong
/// type, an achievement counts a stat it unlocks at no value of or the other way round, or an
/// API name or set ID is there twice.
pub fn read(text: &str) -> Result<Listed, String> {
    let file: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    if let Value::Array(records) = &file {
        return read_records(records);
    }
    let app = match file.get("app") {
        None => None,
        Some(app) => Some(app.as_u64().ok_or("\"app\" is not an app ID")?),
    };
    let entries = file
        .get("achievements")
        .and_then(Value::as_array)
        .ok_or("there is no \"achievements\" list")?;
    let mut achievements = Vec::with_capacity(entries.len());
    let mut unchecked = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let place = index.saturating_add(1);
        achievements.push(achievement(
            entry,
            &format!("achievement {place}"),
            "api_name",
        )?);
        note_unchecked(entry, &ACHIEVEMENTS_UNCHECKED, &mut unchecked);
    }
    once(achievements.iter().map(|wanted| &wanted.api_name), "")?;
    let sets = match file.get("sets") {
        None => Vec::new(),
        Some(Value::Array(listed)) => {
            let mut sets = Vec::with_capacity(listed.len());
            for (index, entry) in listed.iter().enumerate() {
                let place = index.saturating_add(1);
                sets.push(set(entry, &format!("set {place}"), "setid")?);
            }
            sets
        }
        Some(_) => return Err("\"sets\" is not a list".to_owned()),
    };
    once(sets.iter().map(|wanted| &wanted.setid), "set ")?;
    let stats = match file.get("stats") {
        None => None,
        Some(Value::Array(listed)) => {
            let mut stats = Vec::with_capacity(listed.len());
            for (index, entry) in listed.iter().enumerate() {
                let place = index.saturating_add(1);
                let wanted = stat(entry, &format!("stat {place}"), "api_name")?;
                note_unchecked(entry, &STATS_UNCHECKED, &mut unchecked);
                stats.push(WantedStat {
                    name: Some(wanted.name.unwrap_or_default()),
                    default: Some(wanted.default.unwrap_or(0.0_f64)),
                    ..wanted
                });
            }
            once(stats.iter().map(|wanted| &wanted.api_name), "stat ")?;
            Some(stats)
        }
        Some(_) => return Err("\"stats\" is not a list".to_owned()),
    };
    Ok(Listed {
        app,
        achievements,
        stats,
        sets,
        unchecked,
        ..Listed::default()
    })
}

/// The keys of an achievement that set what Steam's Web API does not answer, with what that is.
const ACHIEVEMENTS_UNCHECKED: [(&[&str], &str); 4] = [
    (&["stat", "at"], "the achievements' progress"),
    (&["set"], "the achievements' sets"),
    (&["archived"], "which achievements are archived"),
    (&["permission"], "who may unlock the achievements"),
];

/// The same of a stat.
const STATS_UNCHECKED: [(&[&str], &str); 1] = [(&["kind"], "the stats' types")];

/// Adds to `unchecked` what of `keys` the `entry` sets, once each.
fn note_unchecked(
    entry: &Value,
    keys: &[(&[&str], &'static str)],
    unchecked: &mut Vec<&'static str>,
) {
    for (named, what) in keys {
        if named.iter().any(|key| entry.get(key).is_some()) && !unchecked.contains(what) {
            unchecked.push(what);
        }
    }
}

/// Reads the localisation Steamworks' achievement editor exports under Bulk Import/Export, as
/// `text`.
///
/// That is CSV with a row for each achievement's name and another for its description: the API
/// name in the first column, `name` or `description` in the second, and the text in each
/// language in a column headed with Steam's name for it, such as `german`.
///
/// # Errors
///
/// When the text is not such a file, heads a column with something that is not a language or
/// with a language twice, or gives an achievement's name or description twice, or one without
/// the other.
pub fn read_localisation(text: &str) -> Result<Listed, String> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let heads = reader.headers().map_err(|error| error.to_string())?.clone();
    let mut columns = heads.iter();
    if !columns
        .next()
        .is_some_and(|head| head.starts_with("api_name"))
        || !columns.next().is_some_and(|head| head.starts_with("field"))
    {
        return Err(
            "is not the localisation Steamworks' achievement editor exports: its first two \
             columns are not api_name and field"
                .to_owned(),
        );
    }
    let languages: Vec<String> = columns.map(str::to_owned).collect();
    if languages.is_empty() {
        return Err("has a column for no language".to_owned());
    }
    for (index, language) in languages.iter().enumerate() {
        if !presence::is_language(language) {
            return Err(presence::not_language(language));
        }
        if languages.iter().take(index).any(|other| other == language) {
            return Err(format!("has two columns for {language}"));
        }
    }
    let mut names: Vec<(String, Vec<String>)> = Vec::new();
    let mut descriptions: Vec<(String, Vec<String>)> = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|error| error.to_string())?;
        let api_name = row.get(0).unwrap_or_default().to_owned();
        if api_name.is_empty() {
            let line = row.position().map_or(0, csv::Position::line);
            return Err(format!("line {line} has no API name"));
        }
        let field = row.get(1).unwrap_or_default();
        let kept = match field {
            "name" => &mut names,
            "description" => &mut descriptions,
            _ => {
                return Err(format!(
                    "{api_name}: \"{field}\" is not name or description"
                ));
            }
        };
        if kept.iter().any(|(kept, _)| *kept == api_name) {
            return Err(format!("{api_name}'s {field} is in the file twice"));
        }
        kept.push((api_name, row.iter().skip(2).map(str::to_owned).collect()));
    }
    let mut translations = Vec::with_capacity(names.len());
    for (api_name, named) in names {
        let Some(at) = descriptions
            .iter()
            .position(|(other, _)| *other == api_name)
        else {
            return Err(format!("{api_name} has a name and no description"));
        };
        let (_, described) = descriptions.remove(at);
        let texts = languages
            .iter()
            .zip(named.into_iter().zip(described))
            .map(|(language, (name, description))| Translation {
                language: language.clone(),
                name,
                description,
            })
            .collect();
        translations.push(Translated { api_name, texts });
    }
    if let Some((api_name, _)) = descriptions.first() {
        return Err(format!("{api_name} has a description and no name"));
    }
    Ok(Listed {
        translations,
        ..Listed::default()
    })
}

/// godot-kit's `records`, as `read` takes them.
fn read_records(records: &[Value]) -> Result<Listed, String> {
    let mut achievements = Vec::new();
    let mut stats = Vec::new();
    let mut sets = Vec::new();
    let mut unchecked = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let label = format!("record {}", index.saturating_add(1));
        let Some(Value::String(kind)) = record.get("type") else {
            return Err(format!("{label} has no \"type\" text"));
        };
        match kind.as_str() {
            "achievement" => {
                achievements.push(achievement(record, &label, "steam")?);
                note_unchecked(record, &ACHIEVEMENTS_UNCHECKED, &mut unchecked);
            }
            "stat" => {
                stats.push(stat(record, &label, "steam")?);
                note_unchecked(record, &STATS_UNCHECKED, &mut unchecked);
            }
            "achievement_set" => sets.push(set(record, &label, "steam")?),
            _ => {}
        }
    }
    once(achievements.iter().map(|wanted| &wanted.api_name), "")?;
    once(stats.iter().map(|wanted| &wanted.api_name), "stat ")?;
    once(sets.iter().map(|wanted| &wanted.setid), "set ")?;
    Ok(Listed {
        achievements,
        stats: Some(stats),
        sets,
        unchecked,
        ..Listed::default()
    })
}

/// The achievement `entry`, its API name under `named_by`, called `label` in what is wrong with
/// it until that name is known.
fn achievement(entry: &Value, label: &str, named_by: &str) -> Result<Wanted, String> {
    let words = |key: &str, needed: bool| -> Result<String, String> {
        match entry.get(key) {
            Some(Value::String(value)) if !(needed && value.is_empty()) => Ok(value.clone()),
            None if !needed => Ok(String::new()),
            _ => Err(format!("{label} has no \"{key}\" text")),
        }
    };
    let api_name = words(named_by, true)?;
    let flag = |key: &str| match entry.get(key) {
        None => Ok(false),
        Some(Value::Bool(set)) => Ok(*set),
        Some(_) => Err(format!("{api_name}: \"{key}\" is not true or false")),
    };
    let text = |key: &str| match entry.get(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(format!("{api_name}: \"{key}\" is not text")),
    };
    // A record is godot-kit's, which words the permission its own way; a file's matches the CSV.
    let worded = |(editor, kit): &(&'static str, &'static str)| {
        if named_by == "steam" { *kit } else { *editor }
    };
    let permission = text("permission")?
        .map(|given| {
            PERMISSIONS
                .iter()
                .find(|each| worded(each) == given)
                .map(|(editor, _)| (*editor).to_owned())
                .ok_or_else(|| {
                    let known: Vec<&str> = PERMISSIONS.iter().map(worded).collect();
                    format!(
                        "{api_name}: \"permission\" is {given}, not {}",
                        known.join(", ")
                    )
                })
        })
        .transpose()?;
    let at = entry
        .get("at")
        .map(|at| {
            at.as_f64()
                .ok_or_else(|| format!("{api_name}: \"at\" is not a number"))
        })
        .transpose()?;
    let progress = match (text("stat")?, at) {
        (None, None) => None,
        (Some(stat), Some(at)) if at > 0.0_f64 => Some(Progress { stat, at }),
        (Some(stat), Some(at)) => {
            return Err(format!(
                "{api_name} unlocks at {at} of {stat}, where a progress bar runs from 0 to a \
                 value above it"
            ));
        }
        (Some(stat), None) => {
            return Err(format!(
                "{api_name} counts {stat} but gives no \"at\", the value of it that unlocks it"
            ));
        }
        (None, Some(at)) => {
            return Err(format!(
                "{api_name} unlocks at {at} but gives no \"stat\" for its progress bar to count"
            ));
        }
    };
    let defined = Definition {
        set: match entry.get("set") {
            None => None,
            Some(set) => {
                Some(set_id(set).ok_or_else(|| format!("{api_name}: \"set\" is not a set's ID"))?)
            }
        },
        archived: flag("archived")?,
        permission,
        progress,
        icon: text("icon")?.map(PathBuf::from),
        icon_locked: text("icon_locked")?.map(PathBuf::from),
    };
    Ok(Wanted {
        name: words("name", true)?,
        description: words("description", false)?,
        hidden: flag("hidden")?,
        id: text("id")?.unwrap_or_else(|| api_name.clone()),
        defined,
        api_name,
    })
}

/// A set's ID as `value` gives it: text, or a whole number from 1, as godot-kit keeps the one the
/// achievement editor gave the set. Valve does not say which its IDs are, so either is taken.
fn set_id(value: &Value) -> Option<String> {
    match value {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Number(id) => id.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
    }
}

/// The set `entry`, as `achievement` reads one.
fn set(entry: &Value, label: &str, named_by: &str) -> Result<WantedSet, String> {
    let setid = entry
        .get(named_by)
        .and_then(set_id)
        .ok_or_else(|| format!("{label} has no \"{named_by}\", the set's ID"))?;
    let name = match entry.get("name") {
        Some(Value::String(name)) if !name.is_empty() => name.clone(),
        _ => return Err(format!("set {setid} has no \"name\" in English")),
    };
    Ok(WantedSet {
        id: match entry.get("id") {
            Some(Value::String(id)) if !id.is_empty() => id.clone(),
            _ => setid.clone(),
        },
        setid,
        name,
    })
}

/// The stat `entry`, as `achievement` reads one.
fn stat(entry: &Value, label: &str, named_by: &str) -> Result<WantedStat, String> {
    let api_name = match entry.get(named_by) {
        Some(Value::String(api_name)) if !api_name.is_empty() => api_name.clone(),
        _ => return Err(format!("{label} has no \"{named_by}\" text")),
    };
    let name = match entry.get("name") {
        None => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => return Err(format!("stat {api_name}: \"name\" is not text")),
    };
    let default = entry
        .get("default")
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| format!("stat {api_name}: \"default\" is not a number"))
        })
        .transpose()?;
    let whole = match entry.get("kind").map(|kind| kind.as_str()) {
        None => None,
        Some(Some("int")) => Some(true),
        Some(Some("float")) => Some(false),
        Some(_) => return Err(format!("stat {api_name}: \"kind\" is not int or float")),
    };
    Ok(WantedStat {
        api_name,
        name,
        default,
        id: match entry.get("id") {
            Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
            _ => None,
        },
        whole,
    })
}

/// The `files`, each read and named, as one.
///
/// That is the app the first that says names, the achievements and stats of each in turn, the
/// stats checked when any file lists them, all that any leaves unchecked, and the translations.
///
/// # Errors
///
/// When an achievement, a stat or a set is in two of the files, or is translated in two, naming
/// both; when there are translations, but not of every achievement, or of one no file lists;
/// and when [`resolve`] refuses them.
pub fn together(files: Vec<(String, Listed)>) -> Result<Listed, String> {
    let mut whole = Listed::default();
    let mut achievements_from: BTreeMap<String, String> = BTreeMap::new();
    let mut stats_from: BTreeMap<String, String> = BTreeMap::new();
    let mut sets_from: BTreeMap<String, String> = BTreeMap::new();
    let mut translated_in: BTreeMap<String, String> = BTreeMap::new();
    for (file, listed) in files {
        for wanted in listed.sets {
            if let Some(first) = sets_from.insert(wanted.setid.clone(), file.clone()) {
                return Err(format!(
                    "set {} is in both {first} and {file}",
                    wanted.setid
                ));
            }
            whole.sets.push(wanted);
        }
        for translated in listed.translations {
            if let Some(first) = translated_in.insert(translated.api_name.clone(), file.clone()) {
                return Err(format!(
                    "{} is translated in both {first} and {file}",
                    translated.api_name
                ));
            }
            whole.translations.push(translated);
        }
        whole.app = whole.app.or(listed.app);
        for wanted in listed.achievements {
            if let Some(first) = achievements_from.insert(wanted.api_name.clone(), file.clone()) {
                return Err(format!("{} is in both {first} and {file}", wanted.api_name));
            }
            whole.achievements.push(wanted);
        }
        if let Some(stats) = listed.stats {
            let all = whole.stats.get_or_insert_with(Vec::new);
            for wanted in stats {
                if let Some(first) = stats_from.insert(wanted.api_name.clone(), file.clone()) {
                    return Err(format!(
                        "stat {} is in both {first} and {file}",
                        wanted.api_name
                    ));
                }
                all.push(wanted);
            }
        }
        for what in listed.unchecked {
            if !whole.unchecked.contains(&what) {
                whole.unchecked.push(what);
            }
        }
    }
    if let Some((api_name, file)) = translated_in
        .iter()
        .find(|(api_name, _)| !achievements_from.contains_key(*api_name))
    {
        return Err(format!(
            "{api_name} is translated in {file} but is in no file of achievements"
        ));
    }
    if !translated_in.is_empty()
        && let Some(untranslated) = whole
            .achievements
            .iter()
            .find(|wanted| !translated_in.contains_key(&wanted.api_name))
    {
        return Err(format!(
            "{} is in no localisation, which every achievement needs once one is checked",
            untranslated.api_name
        ));
    }
    resolve(&mut whole)?;
    Ok(whole)
}

/// Whether a record's `id` is what `named` names: itself, or the base mod's record when `named`
/// is bare, as godot-kit reads a reference.
fn names(id: &str, named: &str) -> bool {
    id == named || id.strip_prefix("base:") == Some(named)
}

/// Makes each achievement's set and progress stat in `whole` their IDs on Steam, refusing what
/// the achievement editor would.
///
/// # Errors
///
/// When an achievement is in a set the files do not list, counts a stat they list none of, or
/// unlocks at a fraction of a stat that counts in whole steps; and when there are more sets or
/// achievements than Steam takes.
fn resolve(whole: &mut Listed) -> Result<(), String> {
    for wanted in &mut whole.achievements {
        if let Some(progress) = &mut wanted.defined.progress
            && let Some(stats) = &whole.stats
        {
            let Some(stat) = stats.iter().find(|stat| {
                stat.api_name == progress.stat
                    || stat
                        .id
                        .as_deref()
                        .is_some_and(|id| names(id, &progress.stat))
            }) else {
                return Err(format!(
                    "{} counts {}, which no file lists as a stat",
                    wanted.api_name, progress.stat
                ));
            };
            if stat.whole == Some(true) && progress.at.fract() > 0.0_f64 {
                return Err(format!(
                    "{} unlocks at {} of {}, which counts in whole steps",
                    wanted.api_name, progress.at, stat.api_name
                ));
            }
            progress.stat.clone_from(&stat.api_name);
        }
        if let Some(set) = &mut wanted.defined.set {
            let Some(found) = whole
                .sets
                .iter()
                .find(|each| each.setid == *set || names(&each.id, set))
            else {
                return Err(format!(
                    "{} is in the set {set}, which no file lists",
                    wanted.api_name
                ));
            };
            set.clone_from(&found.setid);
        }
    }
    if whole.sets.len() >= MOST_SETS {
        return Err(format!(
            "the files list {} beside the base set, and Steam takes {MOST_SETS} with it",
            counted(whole.sets.len(), "set")
        ));
    }
    if whole.achievements.len() > MOST_ACHIEVEMENTS {
        return Err(format!(
            "the files list {} achievements, and Steam takes 5,000 at most, archived ones included",
            whole.achievements.len()
        ));
    }
    Ok(())
}

/// Puts each achievement's and set's texts in English, from the files, and in the language of
/// each of the `catalogues` into `listed`, the languages in alphabetical order; or nothing, with
/// no catalogues.
///
/// An achievement's translations are kept under the contexts `achievement.name` and
/// `achievement.description`, and a set's under `achievement_set.name`, each by its `id`, as
/// godot-kit reads a record's.
///
/// # Errors
///
/// When the achievements are translated by a localisation as well, or a catalogue lacks a text,
/// leaves it empty or marks it fuzzy, naming the catalogue and the text.
pub fn translate(listed: &mut Listed, catalogues: &[Catalogued]) -> Result<(), String> {
    if catalogues.is_empty() {
        return Ok(());
    }
    if !listed.translations.is_empty() {
        return Err(
            "the achievements are translated both in a localisation and in gettext catalogues: \
             give one or the other"
                .to_owned(),
        );
    }
    // None stands for English, which the files themselves give.
    let mut columns: Vec<Option<&Catalogued>> = catalogues.iter().map(Some).collect();
    columns.push(None);
    columns.sort_by_key(|column| column.map_or("english", |from| from.language));
    let text = |from: &Catalogued, field: &str, kind: &str, id: &str, of: &str| {
        let context = format!("{kind}.{field}");
        match from.catalogue.find(&context, id) {
            Some(entry) if entry.fuzzy => Err(format!(
                "{}: the {field} of {of} is marked fuzzy, under msgctxt \"{context}\" and msgid \
                 \"{id}\"",
                from.file
            )),
            Some(entry) if !entry.text.is_empty() => Ok(entry.text.clone()),
            _ => Err(format!(
                "{} has no {field} of {of}, under msgctxt \"{context}\" and msgid \"{id}\"",
                from.file
            )),
        }
    };
    for wanted in &listed.achievements {
        let mut texts = Vec::with_capacity(columns.len());
        for column in &columns {
            texts.push(match column {
                None => Translation {
                    language: "english".to_owned(),
                    name: wanted.name.clone(),
                    description: wanted.description.clone(),
                },
                Some(from) => Translation {
                    language: from.language.to_owned(),
                    name: text(from, "name", "achievement", &wanted.id, &wanted.api_name)?,
                    description: if wanted.description.is_empty() {
                        String::new()
                    } else {
                        text(
                            from,
                            "description",
                            "achievement",
                            &wanted.id,
                            &wanted.api_name,
                        )?
                    },
                },
            });
        }
        listed.translations.push(Translated {
            api_name: wanted.api_name.clone(),
            texts,
        });
    }
    for wanted in &listed.sets {
        let mut names = Vec::with_capacity(columns.len());
        for column in &columns {
            names.push(match column {
                None => ("english".to_owned(), wanted.name.clone()),
                Some(from) => (
                    from.language.to_owned(),
                    text(
                        from,
                        "name",
                        "achievement_set",
                        &wanted.id,
                        &format!("set {}", wanted.setid),
                    )?,
                ),
            });
        }
        listed.set_names.push(SetTranslated {
            setid: wanted.setid.clone(),
            names,
        });
    }
    Ok(())
}

/// Refuses the first of `api_names` that is there twice, said with `kind` before it.
fn once<'name>(
    api_names: impl IntoIterator<Item = &'name String>,
    kind: &str,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for api_name in api_names {
        if !seen.insert(api_name) {
            return Err(format!("{kind}{api_name} is in the file twice"));
        }
    }
    Ok(())
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
    /// A name or description in one language that is one thing on Steam and another in the
    /// localisation.
    Translation {
        api_name: String,
        language: String,
        field: &'static str,
        steam: String,
        file: String,
    },
    /// A name or description the localisation has in a language and Steam does not, which Steam
    /// answers with the English one instead.
    Untranslated {
        api_name: String,
        language: String,
        field: &'static str,
        file: String,
    },
    /// A language Steam holds no text in at all, answering each of these achievements in English.
    NoLanguage {
        language: String,
        api_names: Vec<String>,
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
            Self::Translation {
                api_name,
                language,
                field,
                steam,
                file,
            } => write!(
                formatter,
                "{api_name}: the {field} in {language} is \"{steam}\" on Steam and \"{file}\" in \
                 the localisation"
            ),
            Self::Untranslated {
                api_name,
                language,
                field,
                file,
            } => write!(
                formatter,
                "{api_name}: Steam has no {field} in {language} and shows the English one, where \
                 the localisation has \"{file}\""
            ),
            Self::NoLanguage {
                language,
                api_names,
            } => write!(
                formatter,
                "Steam holds nothing in {language}, so shows {} in English: import the \
                 localisation in Steamworks, under Stats & Achievements, and publish",
                counted(api_names.len(), "achievement")
            ),
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
        if let Some(name) = wanted.name.as_ref().filter(|name| **name != steam.name) {
            drift.push(Drift::Differs {
                api_name: label(&wanted.api_name),
                field: "the display name",
                steam: steam.name.clone(),
                file: name.clone(),
            });
        }
        // Steam and the file both write a whole number with no fraction, or the same fraction,
        // so an exact comparison is what each says.
        #[expect(
            clippy::float_cmp,
            reason = "both are the value as written, not computed"
        )]
        if let Some(default) = wanted.default.filter(|default| *default != steam.default) {
            drift.push(Drift::Differs {
                api_name: label(&wanted.api_name),
                field: "the default",
                steam: steam.default.to_string(),
                file: default.to_string(),
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

/// Every way `held`, Steam's achievements in `language`, differ from the `translations` in it.
///
/// That is each name and description, in the localisation's order. An achievement Steam does not
/// hold is left to [`compare`] to name.
///
/// Steam answers a text it has no translation of with the English one, from `english`, so one
/// that is English there and not in the localisation is said to be missing, and a language
/// Steam answers wholly in English is said once.
#[must_use]
pub fn compare_translations(
    translations: &[Translated],
    language: &str,
    held: &[Achievement],
    english: &[Achievement],
) -> Vec<Drift> {
    let in_english = |steam: &Achievement| {
        english
            .iter()
            .find(|achievement| achievement.api_name == steam.api_name)
            .map(|achievement| (&achievement.name, &achievement.description))
    };
    let mut drift = Vec::new();
    for translated in translations {
        let Some(steam) = held
            .iter()
            .find(|achievement| achievement.api_name == translated.api_name)
        else {
            continue;
        };
        let fallback = in_english(steam);
        for text in translated
            .texts
            .iter()
            .filter(|text| text.language == language)
        {
            for (field, on_steam, in_file, english_text) in [
                (
                    "name",
                    &steam.name,
                    &text.name,
                    fallback.map(|(name, _)| name),
                ),
                (
                    "description",
                    &steam.description,
                    &text.description,
                    fallback.map(|(_, description)| description),
                ),
            ] {
                if on_steam == in_file {
                    continue;
                }
                drift.push(if language != "english" && Some(on_steam) == english_text {
                    Drift::Untranslated {
                        api_name: translated.api_name.clone(),
                        language: language.to_owned(),
                        field,
                        file: in_file.clone(),
                    }
                } else {
                    Drift::Translation {
                        api_name: translated.api_name.clone(),
                        language: language.to_owned(),
                        field,
                        steam: on_steam.clone(),
                        file: in_file.clone(),
                    }
                });
            }
        }
    }
    let wholly_english = held
        .iter()
        .all(|steam| in_english(steam) == Some((&steam.name, &steam.description)));
    if wholly_english
        && drift
            .iter()
            .all(|each| matches!(each, Drift::Untranslated { .. }))
    {
        let mut api_names: Vec<String> = Vec::new();
        for each in &drift {
            if let Drift::Untranslated { api_name, .. } = each
                && !api_names.contains(api_name)
            {
                api_names.push(api_name.clone());
            }
        }
        if !api_names.is_empty() {
            return vec![Drift::NoLanguage {
                language: language.to_owned(),
                api_names,
            }];
        }
    }
    drift
}

/// How many of the `wanted` `kind` the file lists match Steam's.
///
/// When not all do, it goes on to how many differ, how many are not on Steam and how many only
/// Steam has, each counted once however many ways it differs, so the line sizes a list too long
/// to count by eye.
#[must_use]
pub fn tally(drift: &[Drift], wanted: usize, kind: &str) -> String {
    let mut differing: Vec<&str> = Vec::new();
    let mut not_on_steam = 0_usize;
    let mut only_on_steam = 0_usize;
    for difference in drift {
        match difference {
            Drift::NoneOnSteam(count) => not_on_steam = not_on_steam.saturating_add(*count),
            Drift::NotOnSteam(_) => not_on_steam = not_on_steam.saturating_add(1),
            Drift::NotInFile(_) => only_on_steam = only_on_steam.saturating_add(1),
            Drift::Differs { api_name, .. }
            | Drift::NoIcon { api_name, .. }
            | Drift::Translation { api_name, .. }
            | Drift::Untranslated { api_name, .. } => {
                if !differing.contains(&api_name.as_str()) {
                    differing.push(api_name);
                }
            }
            Drift::NoLanguage { api_names, .. } => {
                for api_name in api_names {
                    if !differing.contains(&api_name.as_str()) {
                        differing.push(api_name);
                    }
                }
            }
        }
    }
    let matched = wanted
        .saturating_sub(differing.len())
        .saturating_sub(not_on_steam);
    let verb = if differing.len() == 1 {
        "differs"
    } else {
        "differ"
    };
    let counts: Vec<String> = [
        (differing.len(), verb),
        (not_on_steam, "not on Steam"),
        (only_on_steam, "on Steam only"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, what)| format!("{count} {what}"))
    .collect();
    let line = format!("{matched} of {wanted} {kind} match");
    if counts.is_empty() {
        line
    } else {
        format!("{line}: {}", counts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::po;

    fn wanted(api_name: &str, name: &str) -> Wanted {
        Wanted {
            api_name: api_name.to_owned(),
            name: name.to_owned(),
            description: format!("{name}, described"),
            hidden: false,
            id: api_name.to_owned(),
            defined: Definition::default(),
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
                    description: "A guild's first job came home done.".to_owned(),
                    ..wanted("first_job", "Open for business")
                },
                Wanted {
                    description: String::new(),
                    hidden: true,
                    ..wanted("secret", "Hush")
                },
            ]
        );
        assert_eq!(listed.stats, None, "no list, so the stats are not checked");
        assert_eq!(listed.unchecked, Vec::<&str>::new());
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
            name: Some(name.to_owned()),
            default: Some(default),
            id: None,
            whole: None,
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
    fn godot_kits_records_are_read_by_their_steam_name_with_what_steam_cannot_say_noted() {
        // Shaped as Tall Tales Inn's mods/base/data/achievements.json.
        let listed = read(
            r#"[
                {"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"},
                {"type": "building", "id": "base:taproom"},
                {"type": "achievement", "id": "base:room_to_grow", "steam": "ROOM_TO_GROW",
                 "name": "Room to Grow", "description": "Build a second floor of guest rooms.",
                 "stat": "base:floors", "at": 2},
                {"type": "achievement", "id": "base:secret", "steam": "SECRET",
                 "name": "Hush", "hidden": true, "icon": "res://icons/secret.png"},
                {"type": "stat", "id": "base:gold", "steam": "gold", "name": "Gold earned",
                 "default": 5}
            ]"#,
        )
        .unwrap();
        assert_eq!(listed.app, None);
        assert_eq!(
            listed.achievements,
            [
                Wanted {
                    description: "Build a second floor of guest rooms.".to_owned(),
                    id: "base:room_to_grow".to_owned(),
                    defined: Definition {
                        progress: Some(Progress {
                            stat: "base:floors".to_owned(),
                            at: 2.0_f64,
                        }),
                        ..Definition::default()
                    },
                    ..wanted("ROOM_TO_GROW", "Room to Grow")
                },
                Wanted {
                    description: String::new(),
                    hidden: true,
                    id: "base:secret".to_owned(),
                    defined: Definition {
                        icon: Some(PathBuf::from("res://icons/secret.png")),
                        ..Definition::default()
                    },
                    ..wanted("SECRET", "Hush")
                },
            ]
        );
        assert_eq!(
            listed.stats,
            Some(vec![
                WantedStat {
                    api_name: "floors".to_owned(),
                    name: None,
                    default: None,
                    id: Some("base:floors".to_owned()),
                    whole: Some(true),
                },
                WantedStat {
                    api_name: "gold".to_owned(),
                    name: Some("Gold earned".to_owned()),
                    default: Some(5.0_f64),
                    id: Some("base:gold".to_owned()),
                    whole: None,
                },
            ])
        );
        assert_eq!(
            listed.unchecked,
            ["the stats' types", "the achievements' progress"]
        );

        let bare = read(r#"[{"type": "achievement", "steam": "A", "name": "A"}]"#).unwrap();
        assert_eq!(
            (bare.stats, bare.unchecked),
            (Some(Vec::new()), Vec::new()),
            "the records are all the stats there are, and none sets what Steam cannot say"
        );
    }

    #[test]
    fn records_that_are_not_ones_say_why() {
        for (text, why) in [
            (r#"[{"steam": "A"}]"#, "record 1 has no \"type\" text"),
            (
                r#"[{"type": "stat", "steam": "s"}, {"type": "achievement", "name": "A"}]"#,
                "record 2 has no \"steam\" text",
            ),
            (
                r#"[{"type": "achievement", "steam": "A"}]"#,
                "record 1 has no \"name\" text",
            ),
            (
                r#"[{"type": "stat", "steam": ""}]"#,
                "record 1 has no \"steam\" text",
            ),
            (
                r#"[{"type": "stat", "steam": "s", "default": "0"}]"#,
                "stat s: \"default\" is not a number",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A"},
                    {"type": "achievement", "steam": "A", "name": "B"}]"#,
                "A is in the file twice",
            ),
            (
                r#"[{"type": "stat", "steam": "s"}, {"type": "stat", "steam": "s"}]"#,
                "stat s is in the file twice",
            ),
        ] {
            let error = read(text).unwrap_err();
            assert!(error.starts_with(why), "{text}: {error}");
        }
    }

    #[test]
    fn several_files_are_read_as_one_and_none_may_repeat_another() {
        let file = |text: &str| read(text).unwrap();
        let feats = file(
            r#"[{"type": "achievement", "steam": "A", "name": "A", "stat": "base:s", "at": 1}]"#,
        );
        let counters = file(r#"[{"type": "stat", "id": "base:s", "steam": "s"}]"#);
        let older = file(r#"{"app": 480, "achievements": [{"api_name": "B", "name": "B"}]}"#);
        let whole = together(vec![
            ("feats.json".to_owned(), feats.clone()),
            ("counters.json".to_owned(), counters.clone()),
            ("older.json".to_owned(), older),
        ])
        .unwrap();
        assert_eq!(whole.app, Some(480));
        assert_eq!(
            whole.achievements.first().unwrap().defined.progress,
            Some(Progress {
                stat: "s".to_owned(),
                at: 1.0_f64
            }),
            "the stat a record counts is named by its API name once found"
        );
        assert_eq!(
            whole
                .achievements
                .iter()
                .map(|wanted| wanted.api_name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(
            whole.stats.unwrap().len(),
            1,
            "the stats are checked, since a file lists them"
        );
        assert_eq!(whole.unchecked, ["the achievements' progress"]);
        let no_stats = file(r#"{"achievements": []}"#);
        assert_eq!(
            together(vec![("a.json".to_owned(), no_stats)])
                .unwrap()
                .stats,
            None
        );

        assert_eq!(
            together(vec![
                ("feats.json".to_owned(), feats.clone()),
                ("more.json".to_owned(), feats),
            ])
            .unwrap_err(),
            "A is in both feats.json and more.json"
        );
        assert_eq!(
            together(vec![
                ("counters.json".to_owned(), counters.clone()),
                ("again.json".to_owned(), counters),
            ])
            .unwrap_err(),
            "stat s is in both counters.json and again.json"
        );
    }

    /// Tall Tales Inn's records, as `read` takes them, with the cellar's achievements in a set.
    const INN: &str = r#"[
        {"type": "stat", "id": "base:floors", "steam": "floors", "kind": "int"},
        {"type": "achievement_set", "id": "base:cellar", "steam": 1, "name": "The Cellar"},
        {"type": "achievement", "id": "base:room_to_grow", "steam": "ROOM_TO_GROW",
         "name": "Room to Grow", "description": "Build a second floor of guest rooms.",
         "stat": "floors", "at": 2, "icon": "icons/room_to_grow.png"},
        {"type": "achievement", "id": "base:first_cellar", "steam": "FIRST_CELLAR",
         "name": "Down in the Cellar", "description": "Dig an inn's first cellar.",
         "set": "cellar", "archived": true, "permission": "game_server", "hidden": true}
    ]"#;

    #[test]
    fn sets_archiving_and_permissions_are_read_and_each_achievement_names_its_set_by_its_id() {
        let whole = together(vec![("inn.json".to_owned(), read(INN).unwrap())]).unwrap();
        assert_eq!(
            whole.sets,
            [WantedSet {
                setid: "1".to_owned(),
                name: "The Cellar".to_owned(),
                id: "base:cellar".to_owned(),
            }]
        );
        let cellar = whole.achievements.last().unwrap();
        assert_eq!(
            cellar.defined,
            Definition {
                set: Some("1".to_owned()),
                archived: true,
                permission: Some("GameServer".to_owned()),
                ..Definition::default()
            }
        );
        assert_eq!(
            whole.unchecked,
            [
                "the stats' types",
                "the achievements' progress",
                "the achievements' sets",
                "which achievements are archived",
                "who may unlock the achievements",
            ]
        );
        let object = read(
            r#"{"achievements": [
                    {"api_name": "A", "name": "A", "set": "7", "permission": "OfficialGameServer"},
                    {"api_name": "B", "name": "B", "set": 8}],
                "sets": [{"setid": "7", "name": "Seven"}, {"setid": 8, "name": "Eight"}]}"#,
        )
        .unwrap();
        let grouped = together(vec![("a.json".to_owned(), object)]).unwrap();
        let defined: Vec<(Option<&str>, Option<&str>)> = grouped
            .achievements
            .iter()
            .map(|wanted| {
                (
                    wanted.defined.set.as_deref(),
                    wanted.defined.permission.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            defined,
            [(Some("7"), Some("OfficialGameServer")), (Some("8"), None)]
        );
        let ids: Vec<&str> = grouped.sets.iter().map(|set| set.id.as_str()).collect();
        assert_eq!(ids, ["7", "8"]);
    }

    #[test]
    fn what_the_achievement_editor_would_refuse_in_one_file_is_refused_as_it_is_read() {
        for (text, why) in [
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "stat": "s"}]"#,
                "A counts s but gives no \"at\", the value of it that unlocks it",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "at": 3}]"#,
                "A unlocks at 3 but gives no \"stat\" for its progress bar to count",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "stat": "s", "at": 0}]"#,
                "A unlocks at 0 of s, where a progress bar runs from 0 to a value above it",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "stat": "s", "at": "3"}]"#,
                "A: \"at\" is not a number",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "permission": "GameServer"}]"#,
                "A: \"permission\" is GameServer, not client, game_server, official_game_server",
            ),
            (
                r#"{"achievements": [{"api_name": "A", "name": "A", "permission": "client"}]}"#,
                "A: \"permission\" is client, not Client, GameServer, OfficialGameServer",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "archived": "no"}]"#,
                "A: \"archived\" is not true or false",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "set": 0}]"#,
                "A: \"set\" is not a set's ID",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "set": ""}]"#,
                "A: \"set\" is not a set's ID",
            ),
            (
                r#"[{"type": "achievement_set", "steam": 1}]"#,
                "set 1 has no \"name\" in English",
            ),
            (
                r#"[{"type": "achievement_set", "name": "One"}]"#,
                "record 1 has no \"steam\", the set's ID",
            ),
            (
                r#"[{"type": "achievement_set", "steam": 1.5, "name": "One"}]"#,
                "record 1 has no \"steam\", the set's ID",
            ),
            (
                r#"[{"type": "stat", "steam": "s", "kind": "big"}]"#,
                "stat s: \"kind\" is not int or float",
            ),
            (
                r#"[{"type": "achievement_set", "steam": 1, "name": "One"},
                    {"type": "achievement_set", "steam": "1", "name": "Two"}]"#,
                "set 1 is in the file twice",
            ),
            (
                r#"{"achievements": [], "sets": {}}"#,
                "\"sets\" is not a list",
            ),
        ] {
            assert_eq!(read(text).unwrap_err(), why, "{text}");
        }
    }

    #[test]
    fn what_the_achievement_editor_would_refuse_across_files_is_refused_once_they_are_read() {
        for (text, why) in [
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "stat": "s", "at": 1}]"#,
                "A counts s, which no file lists as a stat",
            ),
            (
                r#"[{"type": "stat", "id": "base:s", "steam": "s", "kind": "int"},
                    {"type": "achievement", "steam": "A", "name": "A", "stat": "s", "at": 1.5}]"#,
                "A unlocks at 1.5 of s, which counts in whole steps",
            ),
            (
                r#"[{"type": "achievement", "steam": "A", "name": "A", "set": "cellar"}]"#,
                "A is in the set cellar, which no file lists",
            ),
        ] {
            assert_eq!(
                together(vec![("a.json".to_owned(), read(text).unwrap())]).unwrap_err(),
                why,
                "{text}"
            );
        }
        let floats = r#"[{"type": "stat", "id": "base:s", "steam": "s", "kind": "float"},
            {"type": "achievement", "steam": "A", "name": "A", "stat": "base:s", "at": 1.5}]"#;
        let progress = |text: &str| {
            together(vec![("a.json".to_owned(), read(text).unwrap())])
                .unwrap()
                .achievements
                .first()
                .unwrap()
                .defined
                .progress
                .clone()
                .unwrap()
        };
        assert_eq!(
            progress(floats),
            Progress {
                stat: "s".to_owned(),
                at: 1.5_f64
            }
        );
        let unlisted =
            r#"{"achievements": [{"api_name": "A", "name": "A", "stat": "s", "at": 1}]}"#;
        assert_eq!(
            progress(unlisted).stat,
            "s",
            "a file that lists no stats leaves the stat to Steamworks"
        );
        let sets = |count: usize| {
            let listed = Listed {
                sets: (0..count)
                    .map(|each| WantedSet {
                        setid: each.to_string(),
                        name: each.to_string(),
                        id: each.to_string(),
                    })
                    .collect(),
                ..Listed::default()
            };
            together(vec![("sets.json".to_owned(), listed)])
        };
        assert_eq!(sets(39).unwrap().sets.len(), 39);
        assert_eq!(
            sets(40).unwrap_err(),
            "the files list 40 sets beside the base set, and Steam takes 40 with it"
        );
        let achievements = |count: usize| {
            let listed = Listed {
                achievements: (0..count)
                    .map(|each| wanted(&each.to_string(), "A"))
                    .collect(),
                ..Listed::default()
            };
            together(vec![("many.json".to_owned(), listed)])
        };
        assert_eq!(achievements(5_000).unwrap().achievements.len(), 5_000);
        assert_eq!(
            achievements(5_001).unwrap_err(),
            "the files list 5001 achievements, and Steam takes 5,000 at most, archived ones \
             included"
        );
    }

    fn catalogued(file: &str, language: &'static str, text: &str) -> Catalogued {
        Catalogued {
            file: file.to_owned(),
            language,
            catalogue: po::read(text).unwrap(),
        }
    }

    const GERMAN: &str = "msgctxt \"achievement.name\"\nmsgid \"base:room_to_grow\"\n\
        msgstr \"Platz zum Wachsen\"\n\
        msgctxt \"achievement.description\"\nmsgid \"base:room_to_grow\"\n\
        msgstr \"Baue einen zweiten Stock mit G\u{e4}stezimmern.\"\n\
        msgctxt \"achievement.name\"\nmsgid \"base:first_cellar\"\nmsgstr \"Unten im Keller\"\n\
        msgctxt \"achievement.description\"\nmsgid \"base:first_cellar\"\n\
        msgstr \"Grabe den ersten Keller einer Herberge.\"\n\
        msgctxt \"achievement_set.name\"\nmsgid \"base:cellar\"\nmsgstr \"Der Keller\"\n";

    #[test]
    fn catalogues_translate_each_achievement_and_set_by_its_id_in_alphabetical_order() {
        let mut whole = together(vec![("inn.json".to_owned(), read(INN).unwrap())]).unwrap();
        let french = GERMAN
            .replace("Platz zum Wachsen", "De la place pour grandir")
            .replace("Der Keller", "La cave");
        translate(
            &mut whole,
            &[
                catalogued("de.po", "german", GERMAN),
                catalogued("fr.po", "french", &french),
            ],
        )
        .unwrap();
        assert_eq!(whole.languages(), ["english", "french", "german"]);
        let room = whole.translations.first().unwrap();
        assert_eq!(room.api_name, "ROOM_TO_GROW");
        assert_eq!(
            room.texts.last().unwrap(),
            &Translation {
                language: "german".to_owned(),
                name: "Platz zum Wachsen".to_owned(),
                description: "Baue einen zweiten Stock mit G\u{e4}stezimmern.".to_owned(),
            }
        );
        assert_eq!(
            room.texts
                .iter()
                .map(|text| text.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Room to Grow",
                "De la place pour grandir",
                "Platz zum Wachsen"
            ]
        );
        assert_eq!(
            whole.set_names,
            [SetTranslated {
                setid: "1".to_owned(),
                names: vec![
                    ("english".to_owned(), "The Cellar".to_owned()),
                    ("french".to_owned(), "La cave".to_owned()),
                    ("german".to_owned(), "Der Keller".to_owned()),
                ],
            }]
        );

        let mut untouched = together(vec![("inn.json".to_owned(), read(INN).unwrap())]).unwrap();
        translate(&mut untouched, &[]).unwrap();
        assert_eq!(untouched.translations, []);
    }

    #[test]
    fn a_text_a_catalogue_lacks_or_leaves_to_confirm_is_named_with_where_it_goes() {
        let whole = together(vec![("inn.json".to_owned(), read(INN).unwrap())]).unwrap();
        for (text, why) in [
            (
                GERMAN.replace("\"Unten im Keller\"", "\"\""),
                "de.po has no name of FIRST_CELLAR, under msgctxt \"achievement.name\" and msgid \
                 \"base:first_cellar\"",
            ),
            (
                GERMAN.replace(
                    "msgctxt \"achievement.description\"\nmsgid \"base:room_to_grow\"",
                    "#, fuzzy\nmsgctxt \"achievement.description\"\nmsgid \"base:room_to_grow\"",
                ),
                "de.po: the description of ROOM_TO_GROW is marked fuzzy, under msgctxt \
                 \"achievement.description\" and msgid \"base:room_to_grow\"",
            ),
            (
                GERMAN.replace("achievement_set.name", "achievement_set.title"),
                "de.po has no name of set 1, under msgctxt \"achievement_set.name\" and msgid \
                 \"base:cellar\"",
            ),
        ] {
            let mut listed = whole.clone();
            assert_eq!(
                translate(&mut listed, &[catalogued("de.po", "german", &text)]).unwrap_err(),
                why
            );
        }
        let mut both = whole;
        both.translations.push(Translated {
            api_name: "ROOM_TO_GROW".to_owned(),
            texts: Vec::new(),
        });
        assert_eq!(
            translate(&mut both, &[catalogued("de.po", "german", GERMAN)]).unwrap_err(),
            "the achievements are translated both in a localisation and in gettext catalogues: \
             give one or the other"
        );
    }

    #[test]
    fn an_achievement_with_no_description_needs_none_translated() {
        let mut whole =
            together(vec![(
            "a.json".to_owned(),
            read(r#"[{"type": "achievement", "id": "base:hush", "steam": "HUSH", "name": "Hush"}]"#)
                .unwrap(),
        )])
            .unwrap();
        let german = "msgctxt \"achievement.name\"\nmsgid \"base:hush\"\nmsgstr \"Pst\"\n";
        translate(&mut whole, &[catalogued("de.po", "german", german)]).unwrap();
        assert_eq!(
            whole.translations.first().unwrap().texts.last().unwrap(),
            &Translation {
                language: "german".to_owned(),
                name: "Pst".to_owned(),
                description: String::new(),
            }
        );
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
        assert_eq!(
            tally(&drift, 3, "achievements"),
            "0 of 3 achievements match: 2 differ, 1 not on Steam, 1 on Steam only",
            "each counted once however many ways it differs"
        );
    }

    #[test]
    fn a_tally_says_how_many_of_the_files_match_and_why_the_rest_do_not() {
        let differs = |api_name: &str| Drift::NoIcon {
            api_name: api_name.to_owned(),
            which: "icon",
        };
        assert_eq!(
            tally(&[], 15, "achievements"),
            "15 of 15 achievements match"
        );
        assert_eq!(
            tally(&[differs("a")], 3, "achievements"),
            "2 of 3 achievements match: 1 differs"
        );
        assert_eq!(
            tally(
                &[Drift::NotOnSteam("stat years_open".to_owned())],
                1,
                "stats"
            ),
            "0 of 1 stats match: 1 not on Steam"
        );
        assert_eq!(
            tally(&[Drift::NotInFile("extra".to_owned())], 2, "stats"),
            "2 of 2 stats match: 1 on Steam only"
        );
        assert_eq!(
            tally(&[Drift::NoneOnSteam(15)], 15, "achievements"),
            "0 of 15 achievements match: 15 not on Steam"
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
            name: Some(name.to_owned()),
            default: Some(default),
            id: None,
            whole: None,
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
        let unset = WantedStat {
            api_name: "NumGames".to_owned(),
            name: None,
            default: None,
            id: None,
            whole: None,
        };
        assert_eq!(
            compare_stats(&[unset], &[held("NumGames", "Games", 1.5_f64)]),
            [],
            "a name and default the file does not set are Steamworks' own"
        );
    }

    /// The column heads Steamworks' achievement editor exports its localisation under.
    const HEADS: &str = "api_name (required; must match an api_name from the definitions CSV),\
                         field (name or description)";

    fn localisation(languages: &str, rows: &str) -> String {
        format!("{HEADS},{languages}\n{rows}")
    }

    fn translated(api_name: &str, texts: &[(&str, &str, &str)]) -> Translated {
        Translated {
            api_name: api_name.to_owned(),
            texts: texts
                .iter()
                .map(|(language, name, description)| Translation {
                    language: (*language).to_owned(),
                    name: (*name).to_owned(),
                    description: (*description).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_localisation_is_read_as_each_achievements_name_and_description_in_each_language() {
        let text = localisation(
            "english,german",
            "ROOM_TO_GROW,name,Room to Grow,Platz zum Wachsen\n\
             ROOM_TO_GROW,description,\"Build a second floor, of guest rooms.\",\"Sag \"\"Hallo\"\"\"\n\
             FIVE_FLOORS,description,Up.,Hoch.\n\
             FIVE_FLOORS,name,Five Floors Up,\n",
        );
        let listed = read_localisation(&text).unwrap();
        assert_eq!(
            listed.translations,
            [
                translated(
                    "ROOM_TO_GROW",
                    &[
                        (
                            "english",
                            "Room to Grow",
                            "Build a second floor, of guest rooms."
                        ),
                        ("german", "Platz zum Wachsen", "Sag \"Hallo\""),
                    ]
                ),
                translated(
                    "FIVE_FLOORS",
                    &[
                        ("english", "Five Floors Up", "Up."),
                        ("german", "", "Hoch.")
                    ]
                ),
            ]
        );
        assert_eq!(listed.languages(), ["english", "german"]);
        assert!(listed.achievements.is_empty() && listed.stats.is_none());
        assert_eq!(
            read_localisation(&format!("\u{feff}{text}")).unwrap(),
            listed,
            "a byte order mark before the heads is not part of them"
        );
    }

    #[test]
    fn a_localisation_that_is_not_one_says_why() {
        for (text, why) in [
            (
                "api_name,name,german\n".to_owned(),
                "is not the localisation Steamworks' achievement editor exports: its first two \
                 columns are not api_name and field",
            ),
            (
                "api_name\n".to_owned(),
                "is not the localisation Steamworks' achievement editor exports: its first two \
                 columns are not api_name and field",
            ),
            (
                "field,api_name,german\n".to_owned(),
                "is not the localisation Steamworks' achievement editor exports: its first two \
                 columns are not api_name and field",
            ),
            (format!("{HEADS}\n"), "has a column for no language"),
            (
                localisation("German", ""),
                "\"German\" is not a language as Steam names them, such as english or schinese",
            ),
            (
                localisation("german,english,german", ""),
                "has two columns for german",
            ),
            (
                localisation("german", ",name,Hallo\n"),
                "line 2 has no API name",
            ),
            (
                localisation("german", "A,title,Hallo\n"),
                "A: \"title\" is not name or description",
            ),
            (
                localisation("german", "A,name,Hallo\nA,name,Hallo\n"),
                "A's name is in the file twice",
            ),
            (
                localisation("german", "A,name,Hallo\n"),
                "A has a name and no description",
            ),
            (
                localisation("german", "A,description,Hallo\n"),
                "A has a description and no name",
            ),
        ] {
            assert_eq!(read_localisation(&text), Err(why.to_owned()), "{text}");
        }
        let ragged = read_localisation(&localisation("german", "A,name\n")).unwrap_err();
        assert!(ragged.contains("found record with 2 fields"), "{ragged}");
    }

    #[test]
    fn a_localisation_checks_every_achievement_the_files_list_and_no_other() {
        let records = read(r#"{"achievements": [{"api_name": "A", "name": "A"}]}"#).unwrap();
        let both = read(
            r#"{"achievements": [{"api_name": "A", "name": "A"}, {"api_name": "B", "name": "B"}]}"#,
        )
        .unwrap();
        let of = |api_names: &[&str]| {
            let rows: Vec<String> = api_names
                .iter()
                .map(|api_name| format!("{api_name},name,N\n{api_name},description,D\n"))
                .collect();
            read_localisation(&localisation("german", &rows.concat())).unwrap()
        };
        let whole = together(vec![
            ("achievements.json".to_owned(), records.clone()),
            ("localisation.csv".to_owned(), of(&["A"])),
        ])
        .unwrap();
        assert_eq!(
            whole.translations,
            [translated("A", &[("german", "N", "D")])]
        );
        assert_eq!(whole.achievements.len(), 1);
        for (files, why) in [
            (
                vec![
                    ("achievements.json", both.clone()),
                    ("one.csv", of(&["A"])),
                    ("two.csv", of(&["B", "A"])),
                ],
                "A is translated in both one.csv and two.csv",
            ),
            (
                vec![
                    ("achievements.json", records),
                    ("localisation.csv", of(&["A", "B"])),
                ],
                "B is translated in localisation.csv but is in no file of achievements",
            ),
            (
                vec![("localisation.csv", of(&["A"]))],
                "A is translated in localisation.csv but is in no file of achievements",
            ),
            (
                vec![
                    ("achievements.json", both),
                    ("localisation.csv", of(&["A"])),
                ],
                "B is in no localisation, which every achievement needs once one is checked",
            ),
        ] {
            let files = files
                .into_iter()
                .map(|(file, listed)| (file.to_owned(), listed))
                .collect();
            assert_eq!(together(files), Err(why.to_owned()));
        }
    }

    #[test]
    fn each_translation_steam_holds_otherwise_is_named_with_its_language() {
        let english = [held("A", "Winner"), held("B", "Orbiter")];
        let translations = [
            translated(
                "A",
                &[
                    ("english", "Winner", "Winner, described"),
                    ("german", "Sieger", "Gewinne."),
                ],
            ),
            translated(
                "B",
                &[
                    ("english", "Orbit", "Orbiter, described"),
                    ("german", "Umkreiser", "Fliege weit."),
                ],
            ),
            translated("Gone", &[("german", "Fort", "Weg.")]),
        ];
        let said =
            |drift: Vec<Drift>| -> Vec<String> { drift.iter().map(ToString::to_string).collect() };
        assert_eq!(
            said(compare_translations(
                &translations,
                "english",
                &english,
                &english
            )),
            ["B: the name in english is \"Orbiter\" on Steam and \"Orbit\" in the localisation"],
            "English is Steam's own text, never one missing"
        );
        let german = [
            Achievement {
                description: "Gewinne.".to_owned(),
                ..held("A", "Sieger")
            },
            Achievement {
                description: "Fliege nah.".to_owned(),
                ..held("B", "Orbiter")
            },
        ];
        assert_eq!(
            said(compare_translations(
                &translations,
                "german",
                &german,
                &english
            )),
            [
                "B: Steam has no name in german and shows the English one, where the \
                 localisation has \"Umkreiser\"",
                "B: the description in german is \"Fliege nah.\" on Steam and \"Fliege weit.\" in \
                 the localisation",
            ]
        );
        assert_eq!(
            said(compare_translations(
                &translations,
                "german",
                &english,
                &english
            )),
            [
                "Steam holds nothing in german, so shows 2 achievements in English: import the \
                 localisation in Steamworks, under Stats & Achievements, and publish"
            ]
        );
        let as_english = [translated(
            "A",
            &[("german", "Winner", "Winner, described")],
        )];
        assert_eq!(
            compare_translations(&as_english, "german", &english, &english),
            [],
            "a translation that is the English text matches Steam's falling back to it"
        );
        let half = [held("A", "Winner")];
        assert_eq!(
            compare_translations(&translations[..1], "german", &half, &english),
            [Drift::NoLanguage {
                language: "german".to_owned(),
                api_names: vec!["A".to_owned()],
            }],
            "an achievement is counted once however many of its texts are missing"
        );
        assert_eq!(
            compare_translations(&translations, "french", &english, &english),
            [],
            "a language the localisation does not have is not checked"
        );
    }

    #[test]
    fn a_language_steam_holds_in_part_is_named_text_by_text() {
        let english = [held("A", "Winner"), held("B", "Orbiter")];
        let translations = [
            translated("A", &[("german", "Sieger", "Gewinne.")]),
            translated("B", &[("german", "Umkreiser", "Orbiter, described")]),
        ];
        let partly = [
            Achievement {
                description: "Gewinne.".to_owned(),
                ..held("A", "Sieger")
            },
            held("B", "Orbiter"),
        ];
        assert_eq!(
            compare_translations(&translations, "german", &partly, &english),
            [Drift::Untranslated {
                api_name: "B".to_owned(),
                language: "german".to_owned(),
                field: "name",
                file: "Umkreiser".to_owned(),
            }]
        );
    }

    #[test]
    fn a_tally_counts_an_achievement_once_whatever_its_languages_lack() {
        let missing = |api_name: &str, language: &str| Drift::Untranslated {
            api_name: api_name.to_owned(),
            language: language.to_owned(),
            field: "name",
            file: "N".to_owned(),
        };
        let drift = [
            missing("A", "german"),
            missing("A", "french"),
            Drift::Translation {
                api_name: "B".to_owned(),
                language: "german".to_owned(),
                field: "name",
                steam: "S".to_owned(),
                file: "N".to_owned(),
            },
            Drift::NoLanguage {
                language: "polish".to_owned(),
                api_names: vec!["A".to_owned(), "C".to_owned()],
            },
        ];
        assert_eq!(
            tally(&drift, 4, "achievements"),
            "1 of 4 achievements match: 3 differ"
        );
    }
}
