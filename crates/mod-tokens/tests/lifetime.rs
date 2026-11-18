//! The expiry and renewal arithmetic of a new token (TOK-22, TOK-23), pure. RED: `expiry` is `todo!()`.
use mod_tokens::lifetime::{DAY_MICROS, expiry};

const T0: i64 = 1_800_000_000_123_456;

#[test]
fn tok_23_the_stagger_extremes_for_7_90_and_365_days() {
    for l in [7_i32, 90, 365] {
        let full = i64::from(l) * DAY_MICROS;
        let (e0, _) = expiry(T0, l, 0.0);
        assert_eq!(e0, T0 + full, "U = 0 gives the whole lifetime");
        let (e1, _) = expiry(T0, l, 1.0);
        assert_eq!(e1, T0 + full * 9 / 10, "U = 1 gives 0.9 of it");
        let (mid, _) = expiry(T0, l, 0.5);
        assert_eq!(mid, T0 + full * 95 / 100);
    }
    // Out-of-range values are clamped, never extrapolated.
    assert_eq!(expiry(T0, 90, -3.0).0, expiry(T0, 90, 0.0).0);
    assert_eq!(expiry(T0, 90, 7.0).0, expiry(T0, 90, 1.0).0);
}

#[test]
fn tok_22_renew_after_is_two_thirds_of_the_lifetime_exactly() {
    for l in [7_i32, 90, 365] {
        for u in [0.0, 0.25, 0.999_999] {
            let (e, r) = expiry(T0, l, u);
            assert_eq!(r, T0 + (e - T0) * 2 / 3, "L = {l}, U = {u}");
            assert!(T0 <= r && r <= e);
        }
    }
}

#[test]
fn tok_23_a_thousand_tokens_at_one_instant_spread_over_at_least_eight_days_within_81_and_90() {
    let mut lo = i64::MAX;
    let mut hi = i64::MIN;
    for k in 0..1000 {
        let u = f64::from(k) / 1000.0;
        let (e, _) = expiry(T0, 90, u);
        lo = lo.min(e - T0);
        hi = hi.max(e - T0);
    }
    assert!(lo >= 81 * DAY_MICROS && hi <= 90 * DAY_MICROS);
    assert!(hi - lo >= 8 * DAY_MICROS, "a spread of at least 8 days");
}
