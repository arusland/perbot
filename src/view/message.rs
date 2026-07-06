//! Text primitives shared by the renderers — datetime/relative formatting,
//! HTML-fragment stripping and previews, the Telegram length clamp — plus the
//! one-line composed replies `main` sends.

use crate::locale::LocaleProvider;
use chrono::NaiveDateTime;
use teloxide::utils::html;

/// Short relative time until `dt` from `now`, delegated to the locale (e.g.
/// `13 mins`, `1h`, `2d`, `1.4y`).
fn format_relative(now: NaiveDateTime, dt: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    loc.format_relative((dt - now).num_seconds())
}

/// Plain-text "HH:MM dd.mm.yyyy (relative)" for a single datetime, e.g.
/// `14:00 23.06.2026 (1d)`. Unescaped — for the fired-reminder preview. List
/// replies use `write_event_row` (HTML) instead.
pub fn format_when(now: NaiveDateTime, dt: NaiveDateTime, loc: &dyn LocaleProvider) -> String {
    format!(
        "{} ({})",
        loc.format_datetime(dt),
        format_relative(now, dt, loc)
    )
}

/// Max characters of message shown in the two-line `/events` row before it is
/// truncated with a trailing `...`.
pub(super) const MESSAGE_PREVIEW_MAX: usize = 50;

/// Plain-text, newline-free rendering of an HTML message fragment: strips HTML
/// tags, unescapes the three specials `teloxide::utils::html::escape` emits
/// (`&amp; &lt; &gt;`), and collapses all whitespace (incl. newlines) to single
/// spaces. The result is plain text; callers targeting HTML must escape it.
pub(super) fn html_to_plain(html_fragment: &str) -> String {
    // Strip tags: drop everything between '<' and the next '>'.
    let mut stripped = String::with_capacity(html_fragment.len());
    let mut in_tag = false;
    for c in html_fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => stripped.push(c),
            _ => {}
        }
    }
    // Unescape: do `&lt;`/`&gt;` before `&amp;` so an escaped `&` is not turned
    // into the start of another entity.
    let unescaped = stripped
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // Collapse all whitespace (incl. newlines) to single spaces; trim ends.
    unescaped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Plain-text, newline-free preview of an HTML message fragment, truncated to
/// `max` characters (chars, not bytes) with a trailing `...` when longer.
/// The result is plain text; callers targeting HTML must escape it.
pub(super) fn message_preview(html_fragment: &str, max: usize) -> String {
    let collapsed = html_to_plain(html_fragment);
    // Truncate by char count for UTF-8 safety.
    if collapsed.chars().count() > max {
        let head: String = collapsed.chars().take(max).collect();
        format!("{head}...")
    } else {
        collapsed
    }
}

/// Telegram's hard message limit, in UTF-16 code units, measured on the rendered
/// text (after entities parsing — HTML tags don't count toward it).
pub const TELEGRAM_MAX_LEN: usize = 4096;

/// Headroom reserved below [`TELEGRAM_MAX_LEN`] for the bits the bot appends to a
/// user's body when it fires a reminder: the `Next launches:` preview (up to a
/// few launch lines + header) and the snooze hint, plus the one-off
/// confirmation's labels. The realistic worst case is ~150 units; 300 is a safe
/// margin so a clamped body never makes an outbound message exceed the limit.
const FIRED_EXTRAS_RESERVE: usize = 300;

/// Maximum rendered length (UTF-16 code units) of a user-supplied reminder body,
/// leaving [`FIRED_EXTRAS_RESERVE`] for the appended preview/hint.
pub const MESSAGE_MAX_LEN: usize = TELEGRAM_MAX_LEN - FIRED_EXTRAS_RESERVE;

/// Rendered length Telegram counts for an HTML fragment: its plain text (tags
/// stripped, specials unescaped) measured in UTF-16 code units — the same unit
/// the Bot API uses for the 4096-char limit.
pub fn rendered_len(html_fragment: &str) -> usize {
    html_to_plain(html_fragment).encode_utf16().count()
}

