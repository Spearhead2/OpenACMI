//! Minimal UTC time handling for `ReferenceTime` and clock displays.
//!
//! Only the ISO 8601 form used by ACMI (`2011-06-02T05:00:00Z`) is needed, so
//! this avoids a date-time dependency.

/// Parses `YYYY-MM-DDThh:mm:ss[.fff][Z]` into seconds since the Unix epoch.
pub fn parse_iso8601(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once(['T', ' ']).unwrap_or((s, "00:00:00"));

    let mut date_parts = date.splitn(3, '-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;

    let mut time_parts = time.splitn(3, ':');
    let hour: f64 = time_parts.next()?.parse().ok()?;
    let minute: f64 = time_parts.next().unwrap_or("0").parse().ok()?;
    let second: f64 = time_parts.next().unwrap_or("0").parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) as f64 * 86_400.0 + hour * 3600.0 + minute * 60.0 + second)
}

/// Formats seconds since the Unix epoch as `YYYY-MM-DD hh:mm:ss` (UTC).
pub fn format_utc(unix_seconds: f64) -> String {
    let total = unix_seconds.floor() as i64;
    let (days, secs) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Formats a duration as `h:mm:ss` (or `m:ss` under an hour); `--:--` if invalid.
pub fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "--:--".to_owned();
    }
    let sign = if seconds < 0.0 { "-" } else { "" };
    let total = seconds.abs().floor() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{sign}{h}:{m:02}:{s:02}")
    } else {
        format!("{sign}{m:02}:{s:02}")
    }
}

// Howard Hinnant's civil-date algorithms (proleptic Gregorian calendar).

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_round_trip() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0.0));
        let t = parse_iso8601("2011-06-02T05:00:00Z").unwrap();
        assert_eq!(t, 1_306_990_800.0);
        assert_eq!(format_utc(t + 3661.0), "2011-06-02 06:01:01");
        assert_eq!(format_utc(parse_iso8601("2024-02-29T23:59:59Z").unwrap()), "2024-02-29 23:59:59");
        assert_eq!(parse_iso8601("not a date"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(75.4), "01:15");
        assert_eq!(format_duration(3725.0), "1:02:05");
        assert_eq!(format_duration(f64::NAN), "--:--");
    }
}
