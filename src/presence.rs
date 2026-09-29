//! Rich presence localisation: Valve's per-language token files, read and checked before they
//! replace what Steam holds.
//!
//! Steam replaces a language's tokens with each upload, so the files are the whole truth: a
//! token taken out of a file is gone from Steam after the next upload.

use serde_json::{Value as Json, json};

use crate::vdf::{self, Value};

/// One language's tokens, as a file gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// Steam's name for the language, such as `english` or `schinese`.
    pub language: String,
    /// Each token, such as `#menu`, and its text, in the file's order.
    pub tokens: Vec<(String, String)>,
}

/// Reads a rich presence file: a `"lang"` block naming its `"Language"`, with its `"Tokens"`.
///
/// # Errors
///
/// When the text is not such a file, names no language or one that is not a single lowercase
/// word, holds no tokens, a token does not start with `#` or is there twice, or a text refers to
/// tokens (`{#season_%season%}`) that no token starts as.
pub fn read(text: &str) -> Result<Language, String> {
    let document = vdf::parse(text).map_err(|error| error.to_string())?;
    let lang = document.block("lang").ok_or("there is no \"lang\" block")?;
    let language = lang
        .text("Language")
        .ok_or("\"lang\" names no \"Language\"")?
        .to_owned();
    if language.is_empty() || !language.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Err(format!(
            "\"{language}\" is not a language as Steam names them, such as english or schinese"
        ));
    }
    let listed = lang.block("Tokens").ok_or("\"lang\" holds no \"Tokens\"")?;
    let mut tokens: Vec<(String, String)> = Vec::with_capacity(listed.pairs.len());
    for pair in &listed.pairs {
        let Value::Text(value) = &pair.value else {
            return Err(format!("{} is a block, not a text", pair.key));
        };
        if !pair.key.starts_with('#') {
            return Err(format!("{} does not start with #", pair.key));
        }
        if tokens
            .iter()
            .any(|(token, _)| token.eq_ignore_ascii_case(&pair.key))
        {
            return Err(format!("{} is there twice", pair.key));
        }
        tokens.push((pair.key.clone(), value.clone()));
    }
    if tokens.is_empty() {
        return Err("\"Tokens\" is empty, which would take every token off Steam".to_owned());
    }
    for (token, value) in &tokens {
        for prefix in references(value) {
            let named = tokens.iter().any(|(other, _)| {
                other
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            });
            if !named {
                return Err(format!(
                    "{token} refers to {{{prefix}...}}, and no token starts {prefix}"
                ));
            }
        }
    }
    Ok(Language { language, tokens })
}

/// The token names a text refers to, such as `#season_` in `{#season_%season%}`: up to the first
/// `%` or the closing brace.
fn references(value: &str) -> Vec<&str> {
    value
        .match_indices("{#")
        .filter_map(|(start, _)| {
            let rest = value.get(start.saturating_add(1)..)?;
            let end = rest.find(['%', '}'])?;
            rest.get(..end)
        })
        .collect()
}

/// What `IProductInfoService/SetRichPresenceLocalization` is given for `app`, as its
/// `input_json`: every language, each with all of its tokens.
#[must_use]
pub fn request(app: u32, languages: &[Language]) -> Json {
    json!({
        "appid": app,
        "languages": languages
            .iter()
            .map(|language| json!({
                "language": language.language,
                "tokens": language
                    .tokens
                    .iter()
                    .map(|(token, value)| json!({"token": token, "value": value}))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fantasy Guild Manager's `steam/rich_presence_english.vdf`, trimmed.
    const FGM: &str = "\"lang\"\n{\n\t\"Language\"\t\"english\"\n\t\"Tokens\"\n\t{\n\
        \t\t\"#menu\"\t\"At the menu\"\n\
        \t\t\"#running\"\t\"Running the %guild% guild: {#season_%season%} of year %year%\"\n\
        \t\t\"#season_autumn\"\t\"Autumn\"\n\t}\n}\n";

    #[test]
    fn a_file_is_read_as_valve_writes_it() {
        let language = read(FGM).unwrap();
        assert_eq!(language.language, "english");
        assert_eq!(
            language.tokens,
            [
                ("#menu".to_owned(), "At the menu".to_owned()),
                (
                    "#running".to_owned(),
                    "Running the %guild% guild: {#season_%season%} of year %year%".to_owned()
                ),
                ("#season_autumn".to_owned(), "Autumn".to_owned()),
            ]
        );
    }

    #[test]
    fn a_file_that_would_upload_wrong_says_why() {
        let with = |language: &str, tokens: &str| {
            format!("\"lang\" {{ \"Language\" \"{language}\" \"Tokens\" {{ {tokens} }} }}")
        };
        for (text, why) in [
            ("\"other\" {}".to_owned(), "there is no \"lang\" block"),
            (
                "\"lang\" { \"Tokens\" { \"#a\" \"A\" } }".to_owned(),
                "\"lang\" names no",
            ),
            (
                with("English", "\"#a\" \"A\""),
                "\"English\" is not a language",
            ),
            (
                "\"lang\" { \"Language\" \"english\" }".to_owned(),
                "\"lang\" holds no",
            ),
            (with("english", ""), "\"Tokens\" is empty"),
            (
                with("english", "\"menu\" \"A\""),
                "menu does not start with #",
            ),
            (
                with("english", "\"#a\" \"A\" \"#A\" \"B\""),
                "#A is there twice",
            ),
            (with("english", "\"#a\" { }"), "#a is a block"),
            (
                with("english", "\"#running\" \"In {#season_%season%}\""),
                "#running refers to {#season_...}, and no token starts #season_",
            ),
        ] {
            let error = read(&text).unwrap_err();
            assert!(error.starts_with(why), "{text}: {error}");
        }
    }

    #[test]
    fn a_reference_is_read_up_to_its_variable_or_brace() {
        assert_eq!(
            references("{#season_%season%} and {#menu} but not #plain"),
            ["#season_", "#menu"]
        );
        assert_eq!(references("{#unclosed"), Vec::<&str>::new());
    }

    #[test]
    fn the_request_holds_every_language_and_token() {
        let english = read(FGM).unwrap();
        let german = Language {
            language: "german".to_owned(),
            tokens: vec![("#menu".to_owned(), "Im Men\u{fc}".to_owned())],
        };
        assert_eq!(
            request(5_335_950, &[english, german]),
            json!({"appid": 5_335_950_u32, "languages": [
                {"language": "english", "tokens": [
                    {"token": "#menu", "value": "At the menu"},
                    {"token": "#running",
                     "value": "Running the %guild% guild: {#season_%season%} of year %year%"},
                    {"token": "#season_autumn", "value": "Autumn"},
                ]},
                {"language": "german", "tokens": [{"token": "#menu", "value": "Im Men\u{fc}"}]},
            ]})
        );
    }
}
