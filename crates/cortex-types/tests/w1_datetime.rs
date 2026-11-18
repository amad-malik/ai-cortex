//! Wave 1 `wave1/ports3` item 2 (cox W1-39): ONE `date-time` rule for every input of a body, a query or a cursor,
//! and its shared vectors as cells. `api/rfc3339-vectors.json` has 14 values to accept and 48 to reject; every piece
//! that takes a date-time passes both lists with its validator, and then switches to this one.
//!
//! The rule: `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)`, upper-case `T` and `Z`, a real calendar date, year 1970
//! to 9999, hour 0-23, minute 0-59, second 0-59 (no leap second), offset minutes 0-59 and the offset within
//! `-12:00 .. +14:00`; NUL or any other character anywhere is a reject. Not normalised: a value outside the real
//! range is a client error. `parse` also gives the instant (UTC microseconds; digits past the sixth are cut).
//!
//! RED: every function is `todo!()`.
use cortex_types::Timestamp;
use cortex_types::datetime::{format, parse, validate};

fn vectors() -> (Vec<String>, Vec<String>) {
    let p = std::env::var("CORTEX_RFC3339_VECTORS")
        .unwrap_or_else(|_| "/path/to/docs/mvp/api/rfc3339-vectors.json".to_owned());
    let text = std::fs::read_to_string(&p).expect("the shared vectors are readable");
    let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .expect(k)
            .iter()
            .map(|s| s.as_str().expect("a string").to_owned())
            .collect()
    };
    (list("accept"), list("reject"))
}

#[test]
fn w1_dt_1_every_accept_vector_validates_and_parses() {
    let (accept, _) = vectors();
    assert_eq!(accept.len(), 14, "the shared list");
    for s in &accept {
        assert!(validate(s).is_ok(), "accept: {s}");
        assert!(parse(s).is_ok(), "accept: {s}");
    }
}

#[test]
fn w1_dt_2_every_reject_vector_is_refused_by_both() {
    let (_, reject) = vectors();
    assert_eq!(reject.len(), 48, "the shared list");
    for s in &reject {
        assert!(validate(s).is_err(), "reject: {s:?}");
        assert!(parse(s).is_err(), "reject: {s:?}");
    }
}

#[test]
fn w1_dt_3_more_rejects_than_the_file_names() {
    let long = format!("2026-11-06T13:00:00.{}Z", "1".repeat(4000));
    for s in [
        "2026-11-06T13:00:00Z\n",
        " 2026-11-06T13:00:00Z",
        "2026-11-06T13:00:00Z ",
        "2026-11-06T13:00:00\0Z",
        "2026-11-06T13:00:00Z\0",
        "2026-11-06T13:00:00\u{ff1a}00Z",
        "２０２６-11-06T13:00:00Z",
        "2026-11-06T13:00:00-00:00:00",
        "2026-11-06T13:00:00+14:00Z",
        "2026-11-06T13:00:00ZZ",
        "2026-11-06T13:00:00,5Z",
        "2026-11-06T13:00:00.5.5Z",
        "2026-1-06T13:00:00Z",
        "26-11-06T13:00:00Z",
        "1969-12-31T23:59:59Z",
        "2026-11-06T13:00:00-12:01",
        "2026-11-06T13:00:00+14:30",
        "2026-02-29T13:00:00Z",
        "1900-02-29T00:00:00Z",
        long.as_str(),
    ] {
        assert!(validate(s).is_err(), "reject: {s:?}");
    }
    // The month lengths.
    for (ok, bad) in [
        ("2026-04-30", "2026-04-31"),
        ("2026-12-31", "2026-12-32"),
        ("2024-02-29", "2024-02-30"),
    ] {
        assert!(validate(&format!("{ok}T00:00:00Z")).is_ok(), "{ok}");
        assert!(validate(&format!("{bad}T00:00:00Z")).is_err(), "{bad}");
    }
}

#[test]
fn w1_dt_3b_every_month_ends_on_its_own_last_day() {
    // A non-leap year, then a leap one: the last day is accepted and the next is refused, in all twelve months.
    for (year, lengths) in [
        (2026, [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]),
        (2028, [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]),
    ] {
        for (m, last) in (1..=12).zip(lengths) {
            assert!(
                validate(&format!("{year}-{m:02}-{last:02}T12:00:00Z")).is_ok(),
                "{year}-{m:02}-{last:02}"
            );
            assert!(
                validate(&format!("{year}-{m:02}-{:02}T12:00:00Z", last + 1)).is_err(),
                "{year}-{m:02}-{:02}",
                last + 1
            );
        }
    }
}

