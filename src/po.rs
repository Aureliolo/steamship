//! Gettext catalogues, the `.po` files a game keeps its translations in, read for the texts Steam
//! shows in each language.

use std::mem;

/// One translation: what `id` says under `context`, in the catalogue's language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub context: Option<String>,
    pub id: String,
    /// The translation, or the first of its plural forms; empty while it is untranslated.
    pub text: String,
    /// Marked fuzzy: a guess left for a translator to confirm, which gettext does not use either.
    pub fuzzy: bool,
}

/// A catalogue: the language its header names, and its entries in the file's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalogue {
    /// The header's `Language`, such as `de` or `pt_BR`.
    pub language: Option<String>,
    pub entries: Vec<Entry>,
}

impl Catalogue {
    /// The entry for `id` under `context`.
    #[must_use]
    pub fn find(&self, context: &str, id: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.context.as_deref() == Some(context) && entry.id == id)
    }

    fn holds(&self, context: Option<&str>, id: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.context.as_deref() == context && entry.id == id)
    }
}

/// Each locale gettext names a language by, with Steam's name for it, from Valve's table of the
/// languages Steam supports; the last three are the other names locales go by.
const LANGUAGES: [(&str, &str); 34] = [
    ("ar", "arabic"),
    ("bg", "bulgarian"),
    ("zh-CN", "schinese"),
    ("zh-TW", "tchinese"),
    ("cs", "czech"),
    ("da", "danish"),
    ("nl", "dutch"),
    ("en", "english"),
    ("fi", "finnish"),
    ("fr", "french"),
    ("de", "german"),
    ("el", "greek"),
    ("hu", "hungarian"),
    ("id", "indonesian"),
    ("it", "italian"),
    ("ja", "japanese"),
    ("ko", "koreana"),
    ("ms", "malay"),
    ("no", "norwegian"),
    ("pl", "polish"),
    ("pt", "portuguese"),
    ("pt-BR", "brazilian"),
    ("ro", "romanian"),
    ("ru", "russian"),
    ("es", "spanish"),
    ("es-419", "latam"),
    ("sv", "swedish"),
    ("th", "thai"),
    ("tr", "turkish"),
    ("uk", "ukrainian"),
    ("vi", "vietnamese"),
    ("nb", "norwegian"),
    ("zh-Hans", "schinese"),
    ("zh-Hant", "tchinese"),
];

/// Steam's name for the language of `locale`, such as `german` for `de` and `brazilian` for
/// `pt_BR`, or none when Steam has no such language.
#[must_use]
pub fn steam_language(locale: &str) -> Option<&'static str> {
    let locale = locale.replace('_', "-");
    LANGUAGES
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(&locale))
        .map(|(_, language)| *language)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Context,
    Id,
    Plural,
    Text,
}

/// The entry being read, until the next one starts.
#[derive(Default)]
struct Pending {
    context: Option<String>,
    id: Option<String>,
    texts: Vec<String>,
    fuzzy: bool,
    /// The line it starts on, to name it by.
    line: usize,
    last: Option<Part>,
}

