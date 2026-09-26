//! Valve's `KeyValues` text format, as steamcmd reads build scripts and serves its manifests.
//!
//! Build scripts hold Windows paths such as `"..\content\"`, and steamcmd reads them literally,
//! so a backslash is an ordinary character here and a quoted string ends at the next quote.
//! Anything this reader does not know how steamcmd treats (`#include`, `#base`, `[$WIN32]`
//! conditions) is refused rather than guessed at.

use std::error::Error;
use std::fmt;

/// Deeper than any script or manifest nests, and shallow enough that the recursive reader cannot
/// exhaust the stack on hostile input.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    pub pairs: Vec<Pair>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub key: String,
    pub value: Value,
}

impl Block {
    /// Every value under `key`, in order. Keys match regardless of case, as they do in Valve's
    /// reader, and a key may repeat (`FileMapping` does).
    pub fn all<'block, 'key>(
        &'block self,
        key: &'key str,
    ) -> impl Iterator<Item = &'block Value> + use<'block, 'key> {
        self.pairs
            .iter()
            .filter(move |pair| pair.key.eq_ignore_ascii_case(key))
            .map(|pair| &pair.value)
    }

    /// The first value under `key`, which is the one Valve's reader answers with.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.all(key).next()
    }

    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Text(text) => Some(text),
            Value::Block(_) => None,
        }
    }

    #[must_use]
    pub fn block(&self, key: &str) -> Option<&Self> {
        match self.get(key)? {
            Value::Block(block) => Some(block),
            Value::Text(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub reason: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {}, column {}: {}",
            self.line, self.column, self.reason
        )
    }
}

impl Error for ParseError {}

/// Reads a whole document: the pairs at its top level, which is one `"AppBuild"` block for a
/// script and several blocks for a manifest.
///
/// # Errors
///
/// Anything that is not well-formed `KeyValues`, or that uses a feature this reader refuses.
pub fn parse(text: &str) -> Result<Block, ParseError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader {
        whole: text,
        rest: text,
    };
    reader.pairs(0)
}

/// Writes `block` back out in the layout Valve's own files use.
///
/// # Errors
///
/// Text that would not read back as written: a double quote, which the format cannot hold, or
/// a key that the reader takes for a directive.
pub fn write(block: &Block) -> Result<String, WriteError> {
    let mut out = String::new();
    write_pairs(block, 0, &mut out)?;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteError {
    pub text: String,
    pub reason: &'static str,
}

impl fmt::Display for WriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "\"{}\" {}", self.text, self.reason)
    }
}

impl Error for WriteError {}

fn write_pairs(block: &Block, depth: usize, out: &mut String) -> Result<(), WriteError> {
    let indent = "\t".repeat(depth);
    for pair in &block.pairs {
        if is_directive(&pair.key) {
            return Err(WriteError {
                text: pair.key.clone(),
                reason: "would be read back as a directive, not a key",
            });
        }
        out.push_str(&indent);
        push_quoted(&pair.key, out)?;
        match &pair.value {
            Value::Text(text) => {
                out.push_str("\t\t");
                push_quoted(text, out)?;
                out.push('\n');
            }
            Value::Block(inner) => {
                out.push('\n');
                out.push_str(&indent);
                out.push_str("{\n");
                write_pairs(inner, depth.saturating_add(1), out)?;
                out.push_str(&indent);
                out.push_str("}\n");
            }
        }
    }
    Ok(())
}

fn push_quoted(text: &str, out: &mut String) -> Result<(), WriteError> {
    if text.contains('"') {
        return Err(WriteError {
            text: text.to_owned(),
            reason: "holds a double quote, which a KeyValues file cannot hold",
        });
    }
    out.push('"');
    out.push_str(text);
    out.push('"');
    Ok(())
}

const fn is_directive(key: &str) -> bool {
    key.eq_ignore_ascii_case("#include") || key.eq_ignore_ascii_case("#base")
}

enum Token<'text> {
    Open,
    Close,
    Text(&'text str),
    End,
}

/// Reads by narrowing `rest`, the text not yet read, so no position is ever computed and no
/// index can fall outside the text or inside a character.
struct Reader<'text> {
    whole: &'text str,
    rest: &'text str,
}

