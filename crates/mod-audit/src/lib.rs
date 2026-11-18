//! Project-scoped audit piece.

use cortex_module::ports::{AuditDetails, DetailRef, DetailValue, PortError};

mod piece;
mod read;
pub use piece::AuditPiece;

/// Validate a producer's structured details against the action contract.
/// A valid code shape does not establish that caller-chosen data is non-secret.
pub fn validate_details(action: &str, details: &AuditDetails) -> Result<(), PortError> {
    let known = matches!(
        action,
        "section.write_reserved"
            | "token.issued"
            | "token.revoked"
            | "token.self_revoked"
            | "token.reissued"
            | "handoff.created"
            | "handoff.claimed"
            | "handoff.released"
            | "handoff.returned"
            | "handoff.accepted"
            | "handoff.reworked"
            | "handoff.cancelled"
            | "record.superseded"
            | "record.invalidated"
            | "record.invalidation_undone"
            | "team.created"
            | "worker.created"
            | "worker.retired"
            | "worker.team_assigned"
            | "project.bootstrapped"
    );
    if !known {
        return Err(PortError::Invalid("audit action"));
    }
    if action == "worker.team_assigned"
        && !details.entries().iter().any(|(key, value)| {
            *key == "job_roles_changed" && matches!(value, DetailValue::Flag(_))
        })
    {
        return Err(PortError::Invalid("audit details"));
    }
    if action == "project.bootstrapped"
        && !["project", "lead", "created", "reissued"]
            .iter()
            .all(|required| details.entries().iter().any(|(key, _)| key == required))
    {
        return Err(PortError::Invalid("audit details"));
    }
    for (key, value) in details.entries() {
        let ok = match (action, *key, value) {
            ("handoff.accepted" | "handoff.reworked", "self_review", DetailValue::Flag(_)) => true,
            ("token.revoked" | "token.self_revoked", "reason", DetailValue::Code(code)) => {
                let text = code.as_str().as_bytes();
                !text.is_empty()
                    && text[0].is_ascii_lowercase()
                    && text.iter().all(|c| c.is_ascii_lowercase() || *c == b'_')
            }
            ("worker.created", "team", DetailValue::Ref(DetailRef::Team(_))) => true,
            ("worker.created", "mode", DetailValue::Code(v)) => {
                matches!(v.as_str(), "model" | "deterministic")
            }
            (
                "worker.team_assigned",
                "team" | "from_team",
                DetailValue::Ref(DetailRef::Team(_)),
            ) => true,
            ("worker.team_assigned", "role" | "from_role", DetailValue::Code(v)) => {
                matches!(v.as_str(), "member" | "team_lead")
            }
            ("worker.team_assigned", "job_roles_changed", DetailValue::Flag(_)) => true,
            ("project.bootstrapped", "project", DetailValue::Ref(DetailRef::Project(_))) => true,
            ("project.bootstrapped", "lead", DetailValue::Ref(DetailRef::Worker(_))) => true,
            ("project.bootstrapped", "created" | "reissued", DetailValue::Flag(_)) => true,
            _ => false,
        };
        if !ok {
            return Err(PortError::Invalid("audit details"));
        }
    }
    Ok(())
}
