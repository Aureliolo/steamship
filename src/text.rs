//! What Steam shows of a game in each language it speaks, from its English and its translations.
//!
//! That is the upload the store page's Localization tab takes, the content survey's statement of
//! how AI was used, the Workshop page's title and description, and rich presence. The English is
//! written by hand beside the build scripts, and each catalogue translates it under a context of
//! Steam's text: `store page`, `steam workshop` or `rich presence`, with the English as the
//! `msgid`.

use serde_json::{Map, Value as Json};

use crate::achievements::Catalogued;
use crate::presence;
use crate::vdf::{self, Block, Pair, Value};

/// The store page in English: its item ID, its text and the statement of how AI was used.
pub const STORE: &str = "store_english.json";
/// The Workshop page's title and description in English.
pub const WORKSHOP: &str = "workshop_english.json";
/// Rich presence in English, from which `rich_presence_english.vdf` is written too.
pub const PRESENCE: &str = "presence_english.json";
/// Rich presence in English as Valve's own file, written by hand when there is no [`PRESENCE`].
pub const PRESENCE_ENGLISH: &str = "rich_presence_english.vdf";
/// The statement of how AI was used, in every language.
pub const DISCLOSURE: &str = "ai_disclosure.json";
/// Each language's recommended system requirements, by their names on the store page's Basic Info
/// form. The Localization tab's upload takes the minimum ones and drops these without a word.
pub const BASIC_INFO: &str = "basic_info.json";
/// Each language's alt text for the screenshots, by their places in the store's order.
///
/// The Localization tab's upload and export carry no screenshot's alt text, which only the Store
/// Page tab's panel for it takes, one language at a time.
pub const SCREENSHOTS: &str = "screenshots.json";
/// The folders the store page's and the Workshop page's text are written into.
pub const STORE_FOLDER: &str = "store";
pub const WORKSHOP_FOLDER: &str = "workshop";

const STORE_CONTEXT: &str = "store page";
const REQUIREMENT: &str = "app[content][sysreqs][";
const ALT_TEXT: &str = "app[assets][extra][";
const RECOMMENDED: &str = "rec";
const WORKSHOP_CONTEXT: &str = "steam workshop";
const PRESENCE_CONTEXT: &str = "rich presence";

/// The English of everything this writes, each part present when its file is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct English {
    pub store: Option<Store>,
    pub workshop: Option<Workshop>,
    pub presence: Option<Presence>,
}

/// The store page, as [`STORE`] gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    /// The store page's item ID, which its Localization tab's export names and which is not the
    /// app's; empty until it is known, when nothing is written for the upload.
    pub item: String,
    /// The statement of how AI was used in making the game, empty for none.
    pub disclosure: String,
    pub blocks: Vec<Part>,
    /// Each screenshot's alt text, in the store's order of them.
    pub screenshots: Vec<String>,
}

/// One block of the store page's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Short(String),
    Heading(String),
    Paragraph(String),
    Item(String),
    /// A picture or a clip, by its name among the Description tab's custom images, which is the
    /// same in every language, and what a screen reader says of it, if anything.
    Image {
        name: String,
        alt: Option<String>,
    },
    /// A field Steam shows in the player's language outside the description, such as the legal
    /// line or a system requirement, by its name on the form.
    Field {
        name: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workshop {
    pub title: String,
    pub description: String,
}

/// A system requirement in one language, as a field of [`STORE`] names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub language: String,
    /// As the form names it: `windows`, `mac`, `linux` or `android`.
    pub platform: String,
    /// `min` or `rec`.
    pub tier: String,
    /// The rest of the field's name, such as `osversion`, or `memory][amount` for a part of one.
    pub field: String,
    pub text: String,
}

/// Rich presence in English.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub tokens: Vec<Token>,
    /// Whether the English file is written from [`PRESENCE`], rather than kept by hand.
    pub writes_english: bool,
}

/// A rich presence token, its English line, and what a catalogue translates it under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub name: String,
    /// As Steam takes it, its fields written `%field%`.
    pub english: String,
    pub context: String,
    pub id: String,
}

/// Reads [`STORE`].
///
/// # Errors
///
/// When it is not an object, holds what a store page has not, its item ID is neither a number nor
/// a text, or a block is of no kind Steam marks up, lacks what its kind needs or holds more.
pub fn store(text: &str) -> Result<Store, String> {
    let page = object(text, "an object holding \"itemid\" and \"blocks\"")?;
    if let Some(other) = page.keys().find(|key| {
        !matches!(
            key.as_str(),
            "itemid" | "disclosure" | "screenshots" | "blocks"
        )
    }) {
        return Err(format!(
            "\"{other}\" is none of itemid, disclosure, screenshots and blocks"
        ));
    }
    let item = match page.get("itemid") {
        None | Some(Json::Null) => String::new(),
        Some(Json::String(item)) => item.clone(),
        Some(Json::Number(item)) => item.to_string(),
        Some(_) => return Err("\"itemid\" is neither a number nor a text".to_owned()),
    };
    let disclosure = match page.get("disclosure") {
        None | Some(Json::Null) => String::new(),
        Some(Json::String(disclosure)) => disclosure.clone(),
        Some(_) => return Err("\"disclosure\" is not a text".to_owned()),
    };
    let listed = match page.get("blocks") {
        None => &Vec::new(),
        Some(Json::Array(blocks)) => blocks,
        Some(_) => return Err("\"blocks\" is not a list".to_owned()),
    };
    let mut blocks = Vec::with_capacity(listed.len());
    for (index, given) in listed.iter().enumerate() {
        blocks.push(block(given, index.saturating_add(1))?);
    }
    shown_once(&blocks)?;
    Ok(Store {
        item,
        disclosure,
        blocks,
        screenshots: screenshots(page.get("screenshots"))?,
    })
}

