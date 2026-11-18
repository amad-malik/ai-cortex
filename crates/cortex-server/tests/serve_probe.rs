//! `probe`: the ledger verdict, the startup check and the server's `Probe` (piece 11 `serve` S4a, mira). DB-free: the database is a fake
//! `Ledger`. RED first.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cortex_db::MigrationSet;
use cortex_db::migrate::LedgerState;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece, Ports, PortsBuilder, Registry, Routes,
};
use cortex_server::assembly::assemble_with;
use cortex_server::health::PieceHealth;
use cortex_server::health_routes::Probe;
use cortex_server::probe::{
    Ledger, LedgerFault, LedgerRead, PROBE_TTL_MICROS, ServerProbe, Startup, Verdict,
    startup_check, verdict,
};
use cortex_types::{Clock, ModuleId, ProblemCodeMinter, SystemClock, TestClock, Timestamp};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

// ---- the verdict ----------------------------------------------------------------------------

type States = Vec<(String, Result<LedgerState, LedgerFault>)>;

fn s(
    module: &str,
    st: Result<LedgerState, LedgerFault>,
) -> (String, Result<LedgerState, LedgerFault>) {
    (module.to_owned(), st)
}
fn cur(m: &str) -> (String, Result<LedgerState, LedgerFault>) {
    s(m, Ok(LedgerState::Current))
}
fn unapplied(m: &str) -> (String, Result<LedgerState, LedgerFault>) {
    s(m, Ok(LedgerState::Unapplied { steps: vec![2] }))
}
fn ahead(m: &str) -> (String, Result<LedgerState, LedgerFault>) {
    s(m, Ok(LedgerState::DbAhead))
}
fn changed(m: &str, v: u32) -> (String, Result<LedgerState, LedgerFault>) {
    s(m, Ok(LedgerState::ChecksumChanged { version: v }))
}
fn fault(m: &str, f: LedgerFault) -> (String, Result<LedgerState, LedgerFault>) {
    s(m, Err(f))
}
fn stop(m: &str, v: Option<u32>) -> Verdict {
    Verdict::Stop {
        module: m.to_owned(),
        version: v,
    }
}

#[test]
fn pr1_the_verdict_of_all_the_sets() {
    let cases: Vec<(&str, States, Verdict)> = vec![
        ("nothing to check fails closed", vec![], Verdict::Unchecked),
        (
            "all current",
            vec![cur("platform"), cur("fx")],
            Verdict::Current,
        ),
        (
            "an unapplied step",
            vec![cur("platform"), unapplied("fx")],
            Verdict::Unapplied,
        ),
        (
            "the database ahead",
            vec![cur("platform"), ahead("fx")],
            Verdict::DbAhead,
        ),
        (
            "a database fault",
            vec![cur("platform"), fault("fx", LedgerFault::Database)],
            Verdict::Unchecked,
        ),
        (
            "a changed checksum stops",
            vec![cur("platform"), changed("fx", 3)],
            stop("fx", Some(3)),
        ),
        (
            "the FIRST changed set in migration order, not the lowest version",
            vec![changed("fx", 3), changed("gy", 1)],
            stop("fx", Some(3)),
        ),
        (
            "a stop beats a not-ready listed before it",
            vec![
                unapplied("platform"),
                ahead("a"),
                fault("b", LedgerFault::Database),
                changed("fx", 2),
            ],
            stop("fx", Some(2)),
        ),
        (
            "a malformed set stops without a version",
            vec![cur("platform"), fault("fx", LedgerFault::OutOfOrder)],
            stop("fx", None),
        ),
        (
            "the first stop wins whichever kind",
            vec![fault("a", LedgerFault::OutOfOrder), changed("b", 4)],
            stop("a", None),
        ),
        (
            "ahead beats unapplied",
            vec![unapplied("a"), ahead("b")],
            Verdict::DbAhead,
        ),
        (
            "ahead beats unapplied, other order",
            vec![ahead("a"), unapplied("b")],
            Verdict::DbAhead,
        ),
        (
            "unapplied beats unchecked",
            vec![fault("a", LedgerFault::Database), unapplied("b")],
            Verdict::Unapplied,
        ),
        (
            "unapplied beats unchecked, other order",
            vec![unapplied("a"), fault("b", LedgerFault::Database)],
            Verdict::Unapplied,
        ),
        (
            "ahead beats unchecked",
            vec![fault("a", LedgerFault::Database), ahead("b")],
            Verdict::DbAhead,
        ),
    ];
    for (name, states, want) in cases {
        assert_eq!(verdict(&states), want, "{name}");
    }
}

