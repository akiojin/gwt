//! Markdown rendering for Board posts (SPEC-2963).
//!
//! Board post bodies are authored in Markdown (the canonical format). Each
//! provider renders it differently:
//! - Local web UI: [`markdown_to_html`] (sanitized HTML, same pipeline as the
//!   Knowledge surface).
//! - Microsoft Teams: [`markdown_to_teams_html`] (HTML body; headings become
//!   bold+`<br>` since Graph does not reliably render `<h1>`–`<h6>`).
//! - Slack: [`markdown_to_slack_mrkdwn`] (mrkdwn; headings degrade to a bold
//!   line since mrkdwn has no heading syntax).
//!
//! HTML output is sanitized with `ammonia` (XSS-safe) before it can reach the
//! web UI or Teams.

use std::collections::HashSet;

use pulldown_cmark::{html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Render Markdown to sanitized HTML for the local web UI.
pub fn markdown_to_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, options);
    let mut raw_html = String::new();
    html::push_html(&mut raw_html, parser);

    ammonia::Builder::default()
        .add_tags(["table", "thead", "tbody", "tr", "th", "td"])
        .clean(&raw_html)
        .to_string()
}

/// Render Markdown to the limited HTML subset Microsoft Teams renders in a
/// channel message body. Headings are rewritten to bold + line break because
/// Graph does not reliably render `<h1>`–`<h6>`.
pub fn markdown_to_teams_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(markdown, options);
    let mut raw_html = String::new();
    html::push_html(&mut raw_html, parser);
    let rewritten = rewrite_headings_to_bold(&raw_html);
    let teams_html = normalize_teams_line_breaks(&rewritten);

    let tags: HashSet<&str> = [
        "p",
        "br",
        "strong",
        "b",
        "em",
        "i",
        "u",
        "ul",
        "ol",
        "li",
        "a",
        "blockquote",
        "code",
        "pre",
        "del",
        "s",
    ]
    .into_iter()
    .collect();
    let sanitized = ammonia::Builder::default()
        .tags(tags)
        .clean(&teams_html)
        .to_string();
    sanitized.replace('\n', "")
}

/// Strip all HTML tags, keeping only text. Used to keep Teams-origin HTML
/// bodies readable on read-back (POST-only scope: no HTML→Markdown parse-back).
pub fn strip_html_tags(html: &str) -> String {
    ammonia::Builder::default()
        .tags(HashSet::new())
        .clean(html)
        .to_string()
}

fn rewrite_headings_to_bold(html: &str) -> String {
    let mut out = html.to_string();
    for level in 1..=6 {
        out = out
            .replace(&format!("<h{level}>"), "<strong>")
            .replace(&format!("</h{level}>"), "</strong><br>");
    }
    out
}

fn normalize_teams_line_breaks(html: &str) -> String {
    let mut out = html.replace("\r\n", "\n").replace('\r', "\n");
    while out.contains(">\n<") {
        out = out.replace(">\n<", "><");
    }
    out = out.replace("</p><p>", "<br><br>");
    for tag in ["ul", "ol", "blockquote", "pre"] {
        out = out.replace(&format!("</p><{tag}"), &format!("<br><{tag}"));
        out = out.replace(&format!("</{tag}><p>"), &format!("</{tag}><br>"));
    }
    out = out.replace("<p>", "").replace("</p>", "");
    out = out.replace('\n', "<br>");
    trim_trailing_br(out)
}

fn trim_trailing_br(mut html: String) -> String {
    while html.ends_with("<br>") {
        html.truncate(html.len() - "<br>".len());
    }
    html
}

