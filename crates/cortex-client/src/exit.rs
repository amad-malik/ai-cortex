//! The ONE exit-code table (W1-43 Q-2): 0 ok; 1 anything else or transport; 2 usage (clap's own); 3 authentication
//! or scope; 4 not found; 5 conflict; 6 invalid request (and 428); 7 busy or unavailable after the bounded retry;
//! 8 incomplete: what is printed is real but the list was cut at its page cap (W1-90, `cortex audit list --all`).

use crate::error::ClientError;

pub const OK: u8 = 0;
pub const OTHER: u8 = 1;
pub const USAGE: u8 = 2;
pub const AUTH: u8 = 3;
pub const NOT_FOUND: u8 = 4;
pub const CONFLICT: u8 = 5;
pub const INVALID: u8 = 6;
pub const BUSY: u8 = 7;
/// W1-90 (cox, 8 Nov): a listing that stopped at its page cap with more records on the server. The records ARE printed (as `doctor`
/// prints its output with exit 7 when the server is not ready); the exit status says the list is not complete.
pub const INCOMPLETE: u8 = 8;

/// The exit code for an HTTP status.
pub fn exit_for_status(status: u16) -> u8 {
    match status {
        200..=299 => OK,
        401 | 403 => AUTH,
        404 => NOT_FOUND,
        409 => CONFLICT,
        400 | 405 | 413 | 415 | 422 | 428 => INVALID,
        429 | 503 => BUSY,
        _ => OTHER,
    }
}

/// The exit code for a failure. A failure is never 0.
pub fn exit_code(error: &ClientError) -> u8 {
    let for_status = |s: u16| match exit_for_status(s) {
        OK => OTHER,
        code => code,
    };
    match error {
        ClientError::Problem(p) => for_status(p.status),
        ClientError::Http { status, .. } => for_status(*status),
        ClientError::Busy { .. } => BUSY,
        _ => OTHER,
    }
}
