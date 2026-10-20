//! Pure text helpers, ported one for one from `frontend/app.js` (`fmtSize`, `fmtSpan`,
//! `fmtDayNum`, `fmtTime`, `fmtDay`, `lastSeenLine`, `fmtClock`, `fmtRtt`, `volPct`,
//! `mbit`, `statusLabel`, the avatar monogram). Shared by every slice; no GPUI in here.
//!
//! Dates follow what the webview printed in the user's English UI: `22:14`, `3 August`.

use chrono::{DateTime, Datelike, Local, TimeZone};
use std::sync::OnceLock;

/// A frozen "now" for the fixture mode, so "Today" and "Yesterday" come out the way the
/// reference screenshots were taken. Never set in a real run.
static FROZEN_NOW: OnceLock<i64> = OnceLock::new();

/// Debug fixture mode only: every "now" in this module (and `now_ms`) answers `ms` from
/// here on. First call wins.
pub fn freeze_clock(ms: i64) {
    let _ = FROZEN_NOW.set(ms);
}

/// The current time in millis since the epoch: use this rather than the system clock in
/// a view, so the fixture mode can freeze it.
pub fn now_ms() -> i64 {
    FROZEN_NOW
        .get()
        .copied()
        .unwrap_or_else(|| Local::now().timestamp_millis())
}

fn now() -> DateTime<Local> {
    local(now_ms()).unwrap_or_else(Local::now)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// `fmtSize`: `512 B`, `1.5 kB`, `12 MB`.
pub fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    let mut v = bytes as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v < 10.0 {
        // `toFixed(1)` rounds a tie up (3.25 MB is "3.3 MB"); `{:.1}` would round it to even.
        format!("{:.1} {}", (v * 10.0).round() / 10.0, UNITS[i])
    } else {
        format!("{} {}", v.round(), UNITS[i])
    }
}

/// `fmtSpan`: a span of seconds as `6h 12m` / `45m` / `30s`. Recap figures only.
pub fn fmt_span(sec: u64) -> String {
    if sec < 60 {
        return format!("{sec}s");
    }
    let min = (sec as f64 / 60.0).round() as u64;
    if min < 60 {
        return format!("{min}m");
    }
    format!("{}h {:02}m", min / 60, min % 60)
}

/// `fmtDayNum`: a `YYYYMMDD` day number as the engine writes them, spelled out. Empty for 0.
pub fn fmt_day_num(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let month = ((n / 100) % 100) as usize;
    let day = n % 100;
    match MONTHS.get(month.wrapping_sub(1)) {
        Some(m) => format!("{day} {m}"),
        None => String::new(),
    }
}

/// `mbit`: bits per second as whole megabits.
pub fn mbit(bits: u32) -> u32 {
    (bits as f64 / 1_000_000.0).round() as u32
}

/// `volPct`: a 0..1 volume as whole percents, which is what every slider speaks.
pub fn vol_pct(v: f32) -> u32 {
    (v.max(0.0) * 100.0).round() as u32
}

