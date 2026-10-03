//! Slice B: message bodies — fenced code blocks with highlighting, links (opened through
//! `commands::open_link`), `@name` mentions — as styled, selectable text. Ported from
//! `renderBody`, `inline`, `codeBlock`, `highlight` and `ytIds` in `app.js`.
//!
//! The parsing is plain Rust over the raw body (there is no HTML to escape any more, so the
//! B-49 and B-58 orderings are moot: ranges are computed on the text itself and never
//! re-scanned). Drawing goes through [`rich_text::RichText`], which keeps the text in the
//! window-wide selection so a message can be selected and copied like in the webview.

mod rich_text;

pub use rich_text::RichText;

use super::super::theme::{self, c};
use super::AppSnapshot;
use gpui_kit::{
    div, px, relative, AnyElement, ClipboardItem, ElementId,
    HighlightStyle, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use std::ops::Range;

/// `.msg-text { max-width: 72ch }` at 15px in the bitmap face, whose ASCII is half-width
/// (7.5px). The extra half pixel keeps a line of exactly 72 characters from wrapping on
/// rounding, as it did not in the webview.
pub const TEXT_MEASURE: f32 = 540.5;

// ------------------------------------------------------------------ parsing

/// One piece of a message body: running text, or a fenced code block.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text { text: String, spans: Vec<Span> },
    Code { info: String, code: String, tokens: Vec<(Range<usize>, Tok)> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpanKind {
    /// `@name` of somebody else, or of me in a message that does not mention me.
    Mention,
    /// `@me` in a message that mentions me.
    MentionSelf,
    Link(String),
}

/// The three code hues plus the quiet comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Key,
    Str,
    Num,
    Com,
}

/// `renderBody`: split on the fences, odd parts are code; text parts that are only
/// whitespace are dropped. `names` are the display names a mention may match (mine first).
pub fn parse_body(text: &str, names: &[&str], me: &str, mentions_you: bool) -> Vec<Block> {
    text.split("```")
        .enumerate()
        .filter(|(i, part)| i % 2 == 1 || !part.trim().is_empty())
        .map(|(i, part)| {
            if i % 2 == 1 {
                code_block(part)
            } else {
                let text = collapse_whitespace(part);
                let spans = inline_spans(&text, names, me, mentions_you);
                Block::Text { text, spans }
            }
        })
        .collect()
}

/// What HTML did to the text of a `<div>` with `<br>` for each newline: runs of spaces and
/// tabs become one space, a line loses its leading and trailing spaces, and one trailing
/// line break draws no empty line.
fn collapse_whitespace(part: &str) -> String {
    let part = part.strip_suffix('\n').unwrap_or(part);
    let lines: Vec<String> = part
        .split('\n')
        .map(|line| line.split([' ', '\t']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))
        .collect();
    lines.join("\n")
}

/// `codeBlock`: whatever follows the opening fence is the header ("code" when empty).
fn code_block(part: &str) -> Block {
    let (info, code) = match part.find('\n') {
        Some(nl) => (part[..nl].trim().to_string(), part[nl + 1..].strip_suffix('\n').unwrap_or(&part[nl + 1..])),
        None => (String::new(), part),
    };
    let code = code.to_string();
    let tokens = highlight(&code);
    Block::Code { info, code, tokens }
}

fn is_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// A JavaScript `\b` at byte `at` of `s` (outside the string counts as a non-word char).
fn boundary(s: &str, at: usize) -> bool {
    let before = s[..at].chars().next_back().is_some_and(is_word);
    let after = s[at..].chars().next().is_some_and(is_word);
    before != after
}

/// `inline`: mentions first (B-49), then links in what is left. A mention is `@` plus a
/// display name that exists, case-insensitive, followed by a word boundary; a link runs
/// from `http(s)://` to the next whitespace or mention.
pub fn inline_spans(text: &str, names: &[&str], me: &str, mentions_you: bool) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for name in names.iter().filter(|n| !n.is_empty()) {
        let mine = *name == me;
        let mut from = 0;
        while let Some(at) = text[from..].find('@').map(|i| from + i) {
            from = at + 1;
            let Some(end) = match_name(text, at + 1, name) else { continue };
            if !boundary(text, end) || spans.iter().any(|s| s.range.start < end && at < s.range.end) {
                continue;
            }
            let kind = if mine && mentions_you { SpanKind::MentionSelf } else { SpanKind::Mention };
            spans.push(Span { range: at..end, kind });
            from = end;
        }
    }
    spans.sort_by_key(|s| s.range.start);

    let mut links = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let Some(at) = ["https://", "http://"]
            .iter()
            .filter_map(|p| text[from..].find(p).map(|i| from + i))
            .min()
        else {
            break;
        };
        if let Some(inside) = spans.iter().find(|s| s.range.contains(&at)) {
            from = inside.range.end;
            continue;
        }
        let stop = spans.iter().map(|s| s.range.start).filter(|&s| s > at).min().unwrap_or(text.len());
        let end = text[at..stop].find(char::is_whitespace).map_or(stop, |i| at + i);
        links.push(Span {
            range: at..end,
            kind: SpanKind::Link(text[at..end].to_string()),
        });
        from = end;
    }
    spans.extend(links);
    spans.sort_by_key(|s| s.range.start);
    spans
}

