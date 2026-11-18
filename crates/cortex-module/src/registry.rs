//! The registry `cortex-server` assembles, and its startup checks (section 1.2).
//!
//! Every check is fatal, and an error names pieces, ports, scopes, codes or paths only.
//! `assemble` collects every error it can find before it stops: the manifest checks and the
//! port wiring first; a piece's `routes` is called only when those all pass, and the router walk
//! runs last.

use std::collections::{BTreeMap, BTreeSet};
use std::task::Poll;
use std::time::Duration;

use cortex_types::{ModuleId, ProblemCodeError, ProblemCodeMinter, ProblemType, ScopeName};

use crate::manifest::{CONTRACT_VERSION, Port};
use crate::piece::{BoxFuture, Health, Piece};
use crate::ports::{Ports, PortsBuilder};
use crate::route::{ProblemSpec, Routes};

/// What a piece's `health()` is given (section 1.3).
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

/// The schema the platform migrations own; no piece may claim it.
const PLATFORM_SCHEMA: &str = "platform";

/// A startup error: names only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartupError {
    #[error("piece `{piece}` was built for a socket contract this server does not support")]
    UnsupportedContract { piece: String },
    #[error("piece id `{piece}` is not a valid module id")]
    BadPieceId { piece: String },
    #[error("piece `{piece}` is compiled in twice")]
    DuplicatePiece { piece: String },
    #[error("piece `{piece}` is enabled but not compiled in")]
    UnknownPiece { piece: String },
    #[error("schema `{schema}` is owned twice")]
    SchemaOwnedTwice { schema: String },
    #[error("core piece `{piece}` is not enabled")]
    CoreDisabled { piece: String },
    #[error("piece `{piece}` uses {port}, which no enabled piece provides")]
    MissingPort { piece: String, port: Port },
    #[error("{port} is provided twice")]
    PortProvidedTwice { port: Port },
    #[error("piece `{piece}` does not provide exactly what it declares: {port}")]
    ProvidesMismatch { piece: String, port: Port },
    #[error("scope `{scope}` is not a valid scope name")]
    BadScope { scope: String },
    #[error("scope `{scope}` is declared twice")]
    ScopeDefinedTwice { scope: String },
    #[error("problem code `{code}` is declared twice, or clashes with a platform code")]
    ProblemCodeTwice { code: String },
    #[error("problem code `{code}` is not a valid code")]
    BadProblemCode { code: String },
    /// W1-31: a code several pieces declare must carry the same status, title and fix in each;
    /// `differs` names the first field that does not.
    #[error("problem code `{code}` is declared by several pieces with a different {differs}")]
    ProblemCodeDiffers { code: String, differs: &'static str },
    #[error("route {method} {path} uses scope `{scope}`, which no enabled piece declares")]
    UnknownScope {
        method: String,
        path: String,
        scope: String,
    },
    #[error("route {method} {path} is mounted twice")]
    RouteTwice { method: String, path: String },
    /// Socket delta 2: a write route's handler gets no query, so it cannot be marked to take one.
    #[error("write route {method} {path} is marked to take a query")]
    QueryOnWrite { method: String, path: String },
}

/// What the server hands the registry.
pub struct Assembly<'a> {
    /// Every compiled piece, one `#[cfg(feature = "mod-x")]` line each, in migration order.
    pub compiled: Vec<Box<dyn Piece>>,
    /// The config's `[modules] enabled`.
    pub enabled: &'a [ModuleId],
    /// The process's one minter (`ProblemCodeMinter::take`), which cortex-http also mints with.
    pub minter: &'a ProblemCodeMinter,
    /// The codes cortex-http mints for itself (`internal_error`, `unavailable`, `busy`,
    /// `conflict`): no piece may declare them.
    pub chain_codes: &'a [&'static str],
}

/// One enabled piece's readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceHealth {
    pub piece: &'static str,
    pub health: Health,
}

