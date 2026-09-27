//! Markdown to the site's HTML. Headings carry anchors, a code block may carry a caption after
//! its language, tables scroll on a narrow screen, and the paragraph under a page's title is its
//! lede.

use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html,
};

/// The site's HTML for `markdown`.
#[must_use]
pub fn to_html(markdown: &str) -> String {
    let mut events = Parser::new_ext(markdown, Options::ENABLE_TABLES);
    let mut out: Vec<Event<'_>> = Vec::new();
    while let Some(event) = events.next() {
        if let Event::Start(Tag::Heading { level, .. }) = event {
            let inner: Vec<Event<'_>> = events
                .by_ref()
                .take_while(|next| !matches!(*next, Event::End(TagEnd::Heading(_))))
                .collect();
            out.push(heading(level, inner));
        } else if let Event::Start(Tag::CodeBlock(kind)) = event {
            let code: String = events
                .by_ref()
                .take_while(|next| !matches!(*next, Event::End(TagEnd::CodeBlock)))
                .filter_map(|next| {
                    if let Event::Text(text) = next {
                        Some(text.into_string())
                    } else {
                        None
                    }
                })
                .collect();
            let info = if let CodeBlockKind::Fenced(info) = kind {
                info.into_string()
            } else {
                String::new()
            };
            out.push(Event::Html(CowStr::from(code_block(&info, &code))));
        } else if let Event::Start(Tag::Table(alignment)) = event {
            out.push(Event::Html(CowStr::Borrowed(r#"<div class="scroll">"#)));
            out.push(Event::Start(Tag::Table(alignment)));
        } else if matches!(event, Event::End(TagEnd::Table)) {
            out.push(event);
            out.push(Event::Html(CowStr::Borrowed("</div>\n")));
        } else {
            out.push(event);
        }
    }
    let mut rendered = String::new();
    html::push_html(&mut rendered, out.into_iter());
    lede(&rendered)
}

/// A heading with an anchor made from its text.
fn heading(level: HeadingLevel, inner: Vec<Event<'_>>) -> Event<'static> {
    let mut text = String::new();
    for event in inner.clone() {
        if let Event::Text(part) | Event::Code(part) = event {
            text.push_str(&part);
        }
    }
    let mut body = String::new();
    html::push_html(&mut body, inner.into_iter());
    let number: u8 = match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    };
    Event::Html(CowStr::from(format!(
        "<h{number} id=\"{}\">{body}</h{number}>\n",
        slug(&text)
    )))
}

/// The anchor a heading's text gives it: lower case, each run of anything else one hyphen.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut gap = false;
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character);
        } else {
            gap = true;
        }
    }
    slug
}

/// A code block, captioned when its info string says more than its language, as in
/// ```` ```sh in your game's repository ````.
#[must_use]
pub fn code_block(info: &str, code: &str) -> String {
    let pre = format!("<pre><code>{}</code></pre>", highlighted(code));
    info.split_once(' ')
        .map(|(_, caption)| caption.trim())
        .filter(|caption| !caption.is_empty())
        .map_or_else(
            || format!("<figure class=\"code\">{pre}</figure>\n"),
            |caption| {
                format!(
                    "<figure class=\"code\"><figcaption>{}</figcaption>{pre}</figure>\n",
                    escaped(caption)
                )
            },
        )
}

/// `code` escaped, its comments dimmed: a `#` at the start of a line or after a space begins one,
/// in every language these pages show.
#[must_use]
pub fn highlighted(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    for (index, line) in code.trim_end_matches('\n').split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let comment = line
            .match_indices('#')
            .map(|(at, _)| at)
            .find(|&at| at == 0 || line.get(..at).is_some_and(|before| before.ends_with(' ')));
        if let Some((code_part, comment_part)) = comment.and_then(|at| line.split_at_checked(at)) {
            out.push_str(&escaped(code_part));
            out.push_str("<i class=\"c\">");
            out.push_str(&escaped(comment_part));
            out.push_str("</i>");
        } else {
            out.push_str(&escaped(line));
        }
    }
    out
}

/// `text` as HTML, its `backticked` names set as code.
#[must_use]
pub fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for part in text.split('`') {
        if in_code {
            out.push_str("<code>");
            out.push_str(&escaped(part));
            out.push_str("</code>");
        } else {
            out.push_str(&escaped(part));
        }
        in_code = !in_code;
    }
    out
}

/// `text` safe inside HTML, attribute values included.
#[must_use]
pub fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The first paragraph after the page's title, marked as its lede.
fn lede(page: &str) -> String {
    page.split_once("</h1>\n<p>").map_or_else(
        || page.to_owned(),
        |(title, rest)| format!("{title}</h1>\n<p class=\"lede\">{rest}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_gets_an_anchor_from_its_text() {
        assert_eq!(
            to_html("## The `ci` command, step 2\n"),
            "<h2 id=\"the-ci-command-step-2\">The <code>ci</code> command, step 2</h2>\n"
        );
        assert_eq!(
            slug("  Already-hyphenated -- text!  "),
            "already-hyphenated-text"
        );
        assert_eq!(slug("--"), "");
        assert_eq!(to_html("###### Six\n"), "<h6 id=\"six\">Six</h6>\n");
    }

    #[test]
    fn the_paragraph_under_the_title_is_the_lede_and_no_other() {
        let page = to_html("# Title\n\nFirst.\n\nSecond.\n");
        assert_eq!(
            page,
            "<h1 id=\"title\">Title</h1>\n<p class=\"lede\">First.</p>\n<p>Second.</p>\n"
        );
        assert_eq!(to_html("Just text.\n"), "<p>Just text.</p>\n");
    }

    #[test]
    fn a_code_block_is_captioned_by_what_follows_its_language() {
        assert_eq!(
            to_html("```sh in your repository\nsteamship ci # once\n```\n"),
            "<figure class=\"code\"><figcaption>in your repository</figcaption><pre><code>\
             steamship ci <i class=\"c\"># once</i></code></pre></figure>\n"
        );
        assert_eq!(
            code_block("yaml", "a: <b>\n"),
            "<figure class=\"code\"><pre><code>a: &lt;b&gt;</code></pre></figure>\n"
        );
        assert_eq!(
            code_block("", "x"),
            "<figure class=\"code\"><pre><code>x</code></pre></figure>\n"
        );
        assert_eq!(
            to_html("    indented\n"),
            "<figure class=\"code\"><pre><code>indented</code></pre></figure>\n"
        );
    }

    #[test]
    fn only_a_hash_that_starts_a_word_starts_a_comment() {
        assert_eq!(
            highlighted("# all\na#b c"),
            "<i class=\"c\"># all</i>\na#b c"
        );
        assert_eq!(highlighted("x # y\n\n"), "x <i class=\"c\"># y</i>");
    }

    #[test]
    fn a_table_scrolls_rather_than_widening_the_page() {
        let table = to_html("| a | b |\n| - | - |\n| 1 | 2 |\n");
        assert!(
            table.starts_with("<div class=\"scroll\"><table>"),
            "{table}"
        );
        assert!(table.ends_with("</table>\n</div>\n"), "{table}");
    }

    #[test]
    fn backticks_become_code_and_the_rest_is_escaped() {
        assert_eq!(
            inline("run `a<b>` & go"),
            "run <code>a&lt;b&gt;</code> &amp; go"
        );
        assert_eq!(inline("no code"), "no code");
        assert_eq!(escaped(r#"a & <b> "c""#), "a &amp; &lt;b&gt; &quot;c&quot;");
    }
}
