//! RFC 3339 for `occurred_at` and `received_at`, without a date library.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Howard Hinnant's civil-date algorithms (public domain).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-10-04T12:30:05.123Z` (UTC, milliseconds), the format the other SDKs send.
pub(crate) fn format_rfc3339(t: SystemTime) -> String {
    let (secs, millis) = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_millis()),
        Err(e) => {
            // Before 1970: floor to whole seconds below.
            let d = e.duration();
            let (s, ms) = (d.as_secs() as i64, d.subsec_millis());
            if ms == 0 {
                (-s, 0)
            } else {
                (-s - 1, 1000 - ms)
            }
        }
    };
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`.
pub(crate) fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    let mut nanos: u32 = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let digits = s.get(start..i)?;
        if digits.is_empty() {
            return None;
        }
        let mut frac = digits.chars().take(9).collect::<String>();
        while frac.len() < 9 {
            frac.push('0');
        }
        nanos = frac.parse().ok()?;
    }
    let offset_secs: i64 = match s.get(i..)? {
        "Z" | "z" => 0,
        tz if tz.len() == 6 && (tz.starts_with('+') || tz.starts_with('-')) && &tz[3..4] == ":" => {
            let oh: i64 = tz[1..3].parse().ok()?;
            let om: i64 = tz[4..6].parse().ok()?;
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    let secs =
        days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60 + se - offset_secs;
    if secs >= 0 {
        Some(UNIX_EPOCH + Duration::new(secs as u64, nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(secs.unsigned_abs()))?
            .checked_add(Duration::from_nanos(u64::from(nanos)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_milliseconds() {
        let t = UNIX_EPOCH + Duration::from_millis(1_791_116_405_123);
        assert_eq!(format_rfc3339(t), "2026-10-04T12:20:05.123Z");
        assert_eq!(format_rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00.000Z"
        );
        assert_eq!(
            format_rfc3339(UNIX_EPOCH - Duration::from_millis(1)),
            "1969-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn parses_what_it_formats_and_offsets() {
        let t = UNIX_EPOCH + Duration::from_millis(1_791_116_405_123);
        assert_eq!(parse_rfc3339(&format_rfc3339(t)), Some(t));
        assert_eq!(parse_rfc3339("2026-10-04T15:20:05.123+03:00"), Some(t));
        assert_eq!(
            parse_rfc3339("2026-10-04T12:20:05Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_791_116_405))
        );
        assert_eq!(
            parse_rfc3339("2026-10-04T12:20:05.123456789Z").map(|x| x.duration_since(t).unwrap()),
            Some(Duration::from_nanos(456_789))
        );
        for bad in [
            "",
            "2026-10-04",
            "2026-13-04T12:20:05Z",
            "2026-10-04T12:20:05",
            "2026-10-04T12:20:05.Z",
            "2026-10-04T12:20:05+3:00",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }
}