/// The block numbered `number` of [`STORE`]'s `blocks`.
fn block(given: &Json, number: usize) -> Result<Part, String> {
    let field = |name: &str| {
        given
            .get(name)
            .and_then(Json::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("block {number} has no \"{name}\" text"))
    };
    let (part, takes): (Part, &[&str]) = match given.get("kind").and_then(Json::as_str) {
        Some("short") => (Part::Short(field("text")?), &["text"]),
        Some("heading") => (Part::Heading(field("text")?), &["text"]),
        Some("paragraph") => (Part::Paragraph(field("text")?), &["text"]),
        Some("item") => (Part::Item(field("text")?), &["text"]),
        Some("image") => {
            let alt = match given.get("alt") {
                None | Some(Json::Null) => None,
                Some(Json::String(alt)) if alt.is_empty() => {
                    return Err(format!(
                        "block {number}'s \"alt\" is empty: leave it out of an image with none"
                    ));
                }
                Some(Json::String(alt)) => Some(alt.clone()),
                Some(_) => return Err(format!("block {number}'s \"alt\" is not a text")),
            };
            (
                Part::Image {
                    name: field("image")?,
                    alt,
                },
                &["image", "alt"],
            )
        }
        Some("field") => (
            Part::Field {
                name: field("field")?,
                text: field("text")?,
            },
            &["field", "text"],
        ),
        Some(kind) => {
            return Err(format!(
                "block {number} is a \"{kind}\", which is none of short, heading, paragraph, \
                 item, image and field"
            ));
        }
        None => return Err(format!("block {number} has no \"kind\"")),
    };
    if let Some(other) = given.as_object().and_then(|holds| {
        holds
            .keys()
            .find(|key| *key != "kind" && !takes.contains(&key.as_str()))
    }) {
        return Err(format!(
            "block {number} holds \"{other}\", which a block of kind {} does not take: it takes {}",
            given.get("kind").and_then(Json::as_str).unwrap_or_default(),
            takes.join(" and ")
        ));
    }
    Ok(part)
}

/// Refuses an About that shows an image a second time while any of its images has alt text,
/// since Steam numbers the images whose alt text it keeps only as seen with each shown once.
fn shown_once(blocks: &[Part]) -> Result<(), String> {
    if !blocks
        .iter()
        .any(|part| matches!(part, Part::Image { alt: Some(_), .. }))
    {
        return Ok(());
    }
    let mut shown: Vec<&str> = Vec::new();
    for (index, part) in blocks.iter().enumerate() {
        if let Part::Image { name, .. } = part {
            if shown.contains(&name.as_str()) {
                return Err(format!(
                    "block {} shows {name} a second time, and alt text is written only for an \
                     About that shows each image once",
                    index.saturating_add(1)
                ));
            }
            shown.push(name);
        }
    }
    Ok(())
}

/// The screenshots' alt text, as [`STORE`]'s `screenshots` lists it.
fn screenshots(given: Option<&Json>) -> Result<Vec<String>, String> {
    let listed = match given {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Array(listed)) => listed,
        Some(_) => {
            return Err("\"screenshots\" is not a list of each screenshot's alt text".to_owned());
        }
    };
    let mut screenshots = Vec::with_capacity(listed.len());
    for (index, alt) in listed.iter().enumerate() {
        let number = index.saturating_add(1);
        match alt.as_str() {
            Some("") => return Err(format!("screenshot {number}'s alt text is empty")),
            Some(alt) => screenshots.push(alt.to_owned()),
            None => return Err(format!("screenshot {number}'s alt text is not a text")),
        }
    }
    Ok(screenshots)
}

/// Reads [`WORKSHOP`].
///
/// # Errors
///
/// When it is not an object holding a `title` and a `description` and nothing else.
pub fn workshop(text: &str) -> Result<Workshop, String> {
    let page = object(text, "an object holding \"title\" and \"description\"")?;
    if let Some(other) = page
        .keys()
        .find(|key| !matches!(key.as_str(), "title" | "description"))
    {
        return Err(format!(
            "\"{other}\" is no field of the Workshop page, which has a title and a description"
        ));
    }
    let field = |name: &str| {
        page.get(name)
            .and_then(Json::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("there is no \"{name}\" text"))
    };
    Ok(Workshop {
        title: field("title")?,
        description: field("description")?,
    })
}

