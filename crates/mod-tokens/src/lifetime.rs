//! The expiry and the renewal point of a new token (S1 4.3 / A3; cells TOK-22, TOK-23). Pure integer arithmetic on
//! microseconds, so a cell can state the result exactly.
//!
//! `expires_at = issued_at + L · (1 − 0.1 · U)` for a lifetime of `L` days and a stagger `U` in `[0, 1)`: tokens
//! issued at one instant spread over the last tenth of their lifetime and do not all expire together.
//! `renew_after = issued_at + ⅔ · (expires_at − issued_at)`.

/// One day in microseconds.
pub const DAY_MICROS: i64 = 86_400_000_000;

/// The shortest and the longest lifetime a project may set, in days (S1 A3; the S2 `identity.projects` CHECK).
pub const MIN_LIFETIME_DAYS: i32 = 7;
pub const MAX_LIFETIME_DAYS: i32 = 365;

/// `(expires_at, renew_after)` in microseconds, for a token issued at `issued` with `lifetime_days` and a stagger
/// `u` in `[0, 1)`. `u` outside the range is clamped.
pub fn expiry(issued: i64, lifetime_days: i32, u: f64) -> (i64, i64) {
    let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
    let full = i64::from(lifetime_days) * DAY_MICROS;
    // The stagger takes up to a tenth off the lifetime. `full` is a whole number of days, so `full * u / 10` is
    // computed in floating point on an exactly representable product for the grid the cells use.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let cut = ((full as f64) * u / 10.0).round() as i64;
    let expires = issued + full - cut;
    (expires, issued + (expires - issued) * 2 / 3)
}
