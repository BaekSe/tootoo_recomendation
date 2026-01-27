use anyhow::Context;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use std::collections::HashSet;

const KST_OFFSET_SECS: i32 = 9 * 3600;

// If the job runs before this time (KST), treat it as "yesterday's" market date.
// KRX close is ~15:30 KST; we use a slightly conservative cutoff.
const CLOSE_CUTOFF_HOUR_KST: u32 = 16;
const CLOSE_CUTOFF_MINUTE_KST: u32 = 0;

pub fn resolve_as_of_date(
    as_of_date_arg: Option<&str>,
    now_utc: DateTime<Utc>,
) -> anyhow::Result<NaiveDate> {
    if let Some(s) = as_of_date_arg {
        return Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d")?);
    }

    let kst = chrono::FixedOffset::east_opt(KST_OFFSET_SECS).context("invalid KST offset")?;
    let now_kst = now_utc.with_timezone(&kst);

    let cutoff_reached =
        (now_kst.hour(), now_kst.minute()) >= (CLOSE_CUTOFF_HOUR_KST, CLOSE_CUTOFF_MINUTE_KST);
    let mut date = now_kst.date_naive();
    if !cutoff_reached {
        date = date - Duration::days(1);
    }

    // Roll back to previous business day.
    let holidays = configured_holidays();
    while is_weekend(date) || holidays.contains(&date) {
        date = date - Duration::days(1);
    }

    Ok(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn rolls_back_on_weekend() {
        // 2026-01-03 is Saturday.
        let now = Utc.with_ymd_and_hms(2026, 1, 3, 8, 0, 0).unwrap();
        let d = resolve_as_of_date(None, now).unwrap();
        // Before cutoff, base is 2026-01-02 (Friday) and weekend rollback shouldn't change it.
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    }

    #[test]
    fn uses_previous_day_before_cutoff() {
        // 2026-01-05 06:00 UTC = 15:00 KST (<16:00 cutoff)
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 6, 0, 0).unwrap();
        let d = resolve_as_of_date(None, now).unwrap();
        // Rolls back to Sunday, then to Friday.
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    }

    #[test]
    fn uses_same_day_after_cutoff() {
        // 2026-01-05 08:00 UTC = 17:00 KST (>=16:00 cutoff)
        let now = Utc.with_ymd_and_hms(2026, 1, 5, 8, 0, 0).unwrap();
        let d = resolve_as_of_date(None, now).unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 1, 5).unwrap());
    }
}

fn is_weekend(date: NaiveDate) -> bool {
    matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
}

fn configured_holidays() -> HashSet<NaiveDate> {
    // Minimal set of widely observed fixed-date holidays.
    // Extend via KR_MARKET_HOLIDAYS="YYYY-MM-DD,YYYY-MM-DD".
    let mut out = HashSet::new();
    let years = [2024, 2025, 2026, 2027, 2028, 2029, 2030];
    for y in years {
        if let Some(d) = NaiveDate::from_ymd_opt(y, 1, 1) {
            out.insert(d);
        }
        if let Some(d) = NaiveDate::from_ymd_opt(y, 12, 25) {
            out.insert(d);
        }
    }

    if let Ok(s) = std::env::var("KR_MARKET_HOLIDAYS") {
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Ok(d) = NaiveDate::parse_from_str(part, "%Y-%m-%d") {
                out.insert(d);
            }
        }
    }

    out
}
