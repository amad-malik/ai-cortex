//! UTC text from a `Timestamp`, with no date library (the civil-from-days conversion is a dozen
//! lines; the same algorithm as `cortex-http/src/time.rs`, which a piece may not depend on for a
//! helper). Deterministic: the text depends on the timestamp only.

use cortex_types::Timestamp;

/// `(year, month, day, hour, minute, second, microsecond)` in UTC.
fn civil(t: Timestamp) -> (i64, i64, i64, i64, i64, i64, i64) {
    let micros = t.unix_micros();
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // Howard Hinnant's civil_from_days: the proleptic Gregorian date of a day count.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, hour, minute, second, frac)
}

/// `YYYY-MM-DD`: the orientation header's date.
pub fn date(t: Timestamp) -> String {
    let (y, mo, d, ..) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ`: the Markdown's times, to the second.
pub fn seconds(t: Timestamp) -> String {
    let (y, mo, d, h, mi, s, _) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `YYYY-MM-DDTHH:MM:SS.ffffffZ`: the JSON's `date-time`, to the microsecond (lossless).
pub fn micros(t: Timestamp) -> String {
    let (y, mo, d, h, mi, s, f) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{f:06}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_a_leap_day_and_a_negative_fraction() {
        let t = |m: i64| Timestamp::from_unix_micros(m);
        assert_eq!(micros(t(0)), "1970-01-01T00:00:00.000000Z");
        // 2024-02-29T23:59:59.999999Z
        assert_eq!(
            micros(t(1_709_251_199_999_999)),
            "2024-02-29T23:59:59.999999Z"
        );
        assert_eq!(seconds(t(1_709_251_199_999_999)), "2024-02-29T23:59:59Z");
        assert_eq!(date(t(1_709_251_199_999_999)), "2024-02-29");
        assert_eq!(micros(t(-1)), "1969-12-31T23:59:59.999999Z");
    }
}