// ---- the startup check ----------------------------------------------------------------------

fn answered(states: States) -> LedgerRead {
    LedgerRead::Answered(states)
}

#[test]
fn pr2_startup_stops_on_a_changed_checksum_and_starts_not_ready_on_the_rest() {
    let line = |r: &LedgerRead| startup_check(r).expect_err("refused").to_string();

    let e = line(&LedgerRead::Unreachable);
    assert_eq!(e, "serve refused: the database could not be reached");

    let e = line(&LedgerRead::PlatformNotRegistered);
    assert_eq!(
        e,
        "serve refused: the database module registry does not hold platform"
    );

    let e = line(&answered(vec![cur("platform"), changed("records", 7)]));
    assert_eq!(
        e,
        "serve refused: migration 7 of records was changed after it was applied"
    );

    let e = line(&answered(vec![fault("handoffs", LedgerFault::OutOfOrder)]));
    assert_eq!(
        e,
        "serve refused: the migration steps of handoffs are not in order"
    );

    for e in [
        line(&LedgerRead::Unreachable),
        line(&LedgerRead::PlatformNotRegistered),
        line(&answered(vec![changed("records", 7)])),
        line(&answered(vec![fault("handoffs", LedgerFault::OutOfOrder)])),
    ] {
        assert!(!e.contains('\n') && !e.is_empty(), "one line: {e:?}");
    }

    assert_eq!(
        startup_check(&answered(vec![cur("platform"), cur("fx")])),
        Ok(Startup::Current)
    );
    let un = startup_check(&answered(vec![unapplied("fx")])).expect("starts");
    let ah = startup_check(&answered(vec![ahead("fx")])).expect("starts");
    let uc = startup_check(&answered(vec![fault("fx", LedgerFault::Database)])).expect("starts");
    let reasons: Vec<&'static str> = [un, ah, uc]
        .into_iter()
        .map(|s| match s {
            Startup::NotReady(r) => r,
            Startup::Current => panic!("must be not ready"),
        })
        .collect();
    assert!(
        reasons.iter().all(|r| !r.is_empty() && !r.contains('\n')),
        "{reasons:?}"
    );
    assert!(
        reasons[0] != reasons[1] && reasons[1] != reasons[2] && reasons[0] != reasons[2],
        "three different fixed reasons: {reasons:?}"
    );
    // An empty answer fails closed; it is not "current".
    assert!(matches!(
        startup_check(&answered(vec![])),
        Ok(Startup::NotReady(_))
    ));
}

// ---- the probe ------------------------------------------------------------------------------

static AUTH: Manifest = Manifest {
    id: "fx_auth",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_auth",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};
static THING: Manifest = Manifest {
    id: "fx_thing",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_thing",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};
static OTHER: Manifest = Manifest {
    id: "fx_other",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_other",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};

struct Fx {
    manifest: &'static Manifest,
    ready: bool,
}

impl Piece for Fx {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.manifest.id).expect("a module id"),
            schema: self.manifest.schema,
            steps: vec![],
        }
    }
    fn provide(&self, _ports: &mut PortsBuilder) {}
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        let ready = self.ready;
        Box::pin(async move {
            if ready {
                Health::Ready
            } else {
                Health::NotReady("fixture not ready")
            }
        })
    }
}

const COMPILED: [&str; 3] = ["fx_auth", "fx_thing", "fx_other"];

/// auth and thing enabled; other compiled but not enabled. `thing_ready` is the readiness of the enabled `fx_thing`.
fn registry(thing_ready: bool) -> Arc<Registry> {
    let pieces: Vec<Box<dyn Piece>> = vec![
        Box::new(Fx {
            manifest: &AUTH,
            ready: true,
        }),
        Box::new(Fx {
            manifest: &THING,
            ready: thing_ready,
        }),
        Box::new(Fx {
            manifest: &OTHER,
            ready: false,
        }),
    ];
    let enabled = ["fx_auth", "fx_thing"].map(|m| ModuleId::parse(m).expect("a module id"));
    Arc::new(assemble_with(pieces, &enabled, minter()).expect("assembles"))
}

/// A database that answers from a script, counts its reads, and takes a moment to answer (so concurrent callers overlap).
struct FakeLedger {
    reads: AtomicUsize,
    script: Mutex<Vec<LedgerRead>>,
}

