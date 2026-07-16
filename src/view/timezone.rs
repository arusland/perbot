//! The two-step timezone picker (Region → City) and the `/timezone` flow's
//! message texts. Zones come from `chrono_tz::TZ_VARIANTS`, grouped by the
//! nine continental IANA prefixes; deprecated/`Etc/` aliases are not offered
//! (though a stored one still parses). Callback envelope: `tz:r` re-opens the
//! region list, `tz:g:<Region>:<page>` shows a city page, `tz:p:<Zone>` picks —
//! the full IANA name (never an index) rides in the data so a stale keyboard
//! can't mis-pick, and every generated payload stays within Telegram's 64-byte
//! callback-data limit (pinned by a test).

use super::list::NOOP_DATA;
use chrono_tz::{TZ_VARIANTS, Tz};
use std::sync::LazyLock;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use teloxide::utils::html;

/// The continental IANA prefixes offered as picker regions, in display order.
const TZ_REGIONS: &[&str] = &[
    "Africa",
    "America",
    "Antarctica",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
];

/// Cities per page of the second-step keyboard (2 columns × 8 rows).
const TZ_CITY_PAGE_SIZE: usize = 16;

/// Prompt attached to the region keyboard by `/timezone`.
pub const TZ_ASK: &str = "Please choose your timezone:";

/// Reply to a chat that tries to schedule without a configured timezone; the
/// region keyboard rides along.
pub const TZ_REQUIRED: &str = "Please choose your timezone first, then re-send your reminder.";

/// Zones of each region, grouped once: `(region, sorted zones under it)`.
static REGION_ZONES: LazyLock<Vec<(&'static str, Vec<Tz>)>> = LazyLock::new(|| {
    TZ_REGIONS
        .iter()
        .map(|&region| {
            let prefix = format!("{region}/");
            let mut zones: Vec<Tz> = TZ_VARIANTS
                .iter()
                .copied()
                .filter(|tz| tz.name().starts_with(&prefix))
                .collect();
            zones.sort_by_key(|tz| tz.name());
            (region, zones)
        })
        .collect()
});

/// The zones under `region`, or `None` for a name that is not a picker region.
fn region_zones(region: &str) -> Option<&'static [Tz]> {
    REGION_ZONES
        .iter()
        .find(|(name, _)| *name == region)
        .map(|(_, zones)| zones.as_slice())
}

/// City-button label: the zone name after the region prefix, underscores
/// spaced (`America/Argentina/Buenos_Aires` → `Argentina/Buenos Aires`).
fn city_label(tz: Tz, region: &str) -> String {
    tz.name()
        .strip_prefix(region)
        .and_then(|s| s.strip_prefix('/'))
        .unwrap_or(tz.name())
        .replace('_', " ")
}

/// First-step keyboard: the nine regions (two per row, callback
/// `tz:g:<Region>:0`) plus a pinned UTC row (`tz:p:UTC`).
pub fn timezone_regions_keyboard() -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = TZ_REGIONS
        .chunks(2)
        .map(|chunk| {
            chunk
                .iter()
                .map(|region| InlineKeyboardButton::callback(*region, format!("tz:g:{region}:0")))
                .collect()
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback("UTC", "tz:p:UTC")]);
    InlineKeyboardMarkup::new(rows)
}

/// Second-step keyboard: one page of `region`'s zones (two per row, callback
/// `tz:p:<Zone>`), a `◀ Prev` / `<page>/<pages>` / `Next ▶` nav row when the
/// region spans several pages, and a `« Regions` row back to the first step.
/// `None` for an unknown region; an out-of-range page clamps to the last one.
pub fn timezone_cities_keyboard(region: &str, page: usize) -> Option<InlineKeyboardMarkup> {
    let zones = region_zones(region)?;
    let pages = super::list::total_pages(zones.len(), TZ_CITY_PAGE_SIZE);
    let page = page.min(pages - 1);
    let slice = &zones[page * TZ_CITY_PAGE_SIZE..zones.len().min((page + 1) * TZ_CITY_PAGE_SIZE)];

    let mut rows: Vec<Vec<InlineKeyboardButton>> = slice
        .chunks(2)
        .map(|chunk| {
            chunk
                .iter()
                .map(|tz| {
                    InlineKeyboardButton::callback(
                        city_label(*tz, region),
                        format!("tz:p:{}", tz.name()),
                    )
                })
                .collect()
        })
        .collect();

    if pages > 1 {
        let mut nav = Vec::new();
        if page > 0 {
            nav.push(InlineKeyboardButton::callback(
                "◀ Prev",
                format!("tz:g:{region}:{}", page - 1),
            ));
        }
        nav.push(InlineKeyboardButton::callback(
            format!("{}/{pages}", page + 1),
            NOOP_DATA,
        ));
        if page + 1 < pages {
            nav.push(InlineKeyboardButton::callback(
                "Next ▶",
                format!("tz:g:{region}:{}", page + 1),
            ));
        }
        rows.push(nav);
    }
    rows.push(vec![InlineKeyboardButton::callback("« Regions", "tz:r")]);
    Some(InlineKeyboardMarkup::new(rows))
}

