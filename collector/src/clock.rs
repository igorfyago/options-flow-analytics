//! Session-aware option time: when does an expiry actually die?
//!
//! An SPY option expiring "today" is alive and tradeable until 16:00 New
//! York, and that chain is exactly where a 0DTE desk lives. Whole-day
//! arithmetic (`num_days() >= 1`) called it dead at midnight UTC, so the
//! collector never stored the traded tenor: every session it captured
//! tomorrow's chain, and on Fridays a 3-day weekend contract, while the
//! downstream desk validated and priced 0DTE. These helpers give both the
//! chain filter and the greeks one honest clock.
//!
//! DST is the manual US rule (EDT from the second Sunday of March 07:00 UTC
//! to the first Sunday of November 06:00 UTC), the same convention the desk
//! uses, so the two ends of the pipeline can never disagree about the close.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};

/// The UTC moment `expiry`'s 16:00 New York close happens.
pub fn expiry_close_utc(expiry: NaiveDate) -> DateTime<Utc> {
    let second_sunday_march = nth_weekday(expiry.year(), 3, Weekday::Sun, 2);
    let first_sunday_november = nth_weekday(expiry.year(), 11, Weekday::Sun, 1);
    // EDT (UTC-4) between the boundaries, EST (UTC-5) outside them
    let edt = expiry > second_sunday_march && expiry <= first_sunday_november
        || (expiry == second_sunday_march)   // close is 16:00, well past the 2am switch
        && expiry != first_sunday_november;
    let close_hour = if edt { 20 } else { 21 };
    Utc.from_utc_datetime(&expiry.and_hms_opt(close_hour, 0, 0).unwrap())
}

/// Days (fractional) until `expiry` stops trading, from `now`.
/// Positive while the contract is alive; <= 0 once the close has passed.
pub fn dte_fraction(expiry: NaiveDate, now: DateTime<Utc>) -> f64 {
    (expiry_close_utc(expiry) - now).num_seconds() as f64 / 86_400.0
}

/// Is this expiry still alive - i.e. may the front chain include it?
/// True for a 0DTE contract before the close, false the second after.
pub fn alive(expiry: NaiveDate, now: DateTime<Utc>) -> bool {
    dte_fraction(expiry, now) > 0.0
}

fn nth_weekday(year: i32, month: u32, wd: Weekday, n: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let offset = (7 + wd.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
    first + chrono::Duration::days((offset + (n - 1) * 7) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, hm: (u32, u32)) -> DateTime<Utc> {
        Utc.from_utc_datetime(
            &NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap()
                .and_hms_opt(hm.0, hm.1, 0).unwrap())
    }

    #[test]
    fn zero_dte_lives_until_the_close_and_not_a_second_longer() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(); // July: EDT, close 20:00Z
        assert!(alive(d, at("2026-07-20", (13, 30))));   // the open
        assert!(alive(d, at("2026-07-20", (19, 59))));   // one minute out
        assert!(!alive(d, at("2026-07-20", (20, 1))));   // the bell has rung
        // and the fraction is a real intraday number, not 0 or 1
        let f = dte_fraction(d, at("2026-07-20", (14, 0)));
        assert!(f > 0.2 && f < 0.3, "expected ~0.25 days, got {f}");
    }

    #[test]
    fn midnight_utc_does_not_kill_the_us_afternoon() {
        // 00:30 UTC is 20:30 ET the PREVIOUS evening: tomorrow's date in UTC.
        // Whole-day-UTC math is what silently rolled the chain a session
        // early; the ET close must be the only authority.
        let d = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        assert!(alive(d, at("2026-07-21", (0, 30))));
        assert!(alive(d, at("2026-07-20", (23, 0))));    // still the prior ET day
    }

    #[test]
    fn winter_close_is_2100_utc() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        assert!(alive(d, at("2026-01-15", (20, 30))));   // 15:30 ET in EST
        assert!(!alive(d, at("2026-01-15", (21, 1))));
    }
}