/// Reads [`PRESENCE`], each token with its English line.
///
/// Under a name ending in `_`, such as `#Bishop_`, `{"records": "bishop"}` stands for a token for
/// each of the game's `records` of that type that has a name: the name followed by the record's ID
/// without its mod, its line the record's name, translated under `bishop.name` and the record's ID
/// as games-kit's are.
///
/// # Errors
///
/// When it is not such an object, a token does not start with `#`, or a record's token is also
/// given as a line.
pub fn presence(text: &str, records: &[Json]) -> Result<Presence, String> {
    let lines = object(text, "an object of tokens and their English")?;
    let mut tokens = Vec::with_capacity(lines.len());
    let mut made = Vec::new();
    for (name, line) in &lines {
        if !name.starts_with('#') {
            return Err(format!("{name} does not start with #"));
        }
        match line {
            Json::String(english) => tokens.push(Token {
                name: name.clone(),
                english: english.clone(),
                context: PRESENCE_CONTEXT.to_owned(),
                id: braced(english),
            }),
            Json::Object(family) => {
                let kind = family
                    .get("records")
                    .and_then(Json::as_str)
                    .filter(|_| family.len() == 1 && name.ends_with('_'))
                    .ok_or_else(|| {
                        format!(
                            "{name} is neither a line nor, ending in _, {{\"records\": a record \
                             type}}"
                        )
                    })?;
                made.extend(named_records(records, kind).map(|(id, english)| Token {
                    name: format!("{name}{}", id.split_once(':').map_or(id, |(_, own)| own)),
                    english: english.to_owned(),
                    context: format!("{kind}.name"),
                    id: id.to_owned(),
                }));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Array(_) => {
                return Err(format!("{name} is neither a line nor a type of record"));
            }
        }
    }
    for token in &made {
        if tokens.iter().any(|given| given.name == token.name) {
            return Err(format!(
                "{} is given a line, and a {} record makes it too",
                token.name,
                token.context.trim_end_matches(".name")
            ));
        }
    }
    tokens.extend(made);
    tokens.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(Presence {
        tokens,
        writes_english: true,
    })
}

/// Reads [`PRESENCE_ENGLISH`], kept by hand.
///
/// # Errors
///
/// When it is not a rich presence file, as `rich-presence` reads one, in English.
pub fn presence_english(text: &str) -> Result<Presence, String> {
    let read = presence::read(text)?;
    if read.language != "english" {
        return Err(format!("is in {}, not english", read.language));
    }
    Ok(Presence {
        tokens: read
            .tokens
            .into_iter()
            .map(|(name, english)| Token {
                id: braced(&english),
                name,
                english,
                context: PRESENCE_CONTEXT.to_owned(),
            })
            .collect(),
        writes_english: false,
    })
}

/// Each record of `records` typed `kind` with an ID and a name, as its ID and name.
fn named_records<'records>(
    records: &'records [Json],
    kind: &'records str,
) -> impl Iterator<Item = (&'records str, &'records str)> {
    records.iter().filter_map(move |record| {
        if record.get("type").and_then(Json::as_str) != Some(kind) {
            return None;
        }
        let id = record.get("id").and_then(Json::as_str)?;
        let name = record
            .get("name")
            .and_then(Json::as_str)
            .filter(|name| !name.is_empty())?;
        Some((id, name))
    })
}

fn object(text: &str, what: &str) -> Result<Map<String, Json>, String> {
    match serde_json::from_str(text) {
        Ok(Json::Object(object)) => Ok(object),
        Ok(_) => Err(format!("is not {what}")),
        Err(error) => Err(format!("is not JSON: {error}")),
    }
}

/// Every file of the text in English and in each language of `translations`, by its path from
/// the build scripts' folder, with what it holds; or every translation missing from them.
///
/// # Errors
///
/// Each text a catalogue does not translate or marks fuzzy, and each rich presence file that would
/// not read back as one.
pub fn written(
    english: &English,
    translations: &[Catalogued],
) -> Result<Vec<(String, String)>, Vec<String>> {
    let mut files = Vec::new();
    let mut wrong = Vec::new();
    let mut languages: Vec<Option<&Catalogued>> = vec![None];
    languages.extend(translations.iter().map(Some));
    let mut disclosed = Vec::new();
    let mut recommended = Vec::new();
    let mut screenshots = Vec::new();
    for each in languages {
        let language = each.map_or("english", |catalogued| catalogued.language);
        let mut said = |context: &str, id: &str| {
            each.map_or_else(
                || id.to_owned(),
                |catalogued| {
                    translated(catalogued, context, id).unwrap_or_else(|why| {
                        wrong.push(why);
                        String::new()
                    })
                },
            )
        };
        if let Some(store) = &english.store {
            if !store.disclosure.is_empty() {
                disclosed.push((language, said(STORE_CONTEXT, &store.disclosure)));
            }
            for (index, alt) in store.screenshots.iter().enumerate() {
                screenshots.push((
                    format!("{language} {}", index.saturating_add(1)),
                    said(STORE_CONTEXT, alt),
                ));
            }
            if !store.item.is_empty() {
                let (typed, fields): (Vec<_>, Vec<_>) = store_fields(store, &mut said)
                    .into_iter()
                    .partition(|(name, _)| {
                        requirement(name).is_some_and(|(_, tier, _)| tier == RECOMMENDED)
                    });
                recommended.extend(
                    typed
                        .into_iter()
                        .map(|(name, text)| (format!("{name}[{language}]"), text)),
                );
                let mut pairs = vec![("itemid", store.item.as_str()), ("language", language)];
                pairs.extend(
                    fields
                        .iter()
                        .map(|(name, text)| (name.as_str(), text.as_str())),
                );
                files.push((
                    format!("{STORE_FOLDER}/{language}.json"),
                    json_object(&pairs),
                ));
            }
        }
        if let Some(page) = &english.workshop {
            let title = said(WORKSHOP_CONTEXT, &page.title);
            let description = said(WORKSHOP_CONTEXT, &page.description);
            files.push((
                format!("{WORKSHOP_FOLDER}/{language}.json"),
                json_object(&[
                    ("language", language),
                    ("title", &title),
                    ("description", &description),
                ]),
            ));
        }
        if let Some(told) = &english.presence
            && (each.is_some() || told.writes_english)
        {
            let tokens: Vec<(String, String)> = told
                .tokens
                .iter()
                .map(|token| {
                    let line = match each {
                        None => token.english.clone(),
                        Some(_) => steam_fields(&said(&token.context, &token.id)),
                    };
                    (token.name.clone(), line)
                })
                .collect();
            let name = format!("rich_presence_{language}.vdf");
            match presence_file(language, tokens) {
                Ok(text) => files.push((name, text)),
                Err(why) => wrong.push(format!("{name} would not be read back: {why}")),
            }
        }
    }
    sheet(&mut files, DISCLOSURE, &disclosed);
    sheet(&mut files, BASIC_INFO, &recommended);
    sheet(&mut files, SCREENSHOTS, &screenshots);
    if wrong.is_empty() {
        files.sort();
        Ok(files)
    } else {
        Err(wrong)
    }
}