/// Escape the three characters Slack reserves in message text.
fn escape_slack_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render Markdown to Slack `mrkdwn`. Bold/italic/strikethrough/links/lists/
/// blockquote/code are mapped; headings degrade to a bold line (mrkdwn has no
/// heading syntax).
pub fn markdown_to_slack_mrkdwn(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);

    let mut out = String::new();
    // Ordered-list counters (None = unordered).
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut in_blockquote = false;
    let mut in_code_block = false;
    // Link text is buffered so we can emit `<dest|text>` on link end.
    let mut link_dest: Option<String> = None;
    let mut link_text = String::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Strong => out.push('*'),
                Tag::Emphasis => out.push('_'),
                Tag::Strikethrough => out.push('~'),
                Tag::Heading { .. } => out.push('*'),
                Tag::Paragraph if in_blockquote => out.push_str("> "),
                Tag::List(start) => list_stack.push(start),
                Tag::Item => {
                    let prefix = match list_stack.last_mut() {
                        Some(Some(n)) => {
                            let s = format!("{n}. ");
                            *n += 1;
                            s
                        }
                        _ => "\u{2022} ".to_string(),
                    };
                    out.push_str(&prefix);
                }
                Tag::Link { dest_url, .. } => {
                    link_dest = Some(dest_url.to_string());
                    link_text.clear();
                }
                Tag::BlockQuote(_) => in_blockquote = true,
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    match kind {
                        CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                            out.push_str("```");
                            out.push_str(&lang);
                            out.push('\n');
                        }
                        _ => out.push_str("```\n"),
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Strong => out.push('*'),
                TagEnd::Emphasis => out.push('_'),
                TagEnd::Strikethrough => out.push('~'),
                TagEnd::Heading(_) => out.push_str("*\n"),
                TagEnd::Paragraph => out.push('\n'),
                TagEnd::List(_) => {
                    list_stack.pop();
                    out.push('\n');
                }
                TagEnd::Item => out.push('\n'),
                TagEnd::Link => {
                    if let Some(dest) = link_dest.take() {
                        out.push_str(&format!("<{dest}|{link_text}>"));
                        link_text.clear();
                    }
                }
                TagEnd::BlockQuote(_) => in_blockquote = false,
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    out.push_str("```\n");
                }
                _ => {}
            },
            Event::Text(text) => {
                if link_dest.is_some() {
                    link_text.push_str(&escape_slack_text(&text));
                } else if in_code_block {
                    // Code content is literal; do not escape.
                    out.push_str(&text);
                } else {
                    out.push_str(&escape_slack_text(&text));
                }
            }
            Event::Code(code) => {
                let rendered = format!("`{code}`");
                if link_dest.is_some() {
                    link_text.push_str(&rendered);
                } else {
                    out.push_str(&rendered);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if link_dest.is_some() {
                    link_text.push(' ');
                } else {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }

    collapse_blank_lines(out.trim())
}

/// Collapse runs of 3+ newlines to a single blank line.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newline_run = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push('\n');
            }
        } else {
            newline_run = 0;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_renders_basics_and_sanitizes_xss() {
        let html = markdown_to_html("# Title\n\n**bold** and *italic*\n\n- a\n- b");
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<em>italic</em>"), "{html}");
        assert!(html.contains("<li>a</li>"), "{html}");

        let dirty = markdown_to_html("<script>alert(1)</script>\n\n[x](javascript:alert(1))");
        assert!(!dirty.contains("<script>"), "script stripped: {dirty}");
        assert!(
            !dirty.contains("href=\"javascript"),
            "js scheme href stripped: {dirty}"
        );
    }

    #[test]
    fn teams_html_maps_headings_to_bold_break_and_strips_h_tags() {
        let html = markdown_to_teams_html("## Heading\n\n**b** _i_");
        assert!(!html.contains("<h2>"), "no h2 tag: {html}");
        assert!(html.contains("<strong>Heading</strong><br>"), "{html}");
        assert!(html.contains("<strong>b</strong>"), "{html}");
        assert!(html.contains("<em>i</em>"), "{html}");
        // XSS still stripped.
        let dirty = markdown_to_teams_html("<script>x</script>ok");
        assert!(!dirty.contains("<script>"), "{dirty}");
    }

    #[test]
    fn teams_html_preserves_paragraph_breaks_without_raw_newlines() {
        let html = markdown_to_teams_html("Current state: A\n\nReason: B\n\nNext: C");
        assert_eq!(html, "Current state: A<br><br>Reason: B<br><br>Next: C");
        assert!(
            !html.contains('\n'),
            "Teams HTML must not carry raw newlines: {html:?}"
        );
        assert!(
            !html.contains("\\n"),
            "Teams HTML must not carry escaped newlines: {html:?}"
        );
    }

    #[test]
    fn teams_html_preserves_soft_breaks_as_br() {
        let html = markdown_to_teams_html("line one\nline two");
        assert_eq!(html, "line one<br>line two");
    }

    #[test]
    fn slack_bold_italic_strike() {
        assert_eq!(markdown_to_slack_mrkdwn("**bold**"), "*bold*");
        assert_eq!(markdown_to_slack_mrkdwn("*italic*"), "_italic_");
        assert_eq!(markdown_to_slack_mrkdwn("_italic_"), "_italic_");
        assert_eq!(markdown_to_slack_mrkdwn("~~gone~~"), "~gone~");
    }

    #[test]
    fn slack_heading_degrades_to_bold_line() {
        assert_eq!(markdown_to_slack_mrkdwn("# Heading"), "*Heading*");
        assert_eq!(markdown_to_slack_mrkdwn("### Sub"), "*Sub*");
    }

    #[test]
    fn slack_lists_links_code_blockquote() {
        assert_eq!(
            markdown_to_slack_mrkdwn("- a\n- b"),
            "\u{2022} a\n\u{2022} b"
        );
        assert_eq!(markdown_to_slack_mrkdwn("1. a\n2. b"), "1. a\n2. b");
        assert_eq!(
            markdown_to_slack_mrkdwn("[gwt](https://example.com)"),
            "<https://example.com|gwt>"
        );
        assert_eq!(markdown_to_slack_mrkdwn("`code`"), "`code`");
        assert_eq!(markdown_to_slack_mrkdwn("> quote"), "> quote");
    }

    #[test]
    fn slack_blockquote_emits_literal_marker_in_multi_element_doc() {
        // Regression for the SPEC-2963 Slack E2E: ensure a blockquote after
        // other blocks still emits a literal `> ` marker (Slack renders the
        // blockquote; its API echoes the char as `&gt;`, which is Slack's
        // transport escaping, not ours).
        let out = markdown_to_slack_mrkdwn("`code`\n\n> a quoted line");
        assert!(out.ends_with("> a quoted line"), "got: {out:?}");
        assert!(!out.contains("&gt;"), "marker must not be escaped: {out:?}");
    }

    #[test]
    fn slack_escapes_reserved_characters() {
        assert_eq!(
            markdown_to_slack_mrkdwn("1 < 2 & 3 > 0"),
            "1 &lt; 2 &amp; 3 &gt; 0"
        );
    }

    #[test]
    fn strip_html_tags_keeps_text() {
        assert_eq!(strip_html_tags("<strong>Hi</strong><br>there"), "Hithere");
    }

    #[test]
    fn slack_code_blocks_and_inline_link_code() {
        // Fenced block with a language: content is literal, never escaped.
        let fenced = markdown_to_slack_mrkdwn("```rust\nlet x = 1 < 2;\n```");
        assert!(fenced.contains("```rust"), "{fenced}");
        assert!(
            fenced.contains("let x = 1 < 2;"),
            "code stays literal: {fenced}"
        );

        // Fenced block without a language.
        let plain = markdown_to_slack_mrkdwn("```\nplain & raw\n```");
        assert!(plain.contains("```"), "{plain}");
        assert!(plain.contains("plain & raw"), "{plain}");

        // Inline code inside link text is buffered into the link rendering.
        let link = markdown_to_slack_mrkdwn("[run `gwt` now](https://example.com)");
        assert_eq!(link, "<https://example.com|run `gwt` now>");

        // A soft break between paragraph lines becomes a newline.
        assert_eq!(
            markdown_to_slack_mrkdwn("line one\nline two"),
            "line one\nline two"
        );
    }
}
