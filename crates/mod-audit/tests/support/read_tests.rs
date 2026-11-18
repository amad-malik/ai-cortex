use super::{decode_cursor, encode_cursor};

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn audit_cursor_round_trips_microsecond_timestamp_and_v7_id() {
    let at = "2026-11-06T12:00:00.123456Z";
    let id = "01930000-0000-7000-8000-000000000001";
    assert_eq!(
        decode_cursor(&encode_cursor(at, id)).unwrap(),
        (at.into(), id.into())
    );
}

#[test]
fn audit_cursor_rejects_other_lists_invalid_dates_bad_ids_and_oversized_inputs() {
    let id = "01930000-0000-7000-8000-000000000001";
    let invalid_date = hex(&format!("a1|2026-02-30T12:00:00.123456Z|{id}"));
    let invalid_uuid = hex("a1|2026-11-06T12:00:00.123456Z|01930000-0000-4000-8000-000000000001");
    assert_eq!(
        invalid_date.len(),
        134,
        "invalid date must reach date validation"
    );
    assert_eq!(
        invalid_uuid.len(),
        134,
        "invalid UUID must reach UUID validation"
    );
    let cases = [
        String::new(),
        "0".into(),
        "zz".into(),
        "00".repeat(101),
        hex(&format!("h1|2026-11-06T12:00:00Z|{id}")),
        invalid_date,
        invalid_uuid,
        hex(&format!("a1|2026-11-06T12:00:00Z|{id}|extra")),
    ];
    for cursor in cases {
        let problem = decode_cursor(&cursor).expect_err("invalid cursor accepted");
        assert_eq!((problem.status, problem.code), (422, "invalid_request"));
        assert_eq!(problem.detail, "The cursor is not valid for this list.");
    }
}

#[test]
fn audit_cursor_requires_the_exact_emitted_utc_microsecond_shape() {
    let id = "01930000-0000-7000-8000-000000000001";
    for at in [
        "2026-11-06T12:00:00Z",
        "2026-11-06T12:00:00.12345Z",
        "2026-11-06T12:00:00.1234567Z",
        "2026-11-06T12:00:00.123456+00:00",
    ] {
        let cursor = hex(&format!("a1|{at}|{id}"));
        let problem = decode_cursor(&cursor).expect_err("noncanonical timestamp accepted");
        assert_eq!((problem.status, problem.code), (422, "invalid_request"));
    }
}

#[test]
fn audit_cursor_rejects_equal_length_foreign_tags_and_uppercase_hex() {
    let at = "2026-11-06T12:00:00.123456Z";
    let id = "01930000-0000-7000-8000-000000000001";
    let audit = encode_cursor(at, id);
    for tag in ["h1", "t1", "r1"] {
        let cursor = hex(&format!("{tag}|{at}|{id}"));
        assert_eq!(cursor.len(), audit.len());
        assert_eq!(decode_cursor(&cursor).unwrap_err().status, 422);
    }
    assert_eq!(
        decode_cursor(&audit.to_ascii_uppercase())
            .unwrap_err()
            .status,
        422
    );
}

#[test]
fn audit_cursor_rejects_equal_length_wrong_uuid_variant() {
    let at = "2026-11-06T12:00:00.123456Z";
    let valid = "01930000-0000-7000-8000-000000000001";
    let invalid_variant = "01930000-0000-7000-0000-000000000001";
    assert!(decode_cursor(&encode_cursor(at, valid)).is_ok());
    let cursor = encode_cursor(at, invalid_variant);
    assert_eq!(cursor.len(), 134);
    let problem = decode_cursor(&cursor).expect_err("non-RFC4122 variant accepted");
    assert_eq!((problem.status, problem.code), (422, "invalid_request"));
}