/// The end of `name` matched case-insensitively at byte `at` of `text`.
fn match_name(text: &str, at: usize, name: &str) -> Option<usize> {
    let mut chars = text[at..].char_indices();
    for want in name.chars() {
        let (_, got) = chars.next()?;
        if !got.to_lowercase().eq(want.to_lowercase()) {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(i, _)| at + i))
}

/// `highlight`: per line, the key before an `=`, quoted strings, numbers on word
/// boundaries and a `#` comment to the end of the line. Unlike the old version this runs
/// on the raw text, so a quoted string shows verbatim in the string hue (the webview
/// garbled it, refs shot 03). Strings win over `#` and digits inside them. The ranges are
/// sorted, disjoint and on char boundaries.
pub fn highlight(code: &str) -> Vec<(Range<usize>, Tok)> {
    let mut out = Vec::new();
    let mut base = 0;
    for line in code.split('\n') {
        highlight_line(line, base, &mut out);
        base += line.len() + 1;
    }
    out
}

fn highlight_line(line: &str, base: usize, out: &mut Vec<(Range<usize>, Tok)>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    // `^(\s*)([A-Za-z_][\w.-]*)(\s*=)`
    let lead = line.len() - line.trim_start().len();
    let ident_end = line[lead..]
        .char_indices()
        .take_while(|(k, ch)| if *k == 0 { ch.is_ascii_alphabetic() || *ch == '_' } else { is_word(*ch) || *ch == '.' || *ch == '-' })
        .last()
        .map(|(k, ch)| lead + k + ch.len_utf8());
    if let Some(end) = ident_end {
        if line[end..].trim_start().starts_with('=') {
            out.push((base + lead..base + end, Tok::Key));
            i = end;
        }
    }
    while i < bytes.len() {
        let ch = line[i..].chars().next().unwrap_or(' ');
        if ch == '"' {
            if let Some(close) = line[i + 1..].find('"') {
                let end = i + 1 + close + 1;
                out.push((base + i..base + end, Tok::Str));
                i = end;
                continue;
            }
        } else if ch == '#' {
            out.push((base + i..base + line.len(), Tok::Com));
            return;
        } else if ch.is_ascii_digit() && boundary(line, i) {
            // `\b(\d[\d_.]*)\b`, with the regex's backtracking to the last boundary.
            let greedy = line[i..]
                .char_indices()
                .take_while(|(k, c)| if *k == 0 { true } else { c.is_ascii_digit() || *c == '_' || *c == '.' })
                .last()
                .map_or(i + 1, |(k, _)| i + k + 1);
            if let Some(end) = (i + 1..=greedy).rev().find(|&e| boundary(line, e)) {
                out.push((base + i..base + end, Tok::Num));
                i = end;
                continue;
            }
        }
        // Skip a whole word, so digits inside an identifier never start a number.
        if is_word(ch) {
            while let Some(c) = line[i..].chars().next().filter(|c| is_word(*c)) {
                i += c.len_utf8();
            }
        } else {
            i += ch.len_utf8();
        }
    }
}

