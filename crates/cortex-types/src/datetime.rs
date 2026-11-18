//! The one `date-time` rule (cox W1-39), and the conversions around it. Cells: `tests/w1_datetime.rs` over the
//! shared vectors `api/rfc3339-vectors.json` (14 to accept, 48 to reject).
//!
//! **The rule:** `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)`, upper-case `T` and `Z`, a real calendar date, year
//! 1970 to 9999, hour 0-23, minute 0-59, second 0-59 (no leap second), offset minutes 0-59 and the offset within
//! `-12:00 .. +14:00` (the real range; Postgres itself stops at 15). NUL or any other character anywhere is a reject.
//! Nothing is normalised: a value outside the real range is a client error (422 `invalid_request`), and refusing it
//! keeps the store free of parsing and of a database error (SQLSTATE 22007, 22008, 22009) answering as a 500.
//!
//! No date library: the civil-from-days conversions are a dozen lines each (rung 5).

use crate::time::Timestamp;

/// Why a text is not an accepted `date-time`. Carries nothing of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a valid date-time")]
pub struct DateTimeError;

/// The parts of an accepted text.
struct Parts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    /// Fractional seconds in microseconds (digits past the sixth are cut).
    micros: i64,
    /// Minutes east of UTC.
    offset_minutes: i64,
}

/// A run of ASCII digits as a number; anything else (a sign, a space, a non-ASCII byte) is `None`.
fn digits(b: &[u8]) -> Option<i64> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(b.iter().fold(0_i64, |n, d| n * 10 + i64::from(d - b'0')))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

fn parts(s: &str) -> Result<Parts, DateTimeError> {
    let b = s.as_bytes();
    // "YYYY-MM-DDTHH:MM:SSZ" is 20 bytes; the longest form is 19 + ".fffffffff" + "+HH:MM" = 35.
    if !(20..=35).contains(&b.len())
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(DateTimeError);
    }
    let n = |from: usize, to: usize| digits(&b[from..to]).ok_or(DateTimeError);
    let (year, month, day) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (hour, minute, second) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let mut i = 19;
    let mut micros = 0;
    if b[i] == b'.' {
        let start = i + 1;
        let mut j = start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if !(1..=9).contains(&(j - start)) {
            return Err(DateTimeError);
        }
        // The first six digits, padded to six: the microseconds.
        let mut six = [b'0'; 6];
        for (slot, d) in six.iter_mut().zip(&b[start..j]) {
            *slot = *d;
        }
        micros = digits(&six).ok_or(DateTimeError)?;
        i = j;
    }
    let offset_minutes = match &b[i..] {
        b"Z" => 0,
        rest if rest.len() == 6 && (rest[0] == b'+' || rest[0] == b'-') && rest[3] == b':' => {
            let hh = digits(&rest[1..3]).ok_or(DateTimeError)?;
            let mm = digits(&rest[4..6]).ok_or(DateTimeError)?;
            if mm > 59 {
                return Err(DateTimeError);
            }
            let total = hh * 60 + mm;
            if rest[0] == b'+' {
                if total > 14 * 60 {
                    return Err(DateTimeError);
                }
                total
            } else {
                if total > 12 * 60 {
                    return Err(DateTimeError);
                }
                -total
            }
        }
        _ => return Err(DateTimeError),
    };
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(DateTimeError);
    }
    Ok(Parts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        micros,
        offset_minutes,
    })
}

/// Howard Hinnant's days_from_civil: the day number (1970-01-01 = 0) of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether `s` obeys the rule. A reject is a client error (422 `invalid_request`), never a database error.
pub fn validate(s: &str) -> Result<(), DateTimeError> {
    parts(s).map(|_| ())
}

/// The instant `s` names, in UTC microseconds, after the same checks as [`validate`]. Digits past the sixth
/// fractional digit are cut.
///
/// **Round-trip limit (stated; cells `w1_dt_6`, `w1_dt_7`):** the year domain 1970 to 9999 is read from the text's
/// own fields in its own offset. A valid local text on the edge (`1970-01-01T00:00:00+01:00`,
/// `9999-12-31T23:59:59-01:00`) is ACCEPTED and names an instant whose UTC date is outside that domain; the
/// instant is exact and Postgres holds it. Such a text is not rejected.
pub fn parse(s: &str) -> Result<Timestamp, DateTimeError> {
    let p = parts(s)?;
    let secs =
        days_from_civil(p.year, p.month, p.day) * 86_400 + p.hour * 3600 + p.minute * 60 + p.second
            - p.offset_minutes * 60;
    Ok(Timestamp::from_unix_micros(secs * 1_000_000 + p.micros))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, UTC, rounded down to the second. The same text the chain puts in
/// `Cortex-Token-Expires`.
///
/// For an instant whose UTC date is inside 1970..9999, the text is accepted by [`validate`] and `parse` gives the
/// instant back, rounded down to the second. For an instant outside (reachable only from a valid local text on the
/// edge, see [`parse`]) the text is written truthfully (`1969-12-31T23:00:00Z`, `10000-01-01T00:59:59Z`) and is not
/// a text `validate` accepts.
pub fn format(t: Timestamp) -> String {
    let secs = t.unix_micros().div_euclid(1_000_000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hour, minute, second) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // Howard Hinnant's civil_from_days.
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
