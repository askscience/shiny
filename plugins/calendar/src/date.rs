//! Date & time normalization for the calendar plugin.
//!
//! Dates are stored as "YYYY-MM-DD", times as "HH:MM" (24-hour, empty string
//! means all-day/unspecified). The helpers here accept the friendly spellings
//! ("today", "tomorrow", "9:30am") the LLM or window may produce and normalize
//! them into the canonical stored form.

use chrono::{Datelike, Days, Local, NaiveDate};

/// Today's date as "YYYY-MM-DD".
pub fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

/// The current month as "YYYY-MM".
pub fn current_month() -> String {
    Local::now().date_naive().format("%Y-%m").to_string()
}

/// Normalize "today" / "tomorrow" / "yesterday" / ISO dates to "YYYY-MM-DD".
pub fn normalize_date(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("date required — use YYYY-MM-DD (or \"today\"/\"tomorrow\")".into());
    }
    let lower = t.to_lowercase();
    let base = Local::now().date_naive();
    let date = match lower.as_str() {
        "today" => base,
        "tomorrow" => base.checked_add_days(Days::new(1)).ok_or("date out of range")?,
        "yesterday" => base.checked_sub_days(Days::new(1)).ok_or("date out of range")?,
        _ => {
            let compact = t.replace('/', "-");
            NaiveDate::parse_from_str(&compact, "%Y-%m-%d")
                .or_else(|_| NaiveDate::parse_from_str(&compact, "%Y%m%d"))
                .map_err(|_| {
                    format!("invalid date \"{s}\" — use YYYY-MM-DD (or \"today\"/\"tomorrow\")")
                })?
        }
    };
    Ok(date.format("%Y-%m-%d").to_string())
}

/// Normalize a time to "HH:MM" (24-hour). Empty input → empty (all-day).
/// Accepts "9", "9:30", "14:00", "9:30am", "5pm", "12:00 am", etc.
pub fn normalize_time(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(String::new());
    }

    let lower = t.to_lowercase();
    let mut body = lower;
    let mut is_pm: Option<bool> = None;
    for (suffix, pm) in [
        ("p.m.", true),
        ("a.m.", false),
        ("pm", true),
        ("am", false),
    ] {
        if let Some(stripped) = body.strip_suffix(suffix) {
            is_pm = Some(pm);
            body = stripped.trim().to_string();
            break;
        }
    }

    let parts: Vec<&str> = body.split(':').collect();
    // "08:15:00" — the seconds a model copies from an ISO timestamp.
    let parts = if parts.len() == 3 && parts[2].parse::<u32>().is_ok_and(|s| s < 60) {
        parts[..2].to_vec()
    } else {
        parts
    };
    let (mut h, m) = match parts.len() {
        1 => {
            let h: u32 = parts[0].parse().map_err(|_| format!("invalid time \"{s}\""))?;
            (h, 0u32)
        }
        2 => {
            let h: u32 = parts[0].parse().map_err(|_| format!("invalid time \"{s}\""))?;
            let m: u32 = parts[1].parse().map_err(|_| format!("invalid time \"{s}\""))?;
            (h, m)
        }
        _ => return Err(format!("invalid time \"{s}\" — use HH:MM")),
    };

    if let Some(pm) = is_pm {
        if pm {
            if h < 12 {
                h += 12;
            }
        } else if h == 12 {
            h = 0;
        }
    }

    if h > 23 || m > 59 {
        return Err(format!("invalid time \"{s}\" — use HH:MM (24-hour)"));
    }
    Ok(format!("{h:02}:{m:02}"))
}

/// First and last day of a "YYYY-MM" month, as (from, to) "YYYY-MM-DD" pairs.
/// A date-and-time a model may send in one field: `2026-11-20T08:15`,
/// `2026-11-20T08:15:00Z`, `2026-11-20 08:15:00+01:00`, or a bare date.
/// Returns `(YYYY-MM-DD, HH:MM or "")`; `None` when no date leads the text.
pub fn split_datetime(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    if t.len() < 10 {
        return None;
    }
    let date = normalize_date(&t[..10]).ok()?;
    let rest = t[10..].trim_start_matches(['T', 't', ' ']);
    // Drop a zone suffix: the event is stored in local wall-clock time.
    let clock: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let time = if clock.is_empty() { String::new() } else { normalize_time(&clock).ok()? };
    Some((date, time))
}

pub fn month_bounds(month: &str) -> Result<(String, String), String> {
    let m = month.trim();
    let first = NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d")
        .map_err(|_| format!("invalid month \"{month}\" — use YYYY-MM"))?;
    let (y, mon) = (first.year(), first.month());
    let (ny, nm) = if mon == 12 { (y + 1, 1) } else { (y, mon + 1) };
    let next = NaiveDate::from_ymd_opt(ny, nm, 1).ok_or("invalid month")?;
    let last = next.checked_sub_days(Days::new(1)).ok_or("invalid month")?;
    Ok((
        first.format("%Y-%m-%d").to_string(),
        last.format("%Y-%m-%d").to_string(),
    ))
}

/// The times to store for an event: an all-day event keeps no start or end
/// time, whatever was passed.
pub fn event_times(all_day: bool, start: String, end: String) -> (String, String) {
    if all_day {
        (String::new(), String::new())
    } else {
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_day_events_drop_their_times() {
        assert_eq!(event_times(true, "09:00".into(), "10:00".into()), (String::new(), String::new()));
        assert_eq!(event_times(false, "09:00".into(), "10:00".into()), ("09:00".into(), "10:00".into()));
    }

    #[test]
    fn dates() {
        assert_eq!(normalize_date("2026-08-14").unwrap(), "2026-08-14");
        assert_eq!(normalize_date("2026/8/4").unwrap(), "2026-08-04");
        assert_eq!(normalize_date("today").unwrap(), today());
    }

    #[test]
    fn datetimes_in_one_field() {
        assert_eq!(split_datetime("2026-11-20T08:15:00"), Some(("2026-11-20".into(), "08:15".into())));
        assert_eq!(split_datetime("2026-11-20T08:15:00Z"), Some(("2026-11-20".into(), "08:15".into())));
        assert_eq!(split_datetime("2026-11-20 21:30+01:00"), Some(("2026-11-20".into(), "21:30".into())));
        assert_eq!(split_datetime("2026-11-20"), Some(("2026-11-20".into(), String::new())));
        assert_eq!(split_datetime("08:15"), None);
        assert_eq!(normalize_time("08:15:00").unwrap(), "08:15");
    }

    #[test]
    fn times() {
        assert_eq!(normalize_time("9:30").unwrap(), "09:30");
        assert_eq!(normalize_time("9:30am").unwrap(), "09:30");
        assert_eq!(normalize_time("5pm").unwrap(), "17:00");
        assert_eq!(normalize_time("12:00 am").unwrap(), "00:00");
        assert_eq!(normalize_time("").unwrap(), "");
        assert!(normalize_time("25:00").is_err());
    }

    #[test]
    fn months() {
        assert_eq!(month_bounds("2026-02").unwrap(), ("2026-02-01".into(), "2026-02-28".into()));
        assert_eq!(month_bounds("2024-02").unwrap(), ("2024-02-01".into(), "2024-02-29".into()));
    }
}