impl FakeLedger {
    fn new(script: Vec<LedgerRead>) -> Arc<Self> {
        Arc::new(FakeLedger {
            reads: AtomicUsize::new(0),
            script: Mutex::new(script),
        })
    }
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn read(&self) -> LedgerRead {
        let n = self.reads.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let script = self.script.lock().expect("script");
        script[n.min(script.len() - 1)].clone()
    }
}

fn clock_at(micros: i64) -> Arc<TestClock> {
    Arc::new(TestClock::at(Timestamp::from_unix_micros(micros)))
}

fn probe(ledger: &Arc<FakeLedger>, reg: Arc<Registry>, clock: &Arc<TestClock>) -> ServerProbe {
    let c: Arc<dyn Clock> = clock.clone();
    ServerProbe::new(ledger.clone(), reg, COMPILED.to_vec(), c)
}

async fn answer(p: &ServerProbe) -> (bool, bool) {
    let s = p.snapshot().await;
    (s.reachable, s.current)
}

#[tokio::test]
async fn pr3_the_probe_maps_what_the_database_said() {
    let cases = [
        (LedgerRead::Unreachable, (false, false)),
        (LedgerRead::PlatformNotRegistered, (true, false)),
        (
            answered(vec![cur("platform"), cur("fx_auth")]),
            (true, true),
        ),
        (
            answered(vec![cur("platform"), unapplied("fx_auth")]),
            (true, false),
        ),
        (answered(vec![ahead("platform")]), (true, false)),
        (answered(vec![changed("platform", 1)]), (true, false)),
        (
            answered(vec![fault("platform", LedgerFault::Database)]),
            (true, false),
        ),
        (answered(vec![]), (true, false)),
    ];
    for (read, want) in cases {
        let ledger = FakeLedger::new(vec![read.clone()]);
        let p = probe(&ledger, registry(true), &clock_at(1_000));
        assert_eq!(answer(&p).await, want, "{read:?}");
    }
}

#[tokio::test]
async fn pr4_every_compiled_piece_is_listed_in_compiled_order_with_enabled_and_ready() {
    let ledger = FakeLedger::new(vec![answered(vec![cur("platform")])]);
    let p = probe(&ledger, registry(false), &clock_at(1_000));
    let got = p.snapshot().await.pieces;
    let want = vec![
        PieceHealth {
            name: "fx_auth".into(),
            enabled: true,
            ready: true,
        },
        PieceHealth {
            name: "fx_thing".into(),
            enabled: true,
            ready: false,
        },
        PieceHealth {
            name: "fx_other".into(),
            enabled: false,
            ready: false,
        },
    ];
    assert_eq!(got, want);
    let p = probe(&ledger, registry(true), &clock_at(1_000));
    assert!(
        p.snapshot().await.pieces[1].ready,
        "a ready enabled piece is ready"
    );
}

#[tokio::test]
async fn pr5_the_database_is_asked_once_per_ttl_whatever_the_callers_do() {
    let ledger = FakeLedger::new(vec![answered(vec![cur("platform")])]);
    let clock = clock_at(5_000_000);
    let p = probe(&ledger, registry(true), &clock);

    // One document = three questions = one read.
    let _ = answer(&p).await;
    let _ = p.snapshot().await;
    assert_eq!(ledger.reads(), 1);

    // Just inside the TTL: still the cached answer.
    clock.set(Timestamp::from_unix_micros(
        5_000_000 + PROBE_TTL_MICROS - 1,
    ));
    let _ = answer(&p).await;
    assert_eq!(ledger.reads(), 1, "inside the TTL");

    // At the TTL: a new read.
    clock.set(Timestamp::from_unix_micros(5_000_000 + PROBE_TTL_MICROS));
    let _ = answer(&p).await;
    assert_eq!(ledger.reads(), 2, "at the TTL");

    // The clock going backwards is not a reason to keep an answer.
    clock.set(Timestamp::from_unix_micros(5_000_000));
    let _ = answer(&p).await;
    assert_eq!(ledger.reads(), 3, "a clock that went back");
}

#[tokio::test]
async fn pr6_callers_that_overlap_share_one_read() {
    let ledger = FakeLedger::new(vec![answered(vec![cur("platform")])]);
    let p = Arc::new(probe(&ledger, registry(true), &clock_at(9_000)));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let p = p.clone();
        tasks.push(tokio::spawn(async move { answer(&p).await }));
    }
    for t in tasks {
        assert_eq!(t.await.expect("task"), (true, true));
    }
    assert_eq!(
        ledger.reads(),
        1,
        "sixteen overlapping callers, one database read"
    );
}