/// Every system requirement `store` gives, in English and in each language of `translations`.
///
/// # Errors
///
/// Each requirement a catalogue does not translate or marks fuzzy.
pub fn requirements(
    store: &Store,
    translations: &[Catalogued],
) -> Result<Vec<Requirement>, Vec<String>> {
    let mut found = Vec::new();
    let mut wrong = Vec::new();
    let mut languages: Vec<Option<&Catalogued>> = vec![None];
    languages.extend(translations.iter().map(Some));
    for each in languages {
        let language = each.map_or("english", |catalogued| catalogued.language);
        for part in &store.blocks {
            let Part::Field { name, text } = part else {
                continue;
            };
            let Some((platform, tier, field)) = requirement(name) else {
                continue;
            };
            let text = match each {
                None => text.clone(),
                Some(catalogued) => match translated(catalogued, STORE_CONTEXT, text) {
                    Ok(translation) => translation,
                    Err(why) => {
                        wrong.push(why);
                        continue;
                    }
                },
            };
            found.push(Requirement {
                language: language.to_owned(),
                platform: platform.to_owned(),
                tier: tier.to_owned(),
                field: field.to_owned(),
                text,
            });
        }
    }
    if wrong.is_empty() {
        Ok(found)
    } else {
        Err(wrong)
    }
}

/// The platform, tier and rest of the name of a system requirement's field, such as
/// `app[content][sysreqs][windows][rec][osversion]`; `None` for any other field.
fn requirement(name: &str) -> Option<(&str, &str, &str)> {
    let inner = name.strip_prefix(REQUIREMENT)?.strip_suffix(']')?;
    let mut parts = inner.splitn(3, "][");
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// The translation `catalogued` gives `id` under `context`.
fn translated(catalogued: &Catalogued, context: &str, id: &str) -> Result<String, String> {
    let file = &catalogued.file;
    match catalogued.catalogue.find(context, id) {
        Some(entry) if entry.fuzzy => Err(format!(
            "{file}: \"{id}\", under msgctxt \"{context}\", is marked fuzzy"
        )),
        Some(entry) if !entry.text.is_empty() => Ok(entry.text.clone()),
        _ => Err(format!(
            "{file} does not translate \"{id}\", under msgctxt \"{context}\""
        )),
    }
}

/// The store page's fields by their names on the form, as its Localization tab takes them: the
/// short description, About This Game marked up as Steamworks marks it, the alt text of its
/// images, then each other field.
///
/// Steam numbers the About's images in the order it shows them, and keys each one's alt text by
/// that number.
fn store_fields(
    store: &Store,
    said: &mut impl FnMut(&str, &str) -> String,
) -> Vec<(String, String)> {
    let mut short = String::new();
    let mut about: Vec<String> = Vec::new();
    let mut described = Vec::new();
    let mut images = 0_usize;
    let mut fields = Vec::new();
    let mut listing = false;
    for part in &store.blocks {
        if let Part::Image { alt, .. } = part {
            if let Some(alt) = alt {
                described.push((
                    format!("{ALT_TEXT}{images}][alt_text]"),
                    said(STORE_CONTEXT, alt),
                ));
            }
            images = images.saturating_add(1);
        }
        let line = match part {
            Part::Short(english) => {
                short = said(STORE_CONTEXT, english);
                continue;
            }
            Part::Field { name, text } => {
                fields.push((name.clone(), said(STORE_CONTEXT, text)));
                continue;
            }
            Part::Heading(english) => format!("[h2]{}[/h2]", said(STORE_CONTEXT, english)),
            Part::Paragraph(english) => format!("[p]{}[/p]", said(STORE_CONTEXT, english)),
            Part::Item(english) => format!("[*]{}", said(STORE_CONTEXT, english)),
            Part::Image { name, .. } => format!("[img]{{STEAM_APP_IMAGE}}/extras/{name}[/img]"),
        };
        if matches!(part, Part::Item(_)) != listing {
            about.push(if listing { "[/list]" } else { "[list]" }.to_owned());
            listing = !listing;
        }
        about.push(line);
    }
    if listing {
        about.push("[/list]".to_owned());
    }
    // The store's editor keeps its text with Windows line ends, and hands it back so.
    let mut out = vec![
        ("app[content][short_description]".to_owned(), short),
        ("app[content][about]".to_owned(), about.join("\r\n")),
    ];
    out.extend(described);
    out.extend(fields);
    out
}

/// A rich presence file for `language` holding `tokens`, checked as `rich-presence` reads one.
fn presence_file(language: &str, tokens: Vec<(String, String)>) -> Result<String, String> {
    let text = |key: &str, value: String| Pair {
        key: key.to_owned(),
        value: Value::Text(value),
    };
    let block = |key: &str, pairs: Vec<Pair>| Pair {
        key: key.to_owned(),
        value: Value::Block(Block { pairs }),
    };
    let lang = block(
        "lang",
        vec![
            text("Language", language.to_owned()),
            block(
                "Tokens",
                tokens
                    .into_iter()
                    .map(|(token, line)| text(&token, line))
                    .collect(),
            ),
        ],
    );
    let written = vdf::write(&Block { pairs: vec![lang] }).map_err(|error| error.to_string())?;
    presence::read(&written).map(|_| written)
}

/// `pairs` added to `files` as the file `name`, if there are any.
fn sheet(files: &mut Vec<(String, String)>, name: &str, pairs: &[(impl AsRef<str>, String)]) {
    if pairs.is_empty() {
        return;
    }
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, text)| (key.as_ref(), text.as_str()))
        .collect();
    files.push((name.to_owned(), json_object(&pairs)));
}

