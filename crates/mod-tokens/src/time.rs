//! RFC 3339 text, to the second, in UTC, for the `expires_at` family of fields: the ONE conversion of the workspace,
//! `cortex_types::datetime::format` (wave1/ports3, W1-39), the same one the chain's `Cortex-Token-Expires` header
//! uses, so a body field and that header name the same instant. A piece does not depend on `cortex-http`.

use cortex_types::Timestamp;
use cortex_types::datetime;

/// `YYYY-MM-DDTHH:MM:SSZ`, rounded down to the second.
pub fn rfc3339(t: Timestamp) -> String {
    datetime::format(t)
}

/// The same from microseconds since the epoch.
pub fn rfc3339_micros(micros: i64) -> String {
    datetime::format(Timestamp::from_unix_micros(micros))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        assert_eq!(rfc3339_micros(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_micros(951_782_400_999_999), "2000-02-29T00:00:00Z");
        assert_eq!(
            rfc3339_micros(1_700_000_000_000_000),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(rfc3339_micros(-1), "1969-12-31T23:59:59Z");
    }
}