#[tokio::test]
async fn pr7_an_answer_is_kept_until_the_ttl_then_replaced() {
    let ledger = FakeLedger::new(vec![
        answered(vec![cur("platform")]),
        answered(vec![unapplied("platform")]),
        LedgerRead::Unreachable,
    ]);
    let clock = clock_at(0);
    let p = probe(&ledger, registry(true), &clock);
    assert_eq!(answer(&p).await, (true, true));
    assert_eq!(answer(&p).await, (true, true), "kept inside the TTL");
    clock.set(Timestamp::from_unix_micros(PROBE_TTL_MICROS));
    assert_eq!(answer(&p).await, (true, false), "the second answer");
    clock.set(Timestamp::from_unix_micros(2 * PROBE_TTL_MICROS));
    assert_eq!(answer(&p).await, (false, false), "the third answer");
}

// ---- one observation per document; a slow refresh is still fresh; a hung database is bounded ----------------------------------------

use std::sync::atomic::{AtomicBool, AtomicI64};

use cortex_server::fault::Fault;
use cortex_server::health_routes::HealthState;

/// A fixture piece whose readiness the test flips while the probe runs.
struct Switch {
    manifest: &'static Manifest,
    ready: Arc<AtomicBool>,
}

impl Piece for Switch {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.manifest.id).expect("a module id"),
            schema: self.manifest.schema,
            steps: vec![],
        }
    }
    fn provide(&self, _ports: &mut PortsBuilder) {}
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        let ready = self.ready.load(Ordering::SeqCst);
        Box::pin(async move {
            if ready {
                Health::Ready
            } else {
                Health::NotReady("fixture not ready")
            }
        })
    }
}

/// auth always ready; thing ready as the flag says; other compiled, not enabled.
fn registry_switched(thing: &Arc<AtomicBool>) -> Arc<Registry> {
    let pieces: Vec<Box<dyn Piece>> = vec![
        Box::new(Switch {
            manifest: &AUTH,
            ready: Arc::new(AtomicBool::new(true)),
        }),
        Box::new(Switch {
            manifest: &THING,
            ready: thing.clone(),
        }),
        Box::new(Switch {
            manifest: &OTHER,
            ready: Arc::new(AtomicBool::new(false)),
        }),
    ];
    let enabled = ["fx_auth", "fx_thing"].map(|m| ModuleId::parse(m).expect("a module id"));
    Arc::new(assemble_with(pieces, &enabled, minter()).expect("assembles"))
}

/// Every reading of the clock is one TTL later than the last: whatever the probe asked before is expired when it asks again.
struct Ticking(AtomicI64);

impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_micros(self.0.fetch_add(PROBE_TTL_MICROS, Ordering::SeqCst))
    }
}

/// Reading 0 is snapshot A (migrations NOT current, the piece ready); every later reading is snapshot B (migrations current, the piece
/// NOT ready). Neither is ready. The piece's flag is set here, before the registry is asked for its health.
struct AbLedger {
    reads: AtomicUsize,
    piece: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Ledger for AbLedger {
    async fn read(&self) -> LedgerRead {
        let n = self.reads.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            self.piece.store(true, Ordering::SeqCst);
            answered(vec![cur("platform"), unapplied("fx_auth")])
        } else {
            self.piece.store(false, Ordering::SeqCst);
            answered(vec![cur("platform"), cur("fx_auth")])
        }
    }
}

#[tokio::test]
async fn pr8_a_health_document_is_one_observation_never_a_mix_of_two() {
    let piece = Arc::new(AtomicBool::new(false));
    let ledger = Arc::new(AbLedger {
        reads: AtomicUsize::new(0),
        piece: piece.clone(),
    });
    let probe = ServerProbe::new(
        ledger.clone(),
        registry_switched(&piece),
        COMPILED.to_vec(),
        Arc::new(Ticking(AtomicI64::new(1_000_000))),
    );
    let state = HealthState {
        version: "0.0.0-test".to_owned(),
        probe: Arc::new(probe),
        last_good: None,
        fault: Fault::None,
        clock: Arc::new(SystemClock),
    };
    // Each document finds the cache expired. Snapshot A: migrations behind (not ready). Snapshot B: a piece unready (not ready).
    let first = state.document().await;
    assert!(
        !first.ready,
        "A was not ready, so the document is not ready: {first:?}"
    );
    let second = state.document().await;
    assert!(!second.ready, "B was not ready either: {second:?}");
    assert!(!state.ready().await, "so /v2/health/ready is a 503");
}