impl<'text> Reader<'text> {
    fn pairs(&mut self, depth: usize) -> Result<Block, ParseError> {
        let mut block = Block::default();
        loop {
            self.skip();
            let start = self.rest;
            let key = match self.token()? {
                Token::End if depth == 0 => return Ok(block),
                Token::End => return Err(self.error_at(start, "a block is missing its closing }")),
                Token::Close if depth == 0 => {
                    return Err(self.error_at(start, "a } closes nothing"));
                }
                Token::Close => return Ok(block),
                Token::Open => return Err(self.error_at(start, "expected a key, found {")),
                Token::Text(key) => key,
            };
            if is_directive(key) {
                return Err(self.error_at(start, "#include and #base are not supported"));
            }
            self.skip();
            let value_start = self.rest;
            let value = match self.token()? {
                Token::Text(text) => Value::Text(text.to_owned()),
                Token::Open => {
                    let Some(deeper) = depth.checked_add(1).filter(|&deeper| deeper < MAX_DEPTH)
                    else {
                        return Err(self.error_at(value_start, "blocks nest too deeply"));
                    };
                    Value::Block(self.pairs(deeper)?)
                }
                Token::Close | Token::End => {
                    return Err(self.error_at(start, &format!("key \"{key}\" has no value")));
                }
            };
            block.pairs.push(Pair {
                key: key.to_owned(),
                value,
            });
        }
    }

    /// Moves past whitespace and `//` comments.
    fn skip(&mut self) {
        loop {
            let trimmed = self.rest.trim_start_matches([' ', '\t', '\r', '\n']);
            let Some(comment) = trimmed.strip_prefix("//") else {
                self.rest = trimmed;
                return;
            };
            self.rest = comment.split_once('\n').map_or("", |(_, after)| after);
        }
    }