#[test]
fn w1_dt_4_parse_gives_the_utc_instant_in_microseconds() {
    let at = |s: &str| parse(s).expect(s).unix_micros();
    assert_eq!(at("1970-01-01T00:00:00Z"), 0);
    assert_eq!(at("2026-11-06T13:00:00Z"), 1_793_970_000_000_000);
    assert_eq!(at("2026-11-06T13:00:00+00:00"), 1_793_970_000_000_000);
    assert_eq!(
        at("2026-11-06T13:00:00+01:00"),
        1_793_966_400_000_000,
        "an hour east is an hour earlier"
    );
    assert_eq!(at("2026-11-06T13:00:00-05:30"), 1_793_989_800_000_000);
    assert_eq!(at("2026-11-06T13:00:00+14:00"), 1_793_919_600_000_000);
    assert_eq!(at("2026-11-06T13:00:00-12:00"), 1_794_013_200_000_000);
    assert_eq!(at("2028-02-29T00:00:00Z"), 1_835_395_200_000_000);
    assert_eq!(at("9999-12-31T23:59:59Z"), 253_402_300_799_000_000);
    assert_eq!(at("2026-11-06T13:00:00.123456Z"), 1_793_970_000_123_456);
    assert_eq!(
        at("2026-11-06T13:00:00.123456789Z"),
        1_793_970_000_123_456,
        "digits past the sixth are cut"
    );
    assert_eq!(at("2026-11-06T13:00:00.5Z"), 1_793_970_000_500_000);
}

#[test]
fn w1_dt_5_format_is_the_second_precision_z_form_and_parses_back() {
    assert_eq!(
        format(Timestamp::from_unix_micros(0)),
        "1970-01-01T00:00:00Z"
    );
    assert_eq!(
        format(Timestamp::from_unix_micros(1_793_970_000_999_999)),
        "2026-11-06T13:00:00Z",
        "rounds down"
    );
    assert_eq!(
        format(Timestamp::from_unix_micros(253_402_300_799_000_000)),
        "9999-12-31T23:59:59Z"
    );
    assert_eq!(
        format(Timestamp::from_unix_micros(951_782_400_000_000)),
        "2000-02-29T00:00:00Z",
        "a leap day"
    );
    for micros in [
        0_i64,
        1_000_000,
        86_399_000_000,
        951_782_399_000_000,
        1_793_970_000_000_000,
        4_102_444_799_000_000,
    ] {
        let t = Timestamp::from_unix_micros(micros);
        assert_eq!(parse(&format(t)).expect("parses back"), t, "{micros}");
    }
}

/// vera's PORTS3-L1 (6 Nov): the round-trip limit, stated and covered. The rule's YEAR domain (1970 to 9999) is read
/// from the text's OWN fields, in its own offset. A valid local text on the edge names an instant whose UTC date is
/// outside that domain: it is ACCEPTED (nothing is normalised, and Postgres holds the instant), `parse` gives the
/// instant, and `format` writes it truthfully in UTC, which then is NOT a text `validate` accepts. So the round
/// trip `parse` then `format` then `validate` holds for every instant whose UTC date is inside 1970..9999, and only
/// for those.
#[test]
fn w1_dt_6_a_valid_local_text_on_the_edge_is_accepted_and_its_utc_form_may_leave_the_domain() {
    // The first instant of 1970 in an offset east of UTC is the previous day in UTC.
    let early = "1970-01-01T00:00:00+01:00";
    assert!(validate(early).is_ok(), "a valid local text is accepted");
    let t = parse(early).expect("parses");
    assert_eq!(t.unix_micros(), -3_600_000_000);
    assert_eq!(format(t), "1969-12-31T23:00:00Z");
    assert!(
        validate(&format(t)).is_err(),
        "its UTC form is outside the domain"
    );
    // The last second of 9999 in an offset west of UTC is year 10000 in UTC.
    let late = "9999-12-31T23:59:59-01:00";
    assert!(validate(late).is_ok());
    let t = parse(late).expect("parses");
    assert_eq!(t.unix_micros(), 253_402_300_799_000_000 + 3_600_000_000);
    assert_eq!(
        format(t),
        "10000-01-01T00:59:59Z",
        "five digits of year, truthfully"
    );
    assert!(validate(&format(t)).is_err());
    // The far edges of the offset range, still accepted.
    assert!(validate("1970-01-01T00:00:00-12:00").is_ok());
    assert!(validate("9999-12-31T23:59:59+14:00").is_ok());
    assert_eq!(
        parse("1970-01-01T00:00:00-12:00")
            .expect("parses")
            .unix_micros(),
        12 * 3_600_000_000
    );
}

#[test]
fn w1_dt_7_every_instant_inside_the_utc_domain_round_trips_through_format() {
    // The domain: the first instant of 1970 to the last second of 9999 (format rounds down to the second).
    for secs in [
        0_i64,
        1,
        59,
        86_399,
        86_400,
        951_782_400,     // 2000-02-29
        4_102_444_799,   // 2099-12-31T23:59:59
        32_503_680_000,  // 3000-01-01
        253_402_214_400, // 9999-01-01
        253_402_300_798,
        253_402_300_799, // 9999-12-31T23:59:59
    ] {
        let t = Timestamp::from_unix_micros(secs * 1_000_000 + 999_999);
        let text = format(t);
        assert!(validate(&text).is_ok(), "{secs}: {text}");
        assert_eq!(
            parse(&text).expect("parses back").unix_micros(),
            secs * 1_000_000,
            "{secs}: rounded down to the second"
        );
    }
}
