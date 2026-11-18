//! `SpyPolicy`: a `ScopePolicy` double that RECORDS every `(rule, Target)` a handler passes to
//! `target` (Wave 1; PO-17; cox W1-22). The type cannot stop a piece from building the `Target` with
//! wrong fields, so each piece's handler tests run the handler against a `SpyPolicy` and assert (a)
//! the rule it declared and (b) which `Target` fields it filled.
//!
//! It lives here because a mod crate may not depend on another mod crate (xtask `ModDependsOnMod`),
//! so the double cannot live in `mod-policy`. It is pure: no I/O, no clock.
//!
//! `allows` answers `true` for everything, so the chain's scope check passes and only the target is
//! under test; the real matrix is `mod-policy`'s and is tested there. `target` answers with the
//! fixed answer the spy was built with, after recording the call.
use std::sync::Mutex;

use cortex_module::{Principal, ScopePolicy, Target, TargetRule};
use cortex_types::ScopeName;

/// One call to `ScopePolicy::target`: what the handler declared and what it filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetCall {
    pub rule: TargetRule,
    pub target: Target,
}

/// The recording double.
#[derive(Debug)]
pub struct SpyPolicy {
    answer: bool,
    calls: Mutex<Vec<TargetCall>>,
}

impl SpyPolicy {
    /// `target` answers `true` (the handler proceeds past its check).
    pub fn allowing() -> SpyPolicy {
        SpyPolicy {
            answer: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// `target` answers `false` (the handler must refuse with 403 `target_not_allowed`).
    pub fn denying() -> SpyPolicy {
        SpyPolicy {
            answer: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every call so far, in order. The record is left as it is.
    pub fn calls(&self) -> Vec<TargetCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Every call so far, in order, and the record is emptied.
    pub fn take(&self) -> Vec<TargetCall> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl ScopePolicy for SpyPolicy {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        true
    }

    fn target(&self, _p: &Principal, rule: TargetRule, t: &Target) -> bool {
        // Record first, then answer: a refused call is recorded too.
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(TargetCall {
                rule,
                target: t.clone(),
            });
        self.answer
    }
}
