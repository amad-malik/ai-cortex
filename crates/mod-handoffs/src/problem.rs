//! The refusals this piece raises: `invalid_request` with a field name, and its own thirteen codes
//! with fixed details. Nothing here carries a request value or a row.

use std::borrow::Cow;

use cortex_module::Problem;

use crate::PROBLEMS;
use crate::rules::Refusal;

/// 422 `invalid_request`, naming the field (a fixed word of ours, never a request value).
pub(crate) fn invalid(field: &str, why: &str) -> Problem {
    let mut p = Problem::invalid_request(422, "");
    p.detail = Cow::Owned(format!("{field}: {why}"));
    p
}

/// One of this piece's own codes (S3 `x-error-codes`), with a fixed detail.
pub(crate) fn own(code: &'static str, detail: &'static str) -> Problem {
    let spec = PROBLEMS
        .iter()
        .find(|s| s.code == code)
        .expect("a code this piece mints");
    Problem::new(spec, detail)
}

/// A rule's refusal as the problem it is.
pub(crate) fn refusal(r: Refusal) -> Problem {
    match r {
        Refusal::TargetNotAllowed => Problem::target_not_allowed(),
        Refusal::SelfReviewForbidden => own(
            "self_review_forbidden",
            "The author of a return cannot review it while someone else can.",
        ),
        Refusal::AlreadyClaimed => own("already_claimed", "Another worker holds this handoff."),
        Refusal::InvalidTransition => own(
            "invalid_transition",
            "The handoff's state does not allow this move.",
        ),
        Refusal::NotHolder => own("not_holder", "Only the holder can do this."),
        Refusal::ReasonRequired => own(
            "reason_required",
            "The project lead needs a reason to release another worker's claim.",
        ),
    }
}

/// A statement's error where the request put a date-time into the SQL (`due_at` on create, the time in the list
/// cursor). The grammar in [`crate::validate`] already refuses every time the store cannot hold; this is the second
/// line (cox W1-39): if one ever got through, Postgres's 22007 (invalid datetime format), 22008 (datetime field
/// overflow) or 22009 (invalid time zone displacement) is the caller's 422, not the server's 500. Everything else
/// follows the platform's mapping, so a real fault is still a fault.
pub(crate) fn time_db(e: sqlx::Error) -> Problem {
    if let sqlx::Error::Database(d) = &e
        && matches!(d.code().as_deref(), Some("22007" | "22008" | "22009"))
    {
        return invalid("date-time", "a time this store cannot hold");
    }
    Problem::from(cortex_db::DbError::from(e))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    /// A database error that reports the SQLSTATE it is given and nothing else.
    #[derive(Debug)]
    struct Sqlstate(&'static str);

    impl std::fmt::Display for Sqlstate {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "sqlstate {}", self.0)
        }
    }

    impl std::error::Error for Sqlstate {}

    impl sqlx::error::DatabaseError for Sqlstate {
        fn message(&self) -> &str {
            "a database error"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn fails_with(code: &'static str) -> (u16, &'static str) {
        let p = time_db(sqlx::Error::Database(Box::new(Sqlstate(code))));
        (p.status, p.code)
    }

    /// W1-39, the second line: a date-time the store cannot hold is the caller's 422 whichever of the three SQLSTATEs
    /// Postgres uses (22007 invalid datetime format, 22008 datetime field overflow, 22009 invalid time zone displacement).
    #[test]
    fn a_time_the_store_cannot_hold_is_a_422() {
        for code in ["22007", "22008", "22009"] {
            assert_eq!(fails_with(code), (422, "invalid_request"), "{code}");
        }
    }

    /// Everything else follows the platform's mapping, so a real fault is still a fault: another class-22 error is a
    /// 500, a constraint a 409, a lost session a 503.
    #[test]
    fn every_other_database_error_keeps_the_platforms_answer() {
        assert_eq!(
            fails_with("22012").0,
            500,
            "division by zero is not the caller's"
        );
        assert_eq!(
            fails_with("22P02").0,
            500,
            "nor is a malformed uuid: this piece parses its own ids"
        );
        assert_eq!(fails_with("23505").0, 409);
        assert_eq!(fails_with("42501").0, 500);
        assert_eq!(time_db(sqlx::Error::PoolTimedOut).status, 503);
    }
}