/// Reads the catalogue `text`.
///
/// Comments are left alone but for the fuzzy flag, and so are obsolete entries; of a plural
/// entry, the first form is kept.
///
/// # Errors
///
/// When a line is not gettext's, a string is not quoted or escaped as gettext's are, an entry
/// lacks its `msgid` or its `msgstr`, or two entries have the same context and `msgid`; each
/// named by its line.
pub fn read(text: &str) -> Result<Catalogue, String> {
    let mut catalogue = Catalogue::default();
    let mut pending = Pending::default();
    let mut fuzzy = false;
    for (index, raw) in text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .enumerate()
    {
        let number = index.saturating_add(1);
        let line = raw.trim();
        if let Some(flags) = line.strip_prefix("#,") {
            fuzzy |= flags.split(',').any(|flag| flag.trim() == "fuzzy");
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |why: &str| format!("line {number}: {why}");
        if line.starts_with('"') {
            let Some(part) = pending.last else {
                return Err(at("a string that continues nothing"));
            };
            let more = quoted(line).map_err(|why| at(&why))?;
            match part {
                Part::Context => pending.context.get_or_insert_default().push_str(&more),
                Part::Id => pending.id.get_or_insert_default().push_str(&more),
                Part::Plural => {}
                Part::Text => {
                    if let Some(last) = pending.texts.last_mut() {
                        last.push_str(&more);
                    }
                }
            }
            continue;
        }
        let (keyword, rest) = line.split_once([' ', '\t']).unwrap_or((line, ""));
        let value = quoted(rest.trim_start()).map_err(|why| at(&format!("{keyword} {why}")))?;
        let part = match keyword {
            "msgctxt" => Part::Context,
            "msgid" => Part::Id,
            "msgid_plural" => Part::Plural,
            "msgstr" => Part::Text,
            _ if keyword.starts_with("msgstr[") && keyword.ends_with(']') => Part::Text,
            _ => return Err(at(&format!("\"{keyword}\" is not a gettext keyword"))),
        };
        if matches!(part, Part::Context | Part::Id) && !pending.texts.is_empty() {
            finish(mem::take(&mut pending), &mut catalogue)?;
        }
        if pending.context.is_none() && pending.id.is_none() {
            pending.fuzzy = mem::take(&mut fuzzy);
            pending.line = number;
        }
        match part {
            Part::Context if pending.context.is_some() || pending.id.is_some() => {
                return Err(at("msgctxt comes before its msgid, once"));
            }
            Part::Context => pending.context = Some(value),
            Part::Id if pending.id.is_some() => return Err(at("a second msgid before a msgstr")),
            Part::Id => pending.id = Some(value),
            Part::Plural | Part::Text if pending.id.is_none() => {
                return Err(at(&format!("{keyword} with no msgid before it")));
            }
            Part::Plural if !pending.texts.is_empty() => {
                return Err(at("msgid_plural comes before the msgstr"));
            }
            Part::Plural => {}
            Part::Text if keyword == "msgstr" && !pending.texts.is_empty() => {
                return Err(at("a second msgstr for one msgid"));
            }
            Part::Text => pending.texts.push(value),
        }
        pending.last = Some(part);
    }
    if pending.id.is_some() || pending.context.is_some() {
        finish(pending, &mut catalogue)?;
    }
    Ok(catalogue)
}

/// Adds the entry read as `pending` to `catalogue`, or takes the language from it when it is the
/// header.
fn finish(pending: Pending, catalogue: &mut Catalogue) -> Result<(), String> {
    let line = pending.line;
    let Some(id) = pending.id else {
        return Err(format!("line {line}: a msgctxt with no msgid"));
    };
    let Some(text) = pending.texts.into_iter().next() else {
        return Err(format!("line {line}: msgid \"{id}\" has no msgstr"));
    };
    if id.is_empty() && pending.context.is_none() {
        catalogue.language = text
            .lines()
            .find_map(|field| field.strip_prefix("Language:"))
            .map(str::trim)
            .filter(|language| !language.is_empty())
            .map(str::to_owned);
        return Ok(());
    }
    if catalogue.holds(pending.context.as_deref(), &id) {
        return Err(format!(
            "line {line}: msgid \"{id}\" is in the catalogue twice"
        ));
    }
    catalogue.entries.push(Entry {
        context: pending.context,
        id,
        text,
        fuzzy: pending.fuzzy,
    });
    Ok(())
}

/// The string `text` holds, between its quotes, with gettext's escapes undone.
fn quoted(text: &str) -> Result<String, String> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or("is not followed by a quoted string")?;
    let mut unquoted = String::with_capacity(inner.len());
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => unquoted.push(match characters.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('"') => '"',
                Some('\\') => '\\',
                Some(other) => return Err(format!("has \\{other}, which gettext does not escape")),
                None => return Err("ends its string with a lone backslash".to_owned()),
            }),
            '"' => return Err("has a quote in its string that is not escaped".to_owned()),
            _ => unquoted.push(character),
        }
    }
    Ok(unquoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GERMAN: &str = "\u{feff}# Tidal Mill Inn\n\
        msgid \"\"\n\
        msgstr \"\"\n\
        \"Project-Id-Version: Tidal Mill Inn\\n\"\n\
        \"Language: de\\n\"\n\
        \n\
        #. The achievement's name.\n\
        msgctxt \"achievement.name\"\n\
        msgid \"base:first_anchor\"\n\
        msgstr \"Unten im Keller\"\n\
        \n\
        #, fuzzy\n\
        msgctxt \"achievement.description\"\n\
        msgid \"base:first_anchor\"\n\
        msgstr \"\"\n\
        \"Grabe den \\\"ersten\\\" Keller\\n\"\n\
        \"einer Herberge.\"\n\
        \n\
        msgid \"guest\"\n\
        msgid_plural \"guests\"\n\
        msgstr[0] \"Gast\"\n\
        msgstr[1] \"G\u{e4}ste\"\n\
        #~ msgid \"gone\"\n\
        #~ msgstr \"weg\"\n";

    #[test]
    fn a_catalogue_is_read_as_its_language_and_each_entry_under_its_context() {
        let catalogue = read(GERMAN).unwrap();
        assert_eq!(catalogue.language.as_deref(), Some("de"));
        assert_eq!(
            catalogue.entries,
            vec![
                Entry {
                    context: Some("achievement.name".to_owned()),
                    id: "base:first_anchor".to_owned(),
                    text: "Unten im Keller".to_owned(),
                    fuzzy: false,
                },
                Entry {
                    context: Some("achievement.description".to_owned()),
                    id: "base:first_anchor".to_owned(),
                    text: "Grabe den \"ersten\" Keller\neiner Herberge.".to_owned(),
                    fuzzy: true,
                },
                Entry {
                    context: None,
                    id: "guest".to_owned(),
                    text: "Gast".to_owned(),
                    fuzzy: false,
                },
            ]
        );
        assert_eq!(
            catalogue
                .find("achievement.name", "base:first_anchor")
                .map(|entry| entry.text.as_str()),
            Some("Unten im Keller")
        );
        assert_eq!(catalogue.find("achievement.name", "guest"), None);
    }

    #[test]
    fn a_catalogue_that_is_not_gettexts_says_where() {
        for (text, why) in [
            (
                "msgid \"a\"\nmsgstr \"b\nc\"\n",
                "line 2: msgstr is not followed by a quoted string",
            ),
            (
                "msgid \"a\"\nmsgstr[0 \"b\"\n",
                "line 2: \"msgstr[0\" is not a gettext keyword",
            ),
            (
                "msgid \"a\"\nplural[0] \"b\"\n",
                "line 2: \"plural[0]\" is not a gettext keyword",
            ),
            (
                "msgid \"a\"\nmsgstr \"b\\q\"\n",
                "line 2: msgstr has \\q, which gettext does not escape",
            ),
            (
                "msgid \"a\"\nmsgstr \"b\\\"\n",
                "line 2: msgstr ends its string with a lone backslash",
            ),
            (
                "msgid \"a\\\"\nmsgstr \"\"\n",
                "line 1: msgid ends its string with a lone backslash",
            ),
            (
                "msgid \"a\"b\"\nmsgstr \"\"\n",
                "line 1: msgid has a quote in its string that is not escaped",
            ),
            ("\"a\"\n", "line 1: a string that continues nothing"),
            (
                "msgid \"a\"\nmsgtxt \"b\"\n",
                "line 2: \"msgtxt\" is not a gettext keyword",
            ),
            (
                "msgid \"a\"\nmsgctxt \"b\"\n",
                "line 2: msgctxt comes before its msgid, once",
            ),
            (
                "msgctxt \"a\"\nmsgctxt \"b\"\n",
                "line 2: msgctxt comes before its msgid, once",
            ),
            (
                "msgid \"a\"\nmsgid \"b\"\n",
                "line 2: a second msgid before a msgstr",
            ),
            ("msgstr \"a\"\n", "line 1: msgstr with no msgid before it"),
            (
                "msgid_plural \"a\"\n",
                "line 1: msgid_plural with no msgid before it",
            ),
            (
                "msgid \"a\"\nmsgstr[0] \"b\"\nmsgid_plural \"c\"\n",
                "line 3: msgid_plural comes before the msgstr",
            ),
            (
                "msgid \"a\"\nmsgstr \"b\"\nmsgstr \"c\"\n",
                "line 3: a second msgstr for one msgid",
            ),
            ("msgid \"a\"\n", "line 1: msgid \"a\" has no msgstr"),
            ("msgctxt \"a\"\n", "line 1: a msgctxt with no msgid"),
            (
                "msgid \"a\"\nmsgstr \"b\"\nmsgid \"a\"\nmsgstr \"c\"\n",
                "line 3: msgid \"a\" is in the catalogue twice",
            ),
        ] {
            assert_eq!(read(text), Err(why.to_owned()), "{text}");
        }
    }

    #[test]
    fn the_same_msgid_under_two_contexts_is_two_entries() {
        let text = "msgctxt \"a\"\nmsgid \"x\"\nmsgstr \"1\"\nmsgctxt \"b\"\nmsgid \"x\"\nmsgstr \"2\"\n\
                    msgid \"x\"\nmsgstr \"3\"\n";
        let catalogue = read(text).unwrap();
        assert_eq!(catalogue.entries.len(), 3);
        assert_eq!(catalogue.language, None);
        assert_eq!(
            catalogue.find("b", "x").map(|entry| entry.text.as_str()),
            Some("2")
        );
    }

    #[test]
    fn an_entry_flagged_fuzzy_more_than_once_is_fuzzy() {
        let catalogue =
            read("#, fuzzy\n#, fuzzy, c-format\n#, no-wrap\nmsgid \"a\"\nmsgstr \"b\"\n").unwrap();
        assert!(catalogue.entries.first().unwrap().fuzzy);
    }

    #[test]
    fn a_header_naming_no_language_names_none() {
        let catalogue = read("msgid \"\"\nmsgstr \"Language: \\n\"\n").unwrap();
        assert_eq!(catalogue, Catalogue::default());
    }

    #[test]
    fn each_locale_is_steams_language_by_its_own_name() {
        for (locale, language) in [
            ("de", Some("german")),
            ("pt_BR", Some("brazilian")),
            ("pt-br", Some("brazilian")),
            ("pt", Some("portuguese")),
            ("zh_CN", Some("schinese")),
            ("zh_Hant", Some("tchinese")),
            ("ko", Some("koreana")),
            ("es_419", Some("latam")),
            ("nb", Some("norwegian")),
            ("tlh", None),
            ("de_AT", None),
        ] {
            assert_eq!(steam_language(locale), language, "{locale}");
        }
    }
}