/// `ytIds`: the video ids linked in a body, in order, without repeats, two at most (a card
/// is tall). The same URL shapes as the old `YT_ID` regex; Rust checks the id again
/// before it goes anywhere (`youtube::geldig_id`).
pub fn yt_ids(text: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(at) = ["https://", "http://"]
        .iter()
        .filter_map(|p| text[from..].find(p).map(|i| (from + i, p.len())))
        .min()
    {
        let (start, scheme) = at;
        from = start + scheme;
        let mut rest = &text[start + scheme..];
        for host in ["www.", "m.", "music."] {
            if let Some(r) = rest.strip_prefix(host) {
                rest = r;
                break;
            }
        }
        let id_at = if let Some(r) = rest.strip_prefix("youtube.com/") {
            if let Some(q) = r.strip_prefix("watch?") {
                watch_v(q)
            } else {
                ["shorts/", "live/", "embed/", "v/"].iter().find_map(|p| r.strip_prefix(p))
            }
        } else {
            rest.strip_prefix("youtu.be/")
        };
        let Some(after) = id_at else { continue };
        let id: String = after.chars().take(11).collect();
        if id.chars().count() == 11 && id.chars().all(|c| is_word(c) || c == '-') {
            from = text.len() - after.len() + id.len();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids.truncate(2);
    ids
}

/// `watch\?(?:[\w=&%.-]*&)?v=`: the greedy group tries the last `&` first.
fn watch_v(q: &str) -> Option<&str> {
    let run = q
        .char_indices()
        .take_while(|(_, c)| is_word(*c) || matches!(c, '=' | '&' | '%' | '.' | '-'))
        .last()
        .map_or(0, |(k, c)| k + c.len_utf8());
    q[..run]
        .match_indices('&')
        .map(|(k, _)| k + 1)
        .rev()
        .chain(std::iter::once(0))
        .find_map(|k| q[k..].strip_prefix("v="))
}

// ------------------------------------------------------------------ drawing

fn span_style(kind: &SpanKind) -> HighlightStyle {
    let color = match kind {
        SpanKind::Mention => c::AMBER,
        SpanKind::MentionSelf => c::CYAN,
        SpanKind::Link(_) => theme::ACCENT,
    };
    HighlightStyle {
        color: Some(color.into()),
        ..Default::default()
    }
}

fn tok_style(tok: Tok) -> HighlightStyle {
    let color = match tok {
        Tok::Key => theme::CODE_KEY,
        Tok::Str => theme::CODE_STR,
        Tok::Num => theme::CODE_NUM,
        Tok::Com => c::SILVER,
    };
    HighlightStyle {
        color: Some(color.into()),
        ..Default::default()
    }
}

/// The names a mention may match: mine, then every peer whose identity we know.
pub fn mention_names(snap: &AppSnapshot) -> Vec<String> {
    std::iter::once(snap.s.me.name.clone())
        .chain(snap.known_peers().map(|p| p.name.clone()))
        .filter(|n| !n.is_empty())
        .collect()
}

/// The blocks of one body, drawn: `.msg-text` paragraphs and `.code` windows. `key` makes
/// the selectable runs' ids stable across renders (the message's op).
pub fn render_body(snap: &AppSnapshot, key: &SharedString, text: &str, mentions_you: bool) -> Vec<AnyElement> {
    let names = mention_names(snap);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    parse_body(text, &names, &snap.s.me.name, mentions_you)
        .into_iter()
        .enumerate()
        .map(|(i, block)| match block {
            Block::Text { text, spans } => render_text(key, i, text, spans),
            Block::Code { info, code, tokens } => render_code(key, i, info, code, tokens),
        })
        .collect()
}

fn render_text(key: &SharedString, i: usize, text: String, spans: Vec<Span>) -> AnyElement {
    let highlights: Vec<(Range<usize>, HighlightStyle)> = spans.iter().map(|s| (s.range.clone(), span_style(&s.kind))).collect();
    let links = spans
        .iter()
        .filter_map(|s| match &s.kind {
            SpanKind::Link(url) => Some((s.range.clone(), SharedString::from(url.clone()))),
            _ => None,
        })
        .collect();
    let plates = spans
        .iter()
        .filter(|s| matches!(s.kind, SpanKind::Mention | SpanKind::MentionSelf))
        .map(|s| s.range.clone())
        .collect();
    div()
        .max_w(px(TEXT_MEASURE))
        .text_size(px(15.))
        .line_height(relative(1.5))
        .text_color(c::PAPER)
        .child(
            RichText::new(ElementId::NamedInteger(key.clone(), i as u64), text)
                .highlights(highlights)
                .links(links)
                .plates(plates, c::SPACE),
        )
        .into_any_element()
}

/// `.code`: header with the info string and Copy, then the `<pre>` that scrolls sideways.
fn render_code(key: &SharedString, i: usize, info: String, code: String, tokens: Vec<(Range<usize>, Tok)>) -> AnyElement {
    let copy = code.clone();
    let highlights: Vec<(Range<usize>, HighlightStyle)> = tokens.into_iter().map(|(r, t)| (r, tok_style(t))).collect();
    div()
        .mt(px(6.))
        .mb(px(2.))
        .max_w(px(TEXT_MEASURE))
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::BG_CODE)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .px(px(10.))
                .py(px(5.))
                .border_b_1()
                .border_color(theme::BORDER)
                .bg(c::INK)
                .text_size(px(11.))
                .text_color(c::SILVER)
                .child(if info.is_empty() { "code".to_string() } else { info })
                .child(
                    div()
                        .id(ElementId::NamedInteger(SharedString::from(format!("{key}-copy")), i as u64))
                        .ml_auto()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(c::SILVER)
                        .hover(|s| s.text_color(theme::TEXT))
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))
                        .child("Copy"),
                ),
        )
        .child(
            div()
                .id(ElementId::NamedInteger(SharedString::from(format!("{key}-pre")), i as u64))
                .overflow_x_scroll()
                .px(px(12.))
                .py(px(10.))
                .font_family(theme::MONO_FONT)
                .text_size(px(12.5))
                .line_height(relative(1.6))
                .text_color(c::PAPER)
                .whitespace_nowrap()
                .child(RichText::new(ElementId::NamedInteger(key.clone(), i as u64), code).highlights(highlights)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Text { text, .. } => format!("T:{text}"),
                Block::Code { info, code, .. } => format!("C:{info}|{code}"),
            })
            .collect()
    }

    /// Revert to splitting without the empty-part filter and the trailing-newline strip,
    /// and this fails.
    #[test]
    fn fences_split_into_text_and_code() {
        let b = parse_body("kijk:\n```config.toml\na = 1\n```\nklaar", &[], "Rick", false);
        assert_eq!(text_of(&b), ["T:kijk:", "C:config.toml|a = 1", "T:\nklaar"]);
        let b = parse_body("```\nx\n```", &[], "Rick", false);
        assert_eq!(text_of(&b), ["C:|x"]);
        let b = parse_body("```inline", &[], "Rick", false);
        assert_eq!(text_of(&b), ["C:|inline"]);
    }

    #[test]
    fn whitespace_collapses_like_html() {
        assert_eq!(collapse_whitespace("  a   b \n  c\n"), "a b\nc");
    }

    /// Mentions win over links (B-49); only existing names; `\b` after the name; mine only
    /// lights cyan when the message mentions me.
    #[test]
    fn mentions_and_links() {
        let names = ["Rick", "Sander", "Joost"];
        let s = inline_spans("@Sander @joost hoi @Sanders https://a.nl/x@Rick y", &names, "Rick", false);
        let got: Vec<(&str, &SpanKind)> = s
            .iter()
            .map(|s| (&"@Sander @joost hoi @Sanders https://a.nl/x@Rick y"[s.range.clone()], &s.kind))
            .collect();
        assert_eq!(
            got,
            [
                ("@Sander", &SpanKind::Mention),
                ("@joost", &SpanKind::Mention),
                ("https://a.nl/x", &SpanKind::Link("https://a.nl/x".into())),
                ("@Rick", &SpanKind::Mention),
            ]
        );
        let s = inline_spans("@Rick kijk", &names, "Rick", true);
        assert_eq!(s[0].kind, SpanKind::MentionSelf);
    }

    /// The baseline bug (shot 03): `codec = "h264"` came out as `0h2641`. Here the string
    /// keeps its text and its hue. Revert the string branch to after the number branch and
    /// the `264` would be a number inside the string.
    #[test]
    fn highlight_keeps_strings_whole() {
        let code = "# hoger 12\nbitrate_kbps = 12000\ncodec = \"h264\" # x1\nv = 1.5.\nid12 = a12b";
        let toks: Vec<(&str, Tok)> = highlight(code).into_iter().map(|(r, t)| (&code[r], t)).collect();
        assert_eq!(
            toks,
            [
                ("# hoger 12", Tok::Com),
                ("bitrate_kbps", Tok::Key),
                ("12000", Tok::Num),
                ("codec", Tok::Key),
                ("\"h264\"", Tok::Str),
                ("# x1", Tok::Com),
                ("v", Tok::Key),
                ("1.5", Tok::Num),
                ("id12", Tok::Key),
            ]
        );
    }

    #[test]
    fn youtube_ids_follow_the_old_regex() {
        assert_eq!(yt_ids("kijk https://www.youtube.com/watch?v=M7lc1UVf-VE nu"), ["M7lc1UVf-VE"]);
        assert_eq!(
            yt_ids("https://youtu.be/aaaaaaaaaaa https://youtube.com/watch?t=1&v=bbbbbbbbbbb https://youtu.be/aaaaaaaaaaa https://m.youtube.com/shorts/ccccccccccc"),
            ["aaaaaaaaaaa", "bbbbbbbbbbb"]
        );
        assert!(yt_ids("https://youtu.be/short").is_empty());
        assert!(yt_ids("https://example.com/watch?v=aaaaaaaaaaa").is_empty());
    }
}
