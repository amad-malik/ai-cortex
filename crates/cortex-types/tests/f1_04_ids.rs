//! F1-04 IDs cannot be confused. [BASE] (lane A TESTS r5 §2, LA-05; S1 §2.1; S2 §3.2.)
//!
//! Each ID type holds a UUIDv7 made from the clock; its timestamp is the UUIDv7's millisecond
//! prefix; `parse` accepts only a UUIDv7; and a `WorkerId` cannot be passed where a `ProjectId`
//! is expected (trybuild, with the pinned `mismatched types` diagnostic and a pass partner).
//!
//! RED: the baseline IDs are aliases of `uuid::Uuid`, so the substitution compiles and the
//! compile-fail case fails at run time; `new`, `timestamp` and `parse` are `todo!()`.
//! Control (GREEN): the aliases again make the pinned diagnostic absent.
//! Not here: the monotonic generator across a clock step back is F-1A6 (BOUND A6B).

use cortex_types::{
    Clock, EventId, Id, IdError, InstallationId, ProjectId, RecordId, RequestId, TeamId, Timestamp,
    TokenId, WorkerId,
};

/// A fixed clock: 2023-11-14T22:13:20.123456Z.
struct Fixed(Timestamp);
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        self.0
    }
}
const AT_MICROS: i64 = 1_700_000_000_123_456;

fn is_lowercase_hyphenated_v7(s: &str) -> bool {
    let u = uuid::Uuid::parse_str(s).expect("Display must be a UUID");
    s.len() == 36
        && s == s.to_lowercase()
        && s.chars().filter(|c| *c == '-').count() == 4
        && u.get_version_num() == 7
        // mike's F1A-R2-01 note: the version nibble alone does not make a UUIDv7.
        && u.get_variant() == uuid::Variant::RFC4122
}

fn holds_v7_with_millisecond_timestamp<T: Id + std::fmt::Display + PartialEq + std::fmt::Debug>(
    name: &str,
) {
    let clock = Fixed(Timestamp::from_unix_micros(AT_MICROS));
    let id = T::new(&clock);
    let text = id.to_string();
    assert!(
        is_lowercase_hyphenated_v7(&text),
        "{name}: {text} is not a lowercase hyphenated UUIDv7"
    );
    assert_eq!(
        id.timestamp().unix_millis(),
        AT_MICROS / 1000,
        "{name}: timestamp is not the clock's millisecond"
    );
    assert_eq!(
        id.timestamp().unix_micros(),
        (AT_MICROS / 1000) * 1000,
        "{name}: timestamp is not truncated to the millisecond"
    );
    assert_eq!(
        T::parse(&text).expect("its own text parses"),
        id,
        "{name}: parse(to_string()) round trip"
    );
}

#[test]
fn f1_04_each_id_holds_a_uuidv7_with_its_millisecond_timestamp() {
    holds_v7_with_millisecond_timestamp::<InstallationId>("InstallationId");
    holds_v7_with_millisecond_timestamp::<ProjectId>("ProjectId");
    holds_v7_with_millisecond_timestamp::<TeamId>("TeamId");
    holds_v7_with_millisecond_timestamp::<WorkerId>("WorkerId");
    holds_v7_with_millisecond_timestamp::<TokenId>("TokenId");
    holds_v7_with_millisecond_timestamp::<RecordId>("RecordId");
    holds_v7_with_millisecond_timestamp::<EventId>("EventId");
    holds_v7_with_millisecond_timestamp::<RequestId>("RequestId");
}

#[test]
fn f1_04_parse_refuses_anything_but_a_uuidv7() {
    // Control inside the case: a valid UUIDv4 is a UUID, and must still be refused.
    assert_eq!(
        ProjectId::parse("550e8400-e29b-41d4-a716-446655440000"),
        Err(IdError::NotV7(4))
    );
    assert!(matches!(
        ProjectId::parse("not-a-uuid"),
        Err(IdError::NotUuid(_))
    ));
    assert!(matches!(ProjectId::parse(""), Err(IdError::NotUuid(_))));
}

#[test]
fn f1_04_a_worker_id_is_not_a_project_id() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/ids_same_type_pass.rs");
    t.compile_fail("tests/ui/ids_confused_fail.rs");
}