/// `pairs` as a JSON object, in their order, laid out as Prettier lays out JSON.
fn json_object(pairs: &[(&str, &str)]) -> String {
    let quoted = |text: &str| Json::String(text.to_owned()).to_string();
    let lines: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("  {}: {}", quoted(key), quoted(value)))
        .collect();
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

/// `line` with Steam's fields, `%field%`, written as a translator sees them, `{field}`: read as
/// printf, `%bishop%` would hold a `%c`, and a language that moves the words would lose it.
fn braced(line: &str) -> String {
    rewrapped(line, '%', '%', '{', '}')
}

/// `line` with each `{field}` a translation carries written back as Steam takes it, `%field%`; a
/// token's whole reference, `{#Token_{field}}`, keeps its braces.
fn steam_fields(line: &str) -> String {
    rewrapped(line, '{', '}', '%', '%')
}

/// `line` with each word of letters, digits and `_` between `open` and `close` put between `into`
/// and `out` instead.
fn rewrapped(line: &str, open: char, close: char, into: char, out: char) -> String {
    let mut result = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find(open) {
        let (before, from) = rest.split_at(start);
        result.push_str(before);
        let after = from.get(open.len_utf8()..).unwrap_or_default();
        let word = after
            .find(|letter: char| !(letter.is_ascii_alphanumeric() || letter == '_'))
            .map_or(after, |end| after.get(..end).unwrap_or_default());
        let tail = after.get(word.len()..).unwrap_or_default();
        if !word.is_empty() && tail.starts_with(close) {
            result.push(into);
            result.push_str(word);
            result.push(out);
            rest = tail.get(close.len_utf8()..).unwrap_or_default();
        } else {
            result.push(open);
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::po;

    const STORE_PAGE: &str = r#"{
  "itemid": 1000090,
  "disclosure": "Made with AI assistance for coding.",
  "blocks": [
    { "kind": "short", "text": "A duel of melodies." },
    { "kind": "paragraph", "text": "Lay a note." },
    { "kind": "heading", "text": "Score the figure" },
    { "kind": "image", "image": "about_score" },
    { "kind": "item", "text": "Strings" },
    { "kind": "item", "text": "Bells" },
    { "kind": "field", "field": "app[content][legal]", "text": "All rights reserved." },
    { "kind": "paragraph", "text": "Then the next." }
  ]
}"#;

    const GERMAN: &str = r#"msgid ""
msgstr "Language: de\n"

msgctxt "store page"
msgid "A duel of melodies."
msgstr "Das Duell der Melodien."

msgctxt "store page"
msgid "Lay a note."
msgstr "Leg eine Note."

msgctxt "store page"
msgid "Score the figure"
msgstr "Werte die Figur"

msgctxt "store page"
msgid "Strings"
msgstr "Saiten"

msgctxt "store page"
msgid "Bells"
msgstr "Glocken"

msgctxt "store page"
msgid "All rights reserved."
msgstr "Rechte vorbehalten."

msgctxt "store page"
msgid "Then the next."
msgstr "Dann die letzte."

msgctxt "store page"
msgid "The board, set for a duel."
msgstr "Das Brett, bereit zum Duell."

msgctxt "store page"
msgid "A duel at dusk."
msgstr "Das Duell am Abend."

msgctxt "store page"
msgid "The bells ring."
msgstr "Die Glocken klingen."

msgctxt "store page"
msgid "Made with AI assistance for coding."
msgstr "Mit KI-Hilfe beim Programmieren gemacht."

msgctxt "steam workshop"
msgid "Mods for your mill"
msgstr "Mods in deinem Kessel"

msgctxt "steam workshop"
msgid "Made by other players."
msgstr "Von anderen Spielern gemacht."

msgctxt "rich presence"
msgid "Choosing a bishop"
msgstr "Sucht einen Magier aus"

msgctxt "rich presence"
msgid "{#Bishop_{bishop}}, duelling"
msgstr "{#Bishop_{bishop}}, im Duell"

msgctxt "rich presence"
msgid "A bishop"
msgstr "Der Magier"

msgctxt "bishop.name"
msgid "base:alder"
msgstr "Die Erlenhexe"
"#;

    fn german(text: &str) -> Catalogued {
        Catalogued {
            file: "de.po".to_owned(),
            language: "german",
            catalogue: po::read(text).unwrap(),
        }
    }

    fn file<'files>(files: &'files [(String, String)], name: &str) -> &'files str {
        &files.iter().find(|(written, _)| written == name).unwrap().1
    }

    fn records() -> Vec<Json> {
        serde_json::from_str(
            r#"[
                {"type": "bishop", "id": "base:alder", "name": "The Alder Witch"},
                {"type": "bishop", "id": "base:nameless"},
                {"type": "bishop", "id": "base:blank", "name": ""},
                {"type": "bishop", "name": "No ID"},
                {"type": "note", "id": "base:c", "name": "C"}
            ]"#,
        )
        .unwrap()
    }

    const PRESENCE_SOURCE: &str = r##"{
  "#Status_Race": "{#Bishop_%bishop%}, duelling",
  "#Status_Brooding": "Choosing a bishop",
  "#Bishop_other": "A bishop",
  "#Bishop_": {"records": "bishop"}
}"##;

    #[test]
    fn the_store_page_is_written_as_steamworks_uploads_it_in_every_language() {
        let english = English {
            store: Some(store(STORE_PAGE).unwrap()),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            [
                "ai_disclosure.json",
                "store/english.json",
                "store/german.json"
            ]
        );
        assert_eq!(
            file(&files, "store/german.json"),
            "{\n  \"itemid\": \"1000090\",\n  \"language\": \"german\",\n  \
             \"app[content][short_description]\": \"Das Duell der Melodien.\",\n  \
             \"app[content][about]\": \"[p]Leg eine Note.[/p]\\r\\n[h2]Werte die Figur[/h2]\\r\\n\
             [img]{STEAM_APP_IMAGE}/extras/about_score[/img]\\r\\n[list]\\r\\n[*]Saiten\\r\\n\
             [*]Glocken\\r\\n[/list]\\r\\n[p]Dann die letzte.[/p]\",\n  \
             \"app[content][legal]\": \"Rechte vorbehalten.\"\n}\n"
        );
        assert!(
            file(&files, "store/english.json").contains("\"[p]Lay a note.[/p]\\r\\n"),
            "{files:?}"
        );
        assert_eq!(
            file(&files, "ai_disclosure.json"),
            "{\n  \"english\": \"Made with AI assistance for coding.\",\n  \
             \"german\": \"Mit KI-Hilfe beim Programmieren gemacht.\"\n}\n"
        );
    }

    #[test]
    fn recommended_requirements_leave_the_upload_for_the_basic_info_sheet_and_minimum_ones_stay() {
        let page = STORE_PAGE.replace(
            r#"{ "kind": "paragraph", "text": "Then the next." }"#,
            r#"{ "kind": "field", "field": "app[content][sysreqs][linux][min][osversion]",
                 "text": "Strings" },
               { "kind": "field", "field": "app[content][sysreqs][linux][rec][graphics]",
                 "text": "Bells" }"#,
        );
        let store = store(&page).unwrap();
        let english = English {
            store: Some(store.clone()),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        let upload = file(&files, "store/german.json");
        assert!(
            upload.contains("\"app[content][sysreqs][linux][min][osversion]\": \"Saiten\"")
                && !upload.contains("[rec]"),
            "{upload}"
        );
        assert_eq!(
            file(&files, BASIC_INFO),
            "{\n  \"app[content][sysreqs][linux][rec][graphics][english]\": \"Bells\",\n  \
             \"app[content][sysreqs][linux][rec][graphics][german]\": \"Glocken\"\n}\n"
        );
        let found = requirements(&store, &[german(GERMAN)]).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|each| {
                    format!(
                        "{} {} {} {} {}",
                        each.language, each.platform, each.tier, each.field, each.text
                    )
                })
                .collect::<Vec<_>>(),
            [
                "english linux min osversion Strings",
                "english linux rec graphics Bells",
                "german linux min osversion Saiten",
                "german linux rec graphics Glocken"
            ]
        );
        assert_eq!(
            requirements(&store, &[german(&GERMAN.replace("Glocken", ""))]),
            Err(vec![
                "de.po does not translate \"Bells\", under msgctxt \"store page\"".to_owned()
            ])
        );
    }

    #[test]
    fn an_about_image_s_alt_text_goes_up_under_its_place_among_the_about_s_images() {
        let page = STORE_PAGE.replace(
            r#"{ "kind": "item", "text": "Strings" },"#,
            r#"{ "kind": "image", "image": "about_board", "alt": "The board, set for a duel." },
               { "kind": "item", "text": "Strings" },"#,
        );
        let english = English {
            store: Some(store(&page).unwrap()),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        let upload = file(&files, "store/german.json");
        assert!(
            upload.contains(
                "[/img]\\r\\n[list]\\r\\n[*]Saiten\\r\\n[*]Glocken\\r\\n[/list]\\r\\n\
                 [p]Dann die letzte.[/p]\",\n  \
                 \"app[assets][extra][1][alt_text]\": \"Das Brett, bereit zum Duell.\",\n  \
                 \"app[content][legal]\""
            ) && !upload.contains("[extra][0]"),
            "{upload}"
        );
        assert!(
            upload.contains("[img]{STEAM_APP_IMAGE}/extras/about_board[/img]"),
            "{upload}"
        );
        assert!(
            file(&files, "store/english.json")
                .contains("\"app[assets][extra][1][alt_text]\": \"The board, set for a duel.\""),
            "{files:?}"
        );
        assert_eq!(
            written(
                &english,
                &[german(&GERMAN.replace("Das Brett, bereit zum Duell.", ""))]
            ),
            Err(vec![
                "de.po does not translate \"The board, set for a duel.\", under msgctxt \
                 \"store page\""
                    .to_owned()
            ])
        );
    }

    #[test]
    fn screenshots_alt_text_is_written_to_a_sheet_of_its_own_in_every_language() {
        let page = STORE_PAGE.replacen(
            '{',
            r#"{ "screenshots": ["A duel at dusk.", "The bells ring."],"#,
            1,
        );
        let english = English {
            store: Some(store(&page).unwrap()),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        assert_eq!(
            file(&files, SCREENSHOTS),
            "{\n  \"english 1\": \"A duel at dusk.\",\n  \"english 2\": \"The bells ring.\",\n  \
             \"german 1\": \"Das Duell am Abend.\",\n  \"german 2\": \"Die Glocken klingen.\"\n}\n"
        );
        assert!(
            !file(&files, "store/german.json").contains("Abend"),
            "{files:?}"
        );
    }

    #[test]
    fn only_a_field_named_as_a_system_requirement_with_a_platform_tier_and_part_is_one() {
        assert_eq!(
            requirement("app[content][sysreqs][mac][rec][memory][amount]"),
            Some(("mac", "rec", "memory][amount"))
        );
        for other in [
            "app[content][legal]",
            "app[content][sysreqs][windows][min]",
            "app[content][sysreqs][windows][min][osversion",
            "sysreqs[windows][min][osversion]",
        ] {
            assert_eq!(requirement(other), None, "{other}");
        }
    }

    #[test]
    fn a_store_page_naming_no_item_writes_no_upload_but_its_disclosure() {
        let page = STORE_PAGE.replace("\"itemid\": 1000090,", "");
        let english = English {
            store: Some(store(&page).unwrap()),
            ..English::default()
        };
        let files = written(&english, &[]).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["ai_disclosure.json"]
        );
        let without = STORE_PAGE.replace(
            "  \"disclosure\": \"Made with AI assistance for coding.\",\n",
            "",
        );
        assert_eq!(store(&without).unwrap().disclosure, "");
        assert_eq!(store("{\"itemid\": \"12\"}").unwrap().item, "12");
    }

    #[test]
    fn a_store_page_steam_cannot_take_is_refused_naming_what_is_wrong() {
        for (page, why) in [
            ("[]", "is not an object holding \"itemid\" and \"blocks\""),
            (
                "{\"itemid\": true}",
                "\"itemid\" is neither a number nor a text",
            ),
            ("{\"disclosure\": 1}", "\"disclosure\" is not a text"),
            ("{\"blocks\": {}}", "\"blocks\" is not a list"),
            ("{\"blocks\": [{}]}", "block 1 has no \"kind\""),
            (
                "{\"blocks\": [{\"kind\": \"short\", \"text\": \"\"}, {\"kind\": \"quote\"}]}",
                "block 2 is a \"quote\", which is none of short, heading, paragraph, item, \
                 image and field",
            ),
            (
                "{\"blocks\": [{\"kind\": \"image\", \"file\": \"a.png\"}]}",
                "block 1 has no \"image\" text",
            ),
            (
                "{\"blocks\": [{\"kind\": \"field\", \"text\": \"a\"}]}",
                "block 1 has no \"field\" text",
            ),
            (
                "{\"blocks\": [{\"kind\": \"image\", \"image\": \"a\", \"alt\": \"\"}]}",
                "block 1's \"alt\" is empty: leave it out of an image with none",
            ),
            (
                "{\"blocks\": [{\"kind\": \"image\", \"image\": \"a\", \"alt\": 1}]}",
                "block 1's \"alt\" is not a text",
            ),
            (
                "{\"blocks\": [{\"kind\": \"image\", \"image\": \"a\", \"alt\": \"A.\"}, \
                 {\"kind\": \"short\", \"text\": \"\"}, {\"kind\": \"image\", \"image\": \"a\"}]}",
                "block 3 shows a a second time, and alt text is written only for an About that \
                 shows each image once",
            ),
            (
                "{\"itemid\": 1, \"title\": \"A\"}",
                "\"title\" is none of itemid, disclosure, screenshots and blocks",
            ),
            (
                "{\"blocks\": [{\"kind\": \"short\", \"text\": \"A.\", \"alt\": \"B.\"}]}",
                "block 1 holds \"alt\", which a block of kind short does not take: it takes text",
            ),
            (
                "{\"blocks\": [{\"kind\": \"image\", \"image\": \"a\", \"width\": 1}]}",
                "block 1 holds \"width\", which a block of kind image does not take: it takes \
                 image and alt",
            ),
            (
                "{\"screenshots\": \"A.\"}",
                "\"screenshots\" is not a list of each screenshot's alt text",
            ),
            (
                "{\"screenshots\": [\"A.\", \"\"]}",
                "screenshot 2's alt text is empty",
            ),
            (
                "{\"screenshots\": [null]}",
                "screenshot 1's alt text is not a text",
            ),
        ] {
            assert_eq!(store(page), Err(why.to_owned()), "{page}");
        }
        assert!(store("{").unwrap_err().starts_with("is not JSON: "));
        assert!(
            store(
                "{\"blocks\": [{\"kind\": \"image\", \"image\": \"a\"}, \
                 {\"kind\": \"image\", \"image\": \"a\"}]}"
            )
            .is_ok(),
            "an image shown twice with no alt text anywhere"
        );
    }

    #[test]
    fn the_workshop_page_is_written_in_every_language() {
        let english = English {
            workshop: Some(
                workshop(
                    r#"{"title": "Mods for your mill", "description": "Made by other players."}"#,
                )
                .unwrap(),
            ),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        assert_eq!(
            file(&files, "workshop/german.json"),
            "{\n  \"language\": \"german\",\n  \"title\": \"Mods in deinem Kessel\",\n  \
             \"description\": \"Von anderen Spielern gemacht.\"\n}\n"
        );
        assert!(file(&files, "workshop/english.json").contains("\"Mods for your mill\""));
        assert_eq!(
            workshop(r#"{"title": "a", "description": "b", "tags": "c"}"#),
            Err(
                "\"tags\" is no field of the Workshop page, which has a title and a description"
                    .to_owned()
            )
        );
        assert_eq!(
            workshop(r#"{"title": "a"}"#),
            Err("there is no \"description\" text".to_owned())
        );
    }

    #[test]
    fn rich_presence_is_written_with_steam_s_fields_and_a_token_for_each_named_record() {
        let english = English {
            presence: Some(presence(PRESENCE_SOURCE, &records()).unwrap()),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        assert_eq!(
            file(&files, "rich_presence_german.vdf"),
            "\"lang\"\n{\n\t\"Language\"\t\t\"german\"\n\t\"Tokens\"\n\t{\n\
             \t\t\"#Bishop_alder\"\t\t\"Die Erlenhexe\"\n\
             \t\t\"#Bishop_other\"\t\t\"Der Magier\"\n\
             \t\t\"#Status_Brooding\"\t\t\"Sucht einen Magier aus\"\n\
             \t\t\"#Status_Race\"\t\t\"{#Bishop_%bishop%}, im Duell\"\n\t}\n}\n"
        );
        assert!(
            file(&files, "rich_presence_english.vdf")
                .contains("\t\t\"#Bishop_alder\"\t\t\"The Alder Witch\"\n"),
            "{files:?}"
        );
    }

    #[test]
    fn rich_presence_kept_by_hand_in_english_is_translated_and_left_as_it_is() {
        let english = English {
            presence: Some(
                presence_english(
                    "\"lang\"\n{\n\t\"Language\"\t\"english\"\n\t\"Tokens\"\n\t{\n\
                     \t\t\"#Status_Brooding\"\t\"Choosing a bishop\"\n\t}\n}\n",
                )
                .unwrap(),
            ),
            ..English::default()
        };
        let files = written(&english, &[german(GERMAN)]).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["rich_presence_german.vdf"]
        );
        assert_eq!(
            presence_english("\"lang\" { \"Language\" \"german\" \"Tokens\" { \"#a\" \"b\" } }"),
            Err("is in german, not english".to_owned())
        );
    }

    #[test]
    fn rich_presence_steam_cannot_take_is_refused_naming_what_is_wrong() {
        for (source, why) in [
            ("[]", "is not an object of tokens and their English"),
            (r#"{"menu": "At the menu"}"#, "menu does not start with #"),
            (
                r##"{"#menu": 1}"##,
                "#menu is neither a line nor a type of record",
            ),
            (
                r##"{"#Bishop": {"records": "bishop"}}"##,
                "#Bishop is neither a line nor, ending in _, {\"records\": a record type}",
            ),
            (
                r##"{"#Bishop_": {"records": "bishop", "field": "title"}}"##,
                "#Bishop_ is neither a line nor, ending in _, {\"records\": a record type}",
            ),
            (
                r##"{"#Bishop_alder": "Alder", "#Bishop_": {"records": "bishop"}}"##,
                "#Bishop_alder is given a line, and a bishop record makes it too",
            ),
        ] {
            assert_eq!(
                presence(source, &records()).map(|_| ()),
                Err(why.to_owned()),
                "{source}"
            );
        }
        let quoted = English {
            presence: Some(presence(r##"{"#menu": "At the \"menu\""}"##, &[]).unwrap()),
            ..English::default()
        };
        assert_eq!(
            written(&quoted, &[]),
            Err(vec![
                "rich_presence_english.vdf would not be read back: \"At the \"menu\"\" holds a \
                 double quote, which a KeyValues file cannot hold"
                    .to_owned()
            ])
        );
    }

    #[test]
    fn every_text_a_catalogue_leaves_untranslated_or_fuzzy_is_named_and_nothing_written() {
        let english = English {
            store: Some(store(STORE_PAGE).unwrap()),
            workshop: Some(
                workshop(r#"{"title": "New title", "description": "Made by other players."}"#)
                    .unwrap(),
            ),
            presence: Some(presence(PRESENCE_SOURCE, &records()).unwrap()),
        };
        let unsure = GERMAN.replace("msgstr \"Saiten\"", "msgstr \"\"").replace(
            "msgctxt \"bishop.name\"",
            "#, fuzzy\nmsgctxt \"bishop.name\"",
        );
        assert_eq!(
            written(&english, &[german(&unsure)]),
            Err(vec![
                "de.po does not translate \"Strings\", under msgctxt \"store page\"".to_owned(),
                "de.po does not translate \"New title\", under msgctxt \"steam workshop\""
                    .to_owned(),
                "de.po: \"base:alder\", under msgctxt \"bishop.name\", is marked fuzzy".to_owned(),
            ])
        );
    }

    #[test]
    fn steam_s_fields_are_braced_for_translators_and_back_with_a_token_kept_whole() {
        assert_eq!(
            braced("{#Bishop_%bishop%} at 100% %"),
            "{#Bishop_{bishop}} at 100% %"
        );
        assert_eq!(
            steam_fields("{#Bishop_{bishop}}, {open {} {two words}"),
            "{#Bishop_%bishop%}, {open {} {two words}"
        );
        assert_eq!(steam_fields("ends {"), "ends {");
    }
}