/// The enabled pieces, wired.
pub struct Registry {
    pieces: Vec<Box<dyn Piece>>,
    ports: Ports,
    routes: Vec<(&'static str, Routes)>,
    problems: Vec<(ProblemType, ProblemSpec)>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field(
                "pieces",
                &self
                    .pieces
                    .iter()
                    .map(|p| p.manifest().id)
                    .collect::<Vec<_>>(),
            )
            .field("ports", &self.ports)
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// Runs every startup check; on success, the enabled pieces in migration order, wired.
    pub fn assemble(a: Assembly<'_>) -> Result<Registry, Vec<StartupError>> {
        use StartupError as E;
        let mut errs = Vec::new();

        // 1. Every compiled piece: its id, its contract version, compiled once.
        let mut compiled = BTreeSet::new();
        for p in &a.compiled {
            let m = p.manifest();
            if ModuleId::parse(m.id).is_err() {
                errs.push(E::BadPieceId { piece: m.id.into() });
            }
            if m.contract != CONTRACT_VERSION {
                errs.push(E::UnsupportedContract { piece: m.id.into() });
            }
            if !compiled.insert(m.id) {
                errs.push(E::DuplicatePiece { piece: m.id.into() });
            }
        }

        // 2. The enabled list: compiled, and every core piece in it.
        let enabled: BTreeSet<&str> = a.enabled.iter().map(ModuleId::as_str).collect();
        for id in &enabled {
            if !compiled.contains(id) {
                errs.push(E::UnknownPiece {
                    piece: (*id).into(),
                });
            }
        }
        for p in &a.compiled {
            let m = p.manifest();
            if m.core && !enabled.contains(m.id) {
                errs.push(E::CoreDisabled { piece: m.id.into() });
            }
        }
        // A disabled piece is dropped here: not migrated, not wired, no route.
        let pieces: Vec<Box<dyn Piece>> = a
            .compiled
            .into_iter()
            .filter(|p| enabled.contains(p.manifest().id))
            .collect();

        // 3. Schemas, scopes and problem codes of the enabled pieces.
        let mut schemas = BTreeSet::from([PLATFORM_SCHEMA]);
        let mut scopes = BTreeSet::new();
        let codes: BTreeSet<&str> = a.chain_codes.iter().copied().collect();
        let mut problems = Vec::new();
        // The first declaration of each code: another piece may declare it too (W1-31).
        let mut declared: BTreeMap<&'static str, ProblemSpec> = BTreeMap::new();
        for p in &pieces {
            let m = p.manifest();
            if !schemas.insert(m.schema) {
                errs.push(E::SchemaOwnedTwice {
                    schema: m.schema.into(),
                });
            }
            for d in m.scopes {
                if ScopeName::parse(d.name).is_err() {
                    errs.push(E::BadScope {
                        scope: d.name.into(),
                    });
                } else if !scopes.insert(d.name) {
                    errs.push(E::ScopeDefinedTwice {
                        scope: d.name.into(),
                    });
                }
            }
            // The codes THIS piece has declared, apart from the canonical spec of a shared code.
            let mut declared_here: BTreeSet<&str> = BTreeSet::new();
            for spec in m.problems {
                match a.minter.mint(spec.code) {
                    Err(ProblemCodeError::BadCode(_)) => errs.push(E::BadProblemCode {
                        code: spec.code.into(),
                    }),
                    Err(ProblemCodeError::PlatformCode(_)) => errs.push(E::ProblemCodeTwice {
                        code: spec.code.into(),
                    }),
                    Ok(_) if codes.contains(spec.code) => errs.push(E::ProblemCodeTwice {
                        code: spec.code.into(),
                    }),
                    // The same piece declaring a code twice is a mistake, identical or not, and it is
                    // refused whichever piece declared the code first (vera's W1P2-V1).
                    Ok(_) if !declared_here.insert(spec.code) => errs.push(E::ProblemCodeTwice {
                        code: spec.code.into(),
                    }),
                    Ok(t) => match declared.get(spec.code) {
                        None => {
                            declared.insert(spec.code, *spec);
                            problems.push((t, *spec));
                        }
                        // W1-31: several pieces may share a code (S3's `reason_required`, `target_retired`)
                        // when status, title and fix are identical; they are minted once.
                        Some(first) => {
                            if let Some(differs) = spec_differs(first, spec) {
                                errs.push(E::ProblemCodeDiffers {
                                    code: spec.code.into(),
                                    differs,
                                });
                            }
                        }
                    },
                }
            }
        }

        // 4. The ports: each enabled piece provides exactly what it declares, each port once,
        //    and every port a piece uses has a provider.
        let mut b = PortsBuilder::new();
        for p in &pieces {
            b.current = p.manifest().id;
            p.provide(&mut b);
        }
        for port in Port::ALL {
            if b.provided.iter().filter(|(q, _)| *q == port).count() > 1 {
                errs.push(E::PortProvidedTwice { port });
            }
        }
        for p in &pieces {
            let m = p.manifest();
            let did: BTreeSet<Port> = b
                .provided
                .iter()
                .filter(|(_, by)| *by == m.id)
                .map(|(q, _)| *q)
                .collect();
            let declared: BTreeSet<Port> = m.provides.iter().copied().collect();
            for port in did.symmetric_difference(&declared) {
                errs.push(E::ProvidesMismatch {
                    piece: m.id.into(),
                    port: *port,
                });
            }
            for port in m.uses {
                if !b.ports.has(*port) {
                    errs.push(E::MissingPort {
                        piece: m.id.into(),
                        port: *port,
                    });
                }
            }
        }
        if !errs.is_empty() {
            return Err(dedup(errs));
        }

        // 5. Routes, each piece seeing only the ports it uses; then the router walk.
        let ports = b.ports;
        let routes: Vec<(&'static str, Routes)> = pieces
            .iter()
            .map(|p| {
                let m = p.manifest();
                (m.id, p.routes(&ports.only(m.uses)))
            })
            .collect();
        let mut mounted = BTreeSet::new();
        for (_, r) in &routes {
            for e in r.entries() {
                if !scopes.contains(e.scope) {
                    errs.push(E::UnknownScope {
                        method: e.method.as_str().into(),
                        path: e.path.into(),
                        scope: e.scope.into(),
                    });
                }
                if e.write && e.query {
                    errs.push(E::QueryOnWrite {
                        method: e.method.as_str().into(),
                        path: e.path.into(),
                    });
                }
                if !mounted.insert((e.method.as_str().to_owned(), e.path)) {
                    errs.push(E::RouteTwice {
                        method: e.method.as_str().into(),
                        path: e.path.into(),
                    });
                }
            }
        }
        if !errs.is_empty() {
            return Err(dedup(errs));
        }
        Ok(Registry {
            pieces,
            ports,
            routes,
            problems,
        })
    }

    /// The modules `cortex-db`'s `DbConfig.registry` must know: `platform` (the idempotency
    /// helper delegates to it), then every enabled piece in migration order. Build the pool's
    /// `ModuleRegistry` from this, never from a hand-built list (piece 11 contract s4b).
    pub fn db_modules(&self) -> Vec<ModuleId> {
        std::iter::once(PLATFORM_SCHEMA)
            .chain(self.pieces.iter().map(|p| p.manifest().id))
            .map(|id| ModuleId::parse(id).expect("ids were checked by assemble"))
            .collect()
    }

    /// The enabled pieces, in migration order (after `platform`).
    pub fn pieces(&self) -> impl Iterator<Item = &dyn Piece> {
        self.pieces.iter().map(|p| p.as_ref())
    }

    /// Every provided port: the chain takes `TokenVerifier` and `ScopePolicy` from here.
    pub fn ports(&self) -> &Ports {
        &self.ports
    }

    /// Each piece's routes, by piece id, for cortex-http to mount.
    pub fn routes(&self) -> impl Iterator<Item = (&'static str, &Routes)> {
        self.routes.iter().map(|(p, r)| (*p, r))
    }

    /// Takes the routes out, for cortex-http to mount.
    pub fn take_routes(&mut self) -> Vec<(&'static str, Routes)> {
        std::mem::take(&mut self.routes)
    }

    /// Every minted piece problem code, in declaration order, each ONCE even when several pieces share it
    /// (W1-31).
    pub fn problem_codes(&self) -> Vec<&'static str> {
        self.problems.iter().map(|(t, _)| t.code()).collect()
    }

    /// A piece's minted problem code, by its text.
    pub fn problem(&self, code: &str) -> Option<(ProblemType, ProblemSpec)> {
        self.problems
            .iter()
            .find(|(t, _)| t.code() == code)
            .copied()
    }

    /// Every enabled piece's own health, each given 2 s; a piece that does not answer in time
    /// is `NotReady("health check timed out")`. The pool and the migration ledger are the
    /// server's part of readiness.
    pub async fn health(&self) -> Vec<PieceHealth> {
        let checks: Vec<BoxFuture<'_, Health>> = self
            .pieces
            .iter()
            .map(|p| -> BoxFuture<'_, Health> {
                Box::pin(async move {
                    tokio::time::timeout(HEALTH_TIMEOUT, p.health())
                        .await
                        .unwrap_or(Health::NotReady("health check timed out"))
                })
            })
            .collect();
        join_all(checks)
            .await
            .into_iter()
            .zip(&self.pieces)
            .map(|(health, p)| PieceHealth {
                piece: p.manifest().id,
                health,
            })
            .collect()
    }
}

/// The errors in their order, each once.
fn dedup(errs: Vec<StartupError>) -> Vec<StartupError> {
    let mut out: Vec<StartupError> = Vec::with_capacity(errs.len());
    for e in errs {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

/// Polls every future until all are done; the results in the futures' order. Every health check
/// runs at once, so readiness waits at most one timeout, not one per piece.
async fn join_all<T>(mut futs: Vec<BoxFuture<'_, T>>) -> Vec<T> {
    let mut out: Vec<Option<T>> = futs.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (f, slot) in futs.iter_mut().zip(out.iter_mut()) {
            if slot.is_none() {
                match f.as_mut().poll(cx) {
                    Poll::Ready(v) => *slot = Some(v),
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    out.into_iter().flatten().collect()
}

/// The first field in which two declarations of one problem code differ, if any (W1-31).
fn spec_differs(a: &ProblemSpec, b: &ProblemSpec) -> Option<&'static str> {
    if a.status != b.status {
        Some("status")
    } else if a.title != b.title {
        Some("title")
    } else if a.fix != b.fix {
        Some("fix")
    } else {
        None
    }
}