    fn token(&mut self) -> Result<Token<'text>, ParseError> {
        let start = self.rest;
        let mut chars = start.chars();
        match chars.next() {
            None => Ok(Token::End),
            Some('{') => {
                self.rest = chars.as_str();
                Ok(Token::Open)
            }
            Some('}') => {
                self.rest = chars.as_str();
                Ok(Token::Close)
            }
            Some('[') => Err(self.error_at(start, "conditions such as [$WIN32] are not supported")),
            Some('"') => match chars.as_str().split_once('"') {
                Some((text, after)) => {
                    self.rest = after;
                    Ok(Token::Text(text))
                }
                None => Err(self.error_at(start, "a quoted string is never closed")),
            },
            Some(first) => {
                // The first character belongs to the token whatever it is, so every read takes
                // at least one character and the reader cannot stop advancing.
                let tail = chars.as_str();
                let end = tail
                    .find([' ', '\t', '\r', '\n', '{', '}', '"', '['])
                    .unwrap_or(tail.len());
                let (text, after) = start
                    .split_at_checked(first.len_utf8().saturating_add(end))
                    .unwrap_or((start, ""));
                self.rest = after;
                Ok(Token::Text(text))
            }
        }
    }

    /// Places `at`, a tail of the text, by line and character.
    fn error_at(&self, at: &str, reason: &str) -> ParseError {
        let read = self.whole.len().saturating_sub(at.len());
        let before = self.whole.get(..read).unwrap_or_default();
        let line = before.rsplit('\n').next().unwrap_or_default();
        ParseError {
            line: before.split('\n').count(),
            column: line.chars().count().saturating_add(1),
            reason: reason.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SIMPLE_APP_BUILD: &str = r#""AppBuild"
{
	"AppID" "1000" // your AppID
	"Desc" "This is a simple build script" // internal description for this build
	"ContentRoot" "..\content\" // root content folder, relative to location of this file
	"BuildOutput" "..\output\" // build output folder for build logs and build cache files
	"Depots"
	{
		"1001" // your DepotID
		{
			"FileMapping"
			{
				"LocalPath" "*" // all files from contentroot folder
				"DepotPath" "." // mapped into the root of the depot
				"recursive" "1" // include all subfolders
			}
		}
	}
}
"#;

    const LINUX_MANIFEST: &str = "\"linux\"\n{\n\t\"version\"\t\t\"1788292693\"\n\
        \t\"steamcmd_linux\"\n\t{\n\t\t\"file\"\t\t\"steamcmd_linux.zip.917c71eb\"\n\
        \t\t\"size\"\t\t\"7924254\"\n\t\t\"IsBootstrapperPackage\"\t\t\"1\"\n\t}\n}\n\
        \"kvsign2\"\n{\n\t\"linux\"\t\t\"eb45dd38\"\n}\n\"kvsignatures\"\n{\n}\n";

    fn parsed(text: &str) -> Block {
        match parse(text) {
            Ok(block) => block,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn reads_valves_simple_script_with_backslashes_literally() {
        let document = parsed(SIMPLE_APP_BUILD);
        let app = document
            .block("appbuild")
            .unwrap_or_else(|| panic!("no AppBuild"));
        assert_eq!(app.text("AppID"), Some("1000"));
        assert_eq!(app.text("contentroot"), Some(r"..\content\"));
        let depot = app.block("Depots").and_then(|depots| depots.block("1001"));
        let mapping = depot.and_then(|depot| depot.block("FileMapping"));
        assert_eq!(
            mapping.and_then(|mapping| mapping.text("Recursive")),
            Some("1")
        );
    }

    #[test]
    fn reads_every_top_level_block_of_a_manifest() {
        let document = parsed(LINUX_MANIFEST);
        let keys: Vec<&str> = document
            .pairs
            .iter()
            .map(|pair| pair.key.as_str())
            .collect();
        assert_eq!(keys, ["linux", "kvsign2", "kvsignatures"]);
        let linux = document
            .block("linux")
            .unwrap_or_else(|| panic!("no linux block"));
        assert_eq!(linux.text("version"), Some("1788292693"));
    }

    #[test]
    fn keeps_repeated_keys_in_order() {
        let document = parsed(r#""a" { "m" "1" "M" "2" "m" { } }"#);
        let block = document.block("a").unwrap_or_else(|| panic!("no a"));
        assert_eq!(block.all("m").count(), 3);
        assert_eq!(block.text("m"), Some("1"));
    }

    #[test]
    fn asks_for_text_or_a_block_and_gets_only_that() {
        let document = parsed(r#""text" "x" "block" { }"#);
        assert_eq!(document.text("block"), None);
        assert_eq!(document.block("text"), None);
        assert_eq!(document.text("absent"), None);
    }

    #[test]
    fn says_what_it_could_not_write() {
        let block = Block {
            pairs: vec![Pair {
                key: "k".into(),
                value: Value::Text("a \"b\"".into()),
            }],
        };
        assert_eq!(
            write(&block).map_err(|error| error.to_string()),
            Err("\"a \"b\"\" holds a double quote, which a KeyValues file cannot hold".to_owned())
        );
    }

    #[test]
    fn reads_bare_tokens_and_a_byte_order_mark() {
        let document = parsed("\u{feff}key value\nother { inner \"x\" }");
        assert_eq!(document.text("key"), Some("value"));
        assert_eq!(
            document
                .block("other")
                .and_then(|other| other.text("inner")),
            Some("x")
        );
    }

    #[test]
    fn refuses_what_it_cannot_read_faithfully() {
        for (text, reason) in [
            ("\"a\" {", "a block is missing its closing }"),
            ("}", "a } closes nothing"),
            ("{", "expected a key, found {"),
            ("\"a\"", "key \"a\" has no value"),
            ("\"a\" \"b", "a quoted string is never closed"),
            (
                "#include \"other.vdf\"",
                "#include and #base are not supported",
            ),
            (
                "\"a\" \"b\" [$WIN32]",
                "conditions such as [$WIN32] are not supported",
            ),
        ] {
            assert_eq!(
                parse(text).map_err(|error| error.reason),
                Err(reason.to_owned())
            );
        }
    }

    #[test]
    fn places_an_error_by_line_and_character() {
        let error = parse("\"a\"\n{\n  \"\u{e9}\" }").err();
        assert_eq!(error.map(|error| (error.line, error.column)), Some((3, 3)));
    }

    fn nested(depth: usize) -> String {
        format!("{}{}", "\"k\" {".repeat(depth), "}".repeat(depth))
    }

    #[test]
    fn refuses_nesting_past_the_limit() {
        assert_eq!(
            parse(&nested(MAX_DEPTH)).map_err(|error| error.reason),
            Err("blocks nest too deeply".to_owned())
        );
        let deepest = parse(&nested(MAX_DEPTH.saturating_sub(1)));
        assert_eq!(deepest.map(|tree| tree.pairs.len()), Ok(1));
    }

    #[test]
    fn refuses_to_write_what_would_not_read_back() {
        for (key, value) in [("a", "say \"hi\""), ("#base", "x")] {
            let block = Block {
                pairs: vec![Pair {
                    key: key.into(),
                    value: Value::Text(value.into()),
                }],
            };
            assert!(
                matches!(write(&block), Err(WriteError { .. })),
                "{key} {value}"
            );
        }
    }

    fn text() -> impl Strategy<Value = String> {
        "[^\"]{0,12}"
    }

    fn block() -> impl Strategy<Value = Block> {
        let leaf = prop::collection::vec(
            (text(), text()).prop_map(|(key, value)| Pair {
                key,
                value: Value::Text(value),
            }),
            0..4,
        )
        .prop_map(|pairs| Block { pairs });
        leaf.prop_recursive(4, 32, 4, |inner| {
            prop::collection::vec(
                (
                    text(),
                    prop_oneof![text().prop_map(Value::Text), inner.prop_map(Value::Block)],
                )
                    .prop_map(|(key, value)| Pair { key, value }),
                0..4,
            )
            .prop_map(|pairs| Block { pairs })
        })
    }

    proptest! {
        #[test]
        fn writing_then_reading_gives_the_same_tree(block in block()) {
            let written = write(&block).map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(parse(&written), Ok(block));
        }

        #[test]
        fn any_text_that_reads_writes_back_the_same(text in ".{0,200}") {
            if let Ok(tree) = parse(&text) {
                let written =
                    write(&tree).map_err(|error| TestCaseError::fail(error.to_string()))?;
                prop_assert_eq!(parse(&written), Ok(tree));
            }
        }
    }
}
