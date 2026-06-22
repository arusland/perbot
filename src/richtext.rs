//! Renders the surviving reminder body as an HTML fragment, re-mapping the
//! original message's Telegram formatting entities onto the leftover text.
//!
//! Telegram delivers formatting as [`MessageEntity`] offsets over the *original*
//! message text, but [`crate::parser`] strips the time tokens (sometimes from
//! the middle of the text) and normalizes whitespace. [`render_html`] reproduces
//! that normalization while tracking each surviving character's original byte
//! offset, then rebuilds entities over the leftover body and feeds them to
//! teloxide's [`Renderer`].

use std::ops::Range;

use teloxide::types::{MessageEntity, MessageEntityRef};
use teloxide::utils::html;
use teloxide::utils::render::Renderer;

/// One character of the normalized output body, with the UTF-16 offset at which
/// it starts and the byte offset of the source character in the original input.
///
/// For the single spaces normalization inserts between words, `orig` is the byte
/// offset of the *first* original whitespace char that run replaced — so a
/// formatting entity that spanned the original gap still covers the join space
/// (otherwise a single bold run would split at every word boundary).
struct OutChar {
    utf16_start: usize,
    orig: Option<usize>,
    utf16_len: usize,
}

/// Renders the message body (the bytes of `input` selected by `spans`, from
/// [`crate::parser::parse_full`]) as an HTML fragment, applying the original
/// message's `entities`.
///
/// The plain text of the result equals the parser's normalized `message`
/// (`split_whitespace().join(" ")`). With no applicable entity it is just the
/// HTML-escaped message, so the result is always safe to embed in
/// `ParseMode::Html` output.
pub fn render_html(input: &str, spans: &[Range<usize>], entities: &[MessageEntityRef]) -> String {
    let (text, chars) = normalize(input, spans);

    // Rebuild entities over the output body: for every original entity, emit one
    // synthetic `MessageEntity` per maximal run of output chars whose source
    // offset falls inside the entity. Normalization may split one entity into
    // several runs; teloxide's renderer handles the rest (nesting, escaping).
    let mut rebuilt: Vec<MessageEntity> = Vec::new();
    for entity in entities {
        let range = entity.range();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].orig.is_some_and(|o| range.contains(&o)) {
                i += 1;
                continue;
            }
            let offset = chars[i].utf16_start;
            let mut length = 0;
            while i < chars.len() && chars[i].orig.is_some_and(|o| range.contains(&o)) {
                length += chars[i].utf16_len;
                i += 1;
            }
            rebuilt.push(MessageEntity {
                kind: entity.kind().clone(),
                offset,
                length,
            });
        }
    }

    // The renderer's empty-tags fast path returns the text unescaped, so escape
    // ourselves when nothing applies.
    if rebuilt.is_empty() {
        html::escape(&text)
    } else {
        Renderer::new(&text, &rebuilt).as_html()
    }
}

/// Reproduces the parser's `split_whitespace().join(" ")` over the surviving
/// `spans`, returning the normalized text plus a per-character map back to the
/// original byte offsets.
fn normalize(input: &str, spans: &[Range<usize>]) -> (String, Vec<OutChar>) {
    let mut text = String::new();
    let mut chars: Vec<OutChar> = Vec::new();
    let mut utf16 = 0usize;
    let mut pending_space: Option<usize> = None; // first whitespace byte of the run
    let mut seen_word = false;

    for span in spans {
        let mut byte = span.start;
        for ch in input[span.clone()].chars() {
            if ch.is_whitespace() {
                if seen_word && pending_space.is_none() {
                    pending_space = Some(byte);
                }
            } else {
                if let Some(orig) = pending_space.take() {
                    let len = ' '.len_utf16();
                    chars.push(OutChar {
                        utf16_start: utf16,
                        orig: Some(orig),
                        utf16_len: len,
                    });
                    text.push(' ');
                    utf16 += len;
                }
                let len = ch.len_utf16();
                chars.push(OutChar {
                    utf16_start: utf16,
                    orig: Some(byte),
                    utf16_len: len,
                });
                text.push(ch);
                utf16 += len;
                seen_word = true;
            }
            byte += ch.len_utf8();
        }
    }

    (text, chars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    /// Builds entity refs over `input` from raw (UTF-16) entities.
    fn refs(input: &str, entities: Vec<MessageEntity>) -> Vec<MessageEntityRef<'_>> {
        // Leak the entities so the borrowed refs can be returned for the test.
        let leaked: &'static [MessageEntity] = Box::leak(entities.into_boxed_slice());
        MessageEntityRef::parse(input, leaked)
    }

    #[test]
    fn bold_in_body_is_preserved() {
        let input = "13:30 call the office";
        let (_event, spans) = parser::parse_full(input).unwrap();
        // "call" starts at UTF-16 offset 6, length 4.
        let ents = refs(input, vec![MessageEntity::bold(6, 4)]);
        assert_eq!(render_html(input, &spans, &ents), "<b>call</b> the office");
    }

    #[test]
    fn formatting_overlapping_removed_token_is_clipped() {
        // Bold spans the whole line including the leading "13:30 "; only the
        // surviving "call the office" stays bold.
        let input = "13:30 call the office";
        let (_event, spans) = parser::parse_full(input).unwrap();
        let ents = refs(input, vec![MessageEntity::bold(0, 21)]);
        assert_eq!(render_html(input, &spans, &ents), "<b>call the office</b>");
    }

    #[test]
    fn special_chars_inside_a_formatted_run_are_escaped() {
        // "a<b" is bold; the `<` must be escaped *inside* the tags.
        let input = "9:00 a<b";
        let (event, spans) = parser::parse_full(input).unwrap();
        assert_eq!(event.message, "a<b");
        // "a<b" starts at UTF-16 offset 5, length 3.
        let ents = refs(input, vec![MessageEntity::bold(5, 3)]);
        assert_eq!(render_html(input, &spans, &ents), "<b>a&lt;b</b>");
    }

    #[test]
    fn plain_message_is_html_escaped() {
        // No entities: special characters must be escaped, not passed through.
        let input = "a < b & c";
        let spans: Vec<Range<usize>> = std::iter::once(0..input.len()).collect();
        assert_eq!(render_html(input, &spans, &[]), "a &lt; b &amp; c");
    }

    #[test]
    fn output_text_equals_normalized_message() {
        let input = "13:30 call   the office";
        let (event, spans) = parser::parse_full(input).unwrap();
        // With no entities the fragment is exactly the (escaped) plain message.
        assert_eq!(render_html(input, &spans, &[]), event.message);
        assert_eq!(event.message, "call the office");
    }
}