fn local(ms: i64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

/// `fmtTime`: `22:14`, local time.
pub fn fmt_time(ms: i64) -> String {
    local(ms)
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn day_month(d: &DateTime<Local>) -> String {
    format!("{} {}", d.day(), MONTHS[d.month0() as usize])
}

/// `fmtDay`: `Today`, `Yesterday` or `3 August`.
pub fn fmt_day(ms: i64) -> String {
    fmt_day_at(ms, now())
}

fn fmt_day_at(ms: i64, now: DateTime<Local>) -> String {
    let Some(d) = local(ms) else {
        return String::new();
    };
    if d.date_naive() == now.date_naive() {
        return "Today".into();
    }
    if now.date_naive().pred_opt() == Some(d.date_naive()) {
        return "Yesterday".into();
    }
    day_month(&d)
}

/// `lastSeenLine`: only ever from a moment this process observed; without one the plain
/// word, because inventing a time would be worse.
pub fn last_seen_line(last_seen: Option<i64>) -> String {
    last_seen_line_at(last_seen, now())
}

fn last_seen_line_at(last_seen: Option<i64>, now: DateTime<Local>) -> String {
    let Some(ms) = last_seen.filter(|ms| *ms != 0) else {
        return "offline".into();
    };
    let Some(d) = local(ms) else {
        return "offline".into();
    };
    if fmt_day_at(ms, now) == "Today" {
        format!("seen {}", fmt_time(ms))
    } else {
        format!("seen {}, {}", fmt_time(ms), day_month(&d))
    }
}

/// `fmtClock`: a game's seconds as `m:ss`.
pub fn fmt_clock(sec: u32) -> String {
    format!("{}:{:02}", sec / 60, sec % 60)
}

/// `fmtRtt`: `12 ms`, or an em dash when there is no measurement.
pub fn fmt_rtt(rtt: Option<u32>) -> String {
    match rtt {
        Some(ms) => format!("{ms} ms"),
        None => "\u{2014}".into(),
    }
}

/// `toLocaleString()` on a whole number in the English UI: `1,888`.
pub fn fmt_thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `statusLabel`: the label a chosen status gets in rosters and tooltips.
pub fn status_label(status: &str) -> &'static str {
    match status {
        "away" => "Away",
        "busy" => "Busy",
        _ => "",
    }
}

/// The avatar monogram: first character of the trimmed name, uppercased, or `?`.
pub fn monogram(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_the_webview_printed_them() {
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(1536), "1.5 kB");
        assert_eq!(fmt_size(12 * 1024 * 1024), "12 MB");
        assert_eq!(fmt_size(10 * 1024), "10 kB");
        assert_eq!(
            fmt_size(3_407_872),
            "3.3 MB",
            "a tie rounds up, like toFixed"
        );
    }

    #[test]
    fn spans_switch_units_where_app_js_did() {
        assert_eq!(fmt_span(30), "30s");
        assert_eq!(fmt_span(45 * 60), "45m");
        assert_eq!(fmt_span(6 * 3600 + 12 * 60), "6h 12m");
        assert_eq!(fmt_span(3600 + 5 * 60), "1h 05m");
    }

    #[test]
    fn days_and_last_seen() {
        let now = Local.with_ymd_and_hms(2026, 8, 3, 23, 0, 0).unwrap();
        let today = Local
            .with_ymd_and_hms(2026, 8, 3, 22, 14, 0)
            .unwrap()
            .timestamp_millis();
        let yesterday = Local
            .with_ymd_and_hms(2026, 8, 2, 9, 5, 0)
            .unwrap()
            .timestamp_millis();
        let older = Local
            .with_ymd_and_hms(2026, 7, 28, 9, 5, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(fmt_day_at(today, now), "Today");
        assert_eq!(fmt_day_at(yesterday, now), "Yesterday");
        assert_eq!(fmt_day_at(older, now), "28 July");
        assert_eq!(last_seen_line_at(Some(today), now), "seen 22:14");
        assert_eq!(last_seen_line_at(Some(older), now), "seen 09:05, 28 July");
        assert_eq!(last_seen_line_at(None, now), "offline");
        assert_eq!(fmt_day_num(20_260_803), "3 August");
        assert_eq!(fmt_day_num(0), "");
    }

    #[test]
    fn small_figures() {
        assert_eq!(fmt_clock(125), "2:05");
        assert_eq!(fmt_rtt(Some(12)), "12 ms");
        assert_eq!(fmt_rtt(None), "\u{2014}");
        assert_eq!(fmt_thousands(1888), "1,888");
        assert_eq!(fmt_thousands(1_234_567), "1,234,567");
        assert_eq!(vol_pct(0.7), 70);
        assert_eq!(mbit(12_000_000), 12);
        assert_eq!(monogram("  rick"), "R");
        assert_eq!(monogram(""), "?");
        assert_eq!(status_label("busy"), "Busy");
    }
}