/// Clamps an HTML message fragment to [`MESSAGE_MAX_LEN`] rendered UTF-16 units.
/// Returns `(fragment, false)` unchanged when it already fits; otherwise returns
/// `(escaped_truncated_plain_text, true)`. Over-limit truncation falls back to
/// the plain (un-formatted) text re-escaped, which is always valid HTML — losing
/// formatting only in this rare case, where the caller warns the user anyway.
pub fn clamp_message(html_fragment: &str) -> (String, bool) {
    if rendered_len(html_fragment) <= MESSAGE_MAX_LEN {
        return (html_fragment.to_owned(), false);
    }
    let plain = html_to_plain(html_fragment);
    // Truncate by whole chars, accumulating UTF-16 widths so a surrogate pair is
    // never split and the head never exceeds the cap.
    let mut head = String::with_capacity(plain.len());
    let mut units = 0usize;
    for c in plain.chars() {
        let w = c.len_utf16();
        if units + w > MESSAGE_MAX_LEN {
            break;
        }
        head.push(c);
        units += w;
    }
    (html::escape(&head), true)
}

/// Warning shown when a submitted reminder body exceeded Telegram's length limit
/// and was shortened to fit (see [`clamp_message`]).
pub const MESSAGE_TRUNCATED: &str =
    "⚠️ Your message was too long and has been shortened to fit Telegram's limit.";

/// Reply confirming a stored event that has no upcoming launch (its datetime is
/// already in the past): the original input echoed back in bold, HTML-escaped.
pub fn inactive_event_reply(text: &str) -> String {
    format!("<b>{}</b>", html::escape(text))
}

