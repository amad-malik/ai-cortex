//! F1-04, r3 (mike's F1A-R2-01): an ID holds a UUIDv7 with the RFC variant, not version 7 alone.
//! Each case is paired: the same text with the RFC variant (the fourth group starts 8, 9, a or b)
//! parses; with the NCS (0..7), Microsoft (c, d) or future (e, f) variant it is refused, through
//! every ID type's `parse` and its serde decoding. All values are synthetic.
//! RED (baseline ca217ac): only the version nibble is checked, so the non-RFC variants parse.
use cortex_types::{
    EventId, IdError, InstallationId, ProjectId, RecordId, RequestId, TeamId, TokenId, WorkerId,
};

/// Version 7 in the third group; the fourth group's first hex digit varies.
fn text(variant_digit: char) -> String {
    format!("018f3e2a-1b2c-7d3e-{variant_digit}f40-0123456789ab")
}

const RFC: [char; 4] = ['8', '9', 'a', 'b'];
const NOT_RFC: [char; 12] = ['0', '1', '2', '3', '4', '5', '6', '7', 'c', 'd', 'e', 'f'];

macro_rules! check {
    ($($t:ty),+) => {$(
        for d in RFC {
            let s = text(d);
            assert!(<$t>::parse(&s).is_ok(), "{}: the RFC variant {d} was refused", stringify!($t));
            let json = format!("\"{s}\"");
            assert!(serde_json::from_str::<$t>(&json).is_ok(), "{}: serde refused the RFC variant {d}", stringify!($t));
        }
        for d in NOT_RFC {
            let s = text(d);
            assert!(
                matches!(<$t>::parse(&s), Err(IdError::NotV7(_)) | Err(IdError::NotUuid(_))),
                "{}: the non-RFC variant {d} was accepted",
                stringify!($t)
            );
            let json = format!("\"{s}\"");
            assert!(serde_json::from_str::<$t>(&json).is_err(), "{}: serde accepted the non-RFC variant {d}", stringify!($t));
        }
    )+};
}

#[test]
fn f1_04_every_id_requires_the_rfc_variant_as_well_as_version_7() {
    check!(
        InstallationId,
        ProjectId,
        TeamId,
        WorkerId,
        TokenId,
        RecordId,
        EventId,
        RequestId
    );
    // mike's exact counterexample: the NCS variant, version 7.
    assert!(ProjectId::parse("00000000-0000-7000-0000-000000000000").is_err());
}