/// A database whose read takes longer than the TTL on the injected clock.
struct SlowLedger {
    reads: AtomicUsize,
    clock: Arc<TestClock>,
}

#[async_trait::async_trait]
impl Ledger for SlowLedger {
    async fn read(&self) -> LedgerRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let now = self.clock.now().unix_micros();
        self.clock
            .set(Timestamp::from_unix_micros(now + 3 * PROBE_TTL_MICROS));
        answered(vec![cur("platform")])
    }
}

#[tokio::test]
async fn pr9_an_answer_that_took_longer_than_the_ttl_is_still_fresh_when_it_arrives() {
    let clock = clock_at(7_000_000);
    let ledger = Arc::new(SlowLedger {
        reads: AtomicUsize::new(0),
        clock: clock.clone(),
    });
    let c: Arc<dyn Clock> = clock.clone();
    let p = ServerProbe::new(ledger.clone(), registry(true), COMPILED.to_vec(), c);
    let first = p.snapshot().await;
    assert_eq!((first.reachable, first.current), (true, true));
    // The refresh itself took three TTLs on the clock. A caller that queued behind it asks next, at the same moment, and shares its
    // answer instead of starting another read.
    let second = p.snapshot().await;
    assert_eq!(second, first);
    assert_eq!(
        ledger.reads.load(Ordering::SeqCst),
        1,
        "one read for both callers"
    );
}

/// A database that never answers.
struct Hung {
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl Ledger for Hung {
    async fn read(&self) -> LedgerRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        std::future::pending::<()>().await;
        LedgerRead::Unreachable
    }
}

#[tokio::test]
async fn pr10_a_hung_database_is_not_reachable_after_the_refresh_timeout_and_overlapping_callers_share_the_wait()
 {
    let ledger = Arc::new(Hung {
        reads: AtomicUsize::new(0),
    });
    let c: Arc<dyn Clock> = clock_at(3_000_000);
    let p = Arc::new(ServerProbe::with_refresh_timeout(
        ledger.clone(),
        registry(true),
        COMPILED.to_vec(),
        c,
        Duration::from_millis(100),
    ));
    let started = std::time::Instant::now();
    // Four callers ask while the database is hung: they queue behind ONE refresh and share its (timed-out) answer.
    let mut callers = Vec::new();
    for _ in 0..4 {
        let p = p.clone();
        callers.push(tokio::spawn(async move { answer(&p).await }));
    }
    for caller in callers {
        let got = tokio::time::timeout(Duration::from_secs(5), caller)
            .await
            .expect("the probe answered although the database never did")
            .expect("task");
        assert_eq!(
            got,
            (false, false),
            "a database that does not answer is not reachable"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "all four were answered at the one refresh timeout asked for, not four of them: {:?}",
        started.elapsed()
    );
    assert_eq!(
        ledger.reads.load(Ordering::SeqCst),
        1,
        "four overlapping callers, one database read"
    );
    // A later caller inside the TTL reuses the timed-out answer; it does not start a fifth wait.
    let _ = answer(&p).await;
    assert_eq!(ledger.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pr11_the_default_refresh_timeout_is_a_few_seconds_not_minutes() {
    use cortex_server::probe::REFRESH_TIMEOUT;
    assert!(
        REFRESH_TIMEOUT >= Duration::from_secs(1) && REFRESH_TIMEOUT <= Duration::from_secs(10),
        "{REFRESH_TIMEOUT:?}"
    );
    // And `new` uses it: a database that never answers is given up on, by the default, within a few seconds.
    let ledger = Arc::new(Hung {
        reads: AtomicUsize::new(0),
    });
    let c: Arc<dyn Clock> = clock_at(5_000_000);
    let p = ServerProbe::new(ledger.clone(), registry(true), COMPILED.to_vec(), c);
    let started = std::time::Instant::now();
    let got = tokio::time::timeout(REFRESH_TIMEOUT + Duration::from_secs(5), answer(&p))
        .await
        .expect("the default refresh timeout gave up on a database that never answers");
    assert_eq!(got, (false, false));
    assert!(
        started.elapsed() >= REFRESH_TIMEOUT - Duration::from_millis(200),
        "it waited the timeout, not less: {:?}",
        started.elapsed()
    );
}