/// Reply for a message that didn't parse into a time expression: the input
/// echoed back in bold, HTML-escaped.
pub fn unparsable_message(text: &str) -> String {
    format!("Unparsable message: <b>{}</b>", html::escape(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::EN;
    use chrono::{Duration, NaiveDateTime};

    fn at(now: NaiveDateTime, d: Duration) -> String {
        format_relative(now, now + d, &EN)
    }

    #[test]
    fn rendered_len_ignores_tags_and_counts_utf16() {
        // Tags don't count; only the two rendered letters do.
        assert_eq!(rendered_len("<b>hi</b>"), 2);
        // An astral-plane emoji is one char but two UTF-16 code units.
        assert_eq!(rendered_len("😀"), 2);
        // Escaped specials render back to a single visible char.
        assert_eq!(rendered_len("a &amp; b"), 5);
    }

    #[test]
    fn clamp_message_leaves_short_fragment_unchanged() {
        let (out, truncated) = clamp_message("<b>hi</b> there");
        assert_eq!(out, "<b>hi</b> there");
        assert!(!truncated);
    }

    #[test]
    fn clamp_message_truncates_over_limit_to_valid_html() {
        let long = "a".repeat(MESSAGE_MAX_LEN + 50);
        let (out, truncated) = clamp_message(&long);
        assert!(truncated);
        assert_eq!(rendered_len(&out), MESSAGE_MAX_LEN);
        // Escaped plain text carries no dangling tag.
        assert!(!out.contains('<'));
    }

    #[test]
    fn clamp_message_never_splits_a_surrogate_pair() {
        // A run of astral-plane emoji (2 UTF-16 units each) overruns the cap;
        // the head must stop on a whole-emoji boundary and never exceed it.
        let long = "😀".repeat(MESSAGE_MAX_LEN);
        let (out, truncated) = clamp_message(&long);
        assert!(truncated);
        assert!(rendered_len(&out) <= MESSAGE_MAX_LEN);
        // MESSAGE_MAX_LEN is even, so an exact fill of 2-unit chars lands on it.
        assert_eq!(rendered_len(&out), MESSAGE_MAX_LEN);
    }

    #[test]
    fn message_preview_strips_tags_and_unescapes() {
        assert_eq!(
            message_preview("<b>call</b> the office", 50),
            "call the office"
        );
        assert_eq!(message_preview("<a href=\"x\">site</a>", 50), "site");
        assert_eq!(message_preview("a &amp; b", 50), "a & b");
        assert_eq!(message_preview("&lt;tag&gt;", 50), "<tag>");
    }

    #[test]
    fn message_preview_removes_newlines() {
        assert_eq!(message_preview("line1\nline2", 50), "line1 line2");
        assert_eq!(message_preview("a\n\n  b\tc", 50), "a b c");
    }

    #[test]
    fn message_preview_truncates_by_chars() {
        // 30 chars -> first 20 + "...".
        let msg = "abcdefghijklmnopqrstuvwxyz1234";
        assert_eq!(message_preview(msg, 20), "abcdefghijklmnopqrst...");
        // Short message left intact.
        assert_eq!(message_preview("short", 20), "short");
        // Exactly 20 chars: no ellipsis.
        assert_eq!(
            message_preview("01234567890123456789", 20),
            "01234567890123456789"
        );
    }

    #[test]
    fn message_preview_truncation_is_utf8_safe() {
        // 21 multi-byte chars; truncating by bytes would panic, by chars is fine.
        let msg = "ñññññññññññññññññññññ";
        let out = message_preview(msg, 20);
        assert_eq!(out.chars().count(), 23); // 20 + "..."
        assert!(out.ends_with("..."));
    }

    #[test]
    fn relative_time_units() {
        let now =
            NaiveDateTime::parse_from_str("2026-06-15 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(at(now, Duration::seconds(30)), "soon");
        assert_eq!(at(now, Duration::seconds(-5)), "soon");
        assert_eq!(at(now, Duration::seconds(31)), "1 min");
        assert_eq!(at(now, Duration::minutes(1)), "1 min");
        assert_eq!(at(now, Duration::seconds(90)), "1 min"); // exactly half stays
        assert_eq!(at(now, Duration::seconds(91)), "2 mins");
        assert_eq!(at(now, Duration::minutes(13)), "13 mins");
        assert_eq!(at(now, Duration::seconds(59 * 60 + 31)), "1h"); // rounds through 60 min
        assert_eq!(at(now, Duration::hours(1)), "1h");
        assert_eq!(at(now, Duration::minutes(90)), "90m");
        assert_eq!(at(now, Duration::minutes(110)), "2h");
        assert_eq!(at(now, Duration::hours(23)), "23h");
        assert_eq!(at(now, Duration::minutes(23 * 60 + 30)), "23h"); // exactly half stays
        assert_eq!(at(now, Duration::minutes(23 * 60 + 31)), "1d");
        assert_eq!(at(now, Duration::days(2)), "2d");
        assert_eq!(at(now, Duration::hours(36)), "1d"); // exactly half stays
        assert_eq!(at(now, Duration::hours(37)), "2d");
        assert_eq!(at(now, Duration::hours(6 * 24 + 13)), "1w"); // rounds through 7 days
        assert_eq!(at(now, Duration::days(7)), "1w");
        assert_eq!(at(now, Duration::days(10)), "10d");
        assert_eq!(at(now, Duration::days(11)), "11d");
        assert_eq!(at(now, Duration::days(14)), "2w");
        assert_eq!(at(now, Duration::days(21)), "3w");
        assert_eq!(at(now, Duration::days(51 * 7)), "51w"); // just under a year
        assert_eq!(at(now, Duration::days(52 * 7)), "1y"); // 364 days
        assert_eq!(at(now, Duration::days(511)), "1.4y");
        assert_eq!(at(now, Duration::days(693)), "1.9y");
        assert_eq!(at(now, Duration::days(104 * 7)), "2y"); // 728 days
    }
}
