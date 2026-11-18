//! RFC 3339 text for `Cortex-Token-Expires` (S1 §10.1), from a `Timestamp`, to the second, in
//! UTC. No date library: the civil-from-days conversion is a dozen lines (rung 5).
use cortex_types::Timestamp;

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(t: Timestamp) -> String {
    let secs = t.unix_micros().div_euclid(1_000_000);
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
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> String {
        rfc3339(Timestamp::from_unix_micros(secs * 1_000_000 + 999_999))
    }

    #[test]
    fn h05_known_instants() {
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z", "a leap day");
        assert_eq!(at(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(at(4_102_444_799), "2099-12-31T23:59:59Z");
        assert_eq!(
            rfc3339(Timestamp::from_unix_micros(-1)),
            "1969-12-31T23:59:59Z",
            "rounds down"
        );
    }
}