/// `/timezone` reply text: the current setting (or that none is set) above the
/// selection prompt. HTML.
pub fn timezone_current_message(current: Option<Tz>) -> String {
    match current {
        Some(tz) => format!(
            "Current timezone: <b>{}</b>\n\n{}",
            html::escape(tz.name()),
            html::escape(TZ_ASK)
        ),
        None => format!("No timezone set.\n\n{}", html::escape(TZ_ASK)),
    }
}

/// Confirmation shown after a pick, mentioning how many upcoming events were
/// re-anchored to the new zone's wall clock. HTML.
pub fn timezone_set_message(tz: Tz, rescheduled: usize) -> String {
    let tail = match rescheduled {
        0 => String::new(),
        1 => "\n1 event was rescheduled to keep its local time.".to_string(),
        n => format!("\n{n} events were rescheduled to keep their local times."),
    };
    format!(
        "🌍 Timezone set to <b>{}</b>.{tail}",
        html::escape(tz.name())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::InlineKeyboardButtonKind::CallbackData;

    fn datas(kb: &InlineKeyboardMarkup) -> Vec<String> {
        kb.inline_keyboard
            .iter()
            .flatten()
            .map(|b| match &b.kind {
                CallbackData(d) => d.clone(),
                _ => panic!("expected callback data"),
            })
            .collect()
    }

    #[test]
    fn every_zone_callback_fits_telegram_limit() {
        // Telegram caps callback_data at 64 bytes; nothing in the codebase
        // checks it at runtime, so pin it across the whole IANA table here.
        for tz in TZ_VARIANTS {
            let data = format!("tz:p:{}", tz.name());
            assert!(data.len() <= 64, "{data} exceeds 64 bytes");
        }
    }

    #[test]
    fn regions_keyboard_lists_regions_and_pins_utc() {
        let kb = timezone_regions_keyboard();
        let all = datas(&kb);
        for region in TZ_REGIONS {
            assert!(all.contains(&format!("tz:g:{region}:0")), "{region}");
        }
        assert_eq!(all.last().unwrap(), "tz:p:UTC");
        // No deprecated prefixes are offered.
        assert!(all.iter().all(|d| !d.contains("Etc")));
    }

    #[test]
    fn cities_keyboard_pages_and_navigates() {
        // America spans multiple pages: page 0 has no Prev, a noop indicator,
        // a Next to page 1, and the Regions row.
        let kb = timezone_cities_keyboard("America", 0).unwrap();
        let all = datas(&kb);
        assert!(all.contains(&"tz:g:America:1".to_string()));
        assert!(!all.contains(&"tz:g:America:0".to_string()));
        assert!(all.contains(&NOOP_DATA.to_string()));
        assert_eq!(all.last().unwrap(), "tz:r");
        // Every pick button carries a full zone name under the region.
        assert!(
            all.iter()
                .filter(|d| d.starts_with("tz:p:"))
                .all(|d| d.starts_with("tz:p:America/"))
        );
        // At most 16 pick buttons per page.
        assert!(all.iter().filter(|d| d.starts_with("tz:p:")).count() <= TZ_CITY_PAGE_SIZE);

        // Page 1 gains a Prev back to page 0.
        let kb = timezone_cities_keyboard("America", 1).unwrap();
        assert!(datas(&kb).contains(&"tz:g:America:0".to_string()));

        // A wildly out-of-range page clamps instead of panicking.
        assert!(timezone_cities_keyboard("America", 9999).is_some());

        // Unknown regions yield nothing.
        assert!(timezone_cities_keyboard("Atlantis", 0).is_none());
    }

    #[test]
    fn city_labels_strip_prefix_and_underscores() {
        assert_eq!(
            city_label(Tz::America__Argentina__Buenos_Aires, "America"),
            "Argentina/Buenos Aires"
        );
        assert_eq!(city_label(Tz::Europe__Berlin, "Europe"), "Berlin");
    }

    #[test]
    fn messages_mention_zone_and_reschedule_count() {
        assert!(timezone_current_message(None).starts_with("No timezone set."));
        assert!(timezone_current_message(Some(Tz::Europe__Berlin)).contains("Europe/Berlin"));
        assert_eq!(
            timezone_set_message(Tz::UTC, 0),
            "🌍 Timezone set to <b>UTC</b>."
        );
        assert!(timezone_set_message(Tz::Asia__Tokyo, 1).contains("1 event was rescheduled"));
        assert!(timezone_set_message(Tz::Asia__Tokyo, 3).contains("3 events were rescheduled"));
    }
}
