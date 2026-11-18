//! BOOT-01, BOOT-02, BOOT-04 (`mvp/tests/mod-boot.md`, Identity). DB-free.
mod support;

use axum::extract::FromRequestParts;
use axum::http::{Method, Request};
use cortex_module::{Piece, Port, Ports, Role, StrictQuery, TargetRule};
use mod_boot::seam::{HANDOFF_LIMIT, gather};
use mod_boot::{Boot, BootQuery, MANIFEST, assemble};
use support::*;

async fn decode(uri: &str) -> Result<BootQuery, (u16, &'static str)> {
    let (mut parts, ()) = Request::builder()
        .uri(uri)
        .body(())
        .expect("a request")
        .into_parts();
    StrictQuery::<BootQuery>::from_request_parts(&mut parts, &())
        .await
        .map(|q| q.0)
        .map_err(|p| (p.status, p.code))
}

#[test]
fn boot_01_section_1_names_the_token_worker_with_its_whoami_fields() {
    let b = assemble(&inputs_w1(), 12_288).expect("fits");
    let want = format!(
        "## 1 Identity\nworker: W1 ({w1})\nkind: worker\nrole: member\nteam: {team}\njob roles: qa\n\
         project: {pa}\ninstallation: {inst}\ntoken: {tok}, expires 2026-11-20T12:00:00Z\n",
        w1 = w1(),
        team = team_a(),
        pa = project_a(),
        inst = installation(),
        tok = uuid(T_TOKEN, 0x11),
    );
    assert_eq!(section_text(&b.markdown, 1), want);
    let j = &b.boot.identity;
    assert_eq!(
        (j.worker.as_str(), j.name.as_str(), j.kind, j.role),
        (w1().to_string().as_str(), "W1", "worker", "member")
    );
    assert_eq!(j.team, Some(team_a().to_string()));
    assert_eq!(j.job_roles, ["qa"]);
    assert_eq!(j.project, project_a().to_string());
    assert_eq!(j.installation, installation().to_string());
    assert_eq!(j.token_id, uuid(T_TOKEN, 0x11));
    assert_eq!(j.expires_at, "2026-11-20T12:00:00.000000Z");
    assert!(!j.renew_due);
}

#[test]
fn boot_01_renewal_due_is_shown() {
    let mut i = inputs_w1();
    i.identity.renew_due = true;
    let b = assemble(&i, 12_288).expect("fits");
    assert!(section_text(&b.markdown, 1).ends_with("token renewal due\n"));
    assert!(b.boot.identity.renew_due);
}

/// The worker is the principal's; the role is the CURRENT one from `IdentityApi`, not the
/// token's (v1 vector: the registry role wins over a stale profile).
#[tokio::test]
async fn boot_01_gather_reads_the_principals_worker_and_its_current_role() {
    let mut w = world();
    w.a.workers[1].role = Role::TeamLead(team_a());
    let mut r = Reads::for_project(&w, project_a());
    let i = gather(&mut r, &principal_w1()).await.expect("gathers");
    assert_eq!(i.identity.worker, w1());
    assert_eq!(i.identity.role, Role::TeamLead(team_a()));
    assert_eq!(i.identity.name, "W1");
    assert_eq!(i.identity.job_roles, ["qa"]);
    assert_eq!(i.identity.installation, installation());
    assert_eq!(i.identity.token, principal_w1().token);
    assert_eq!(
        r.calls,
        [
            "project".to_owned(),
            format!("worker {}", w1()),
            "section orientation".to_owned(),
            "section rules-project".to_owned(),
            format!("section rules-worker:{}", w1()),
            format!("claimed_by {} {HANDOFF_LIMIT}", w1()),
            format!("pending_for {} {HANDOFF_LIMIT}", w1()),
        ],
        "boot asks only about the principal's own worker"
    );
}

/// An extra query parameter is 422 `invalid_request` (R-4); it is never honoured.
#[tokio::test]
async fn boot_01_an_extra_or_repeated_query_parameter_is_422() {
    assert_eq!(
        decode("/v2/boot?worker=w2").await,
        Err((422, "invalid_request"))
    );
    assert_eq!(
        decode("/v2/boot?budget=8192&project=b").await,
        Err((422, "invalid_request"))
    );
    assert_eq!(
        decode("/v2/boot?budget=8192&budget=4096").await,
        Err((422, "invalid_request"))
    );
    assert_eq!(
        decode("/v2/boot?budget=8192&format=json").await,
        Ok(BootQuery {
            budget: Some(8192),
            format: Some(mod_boot::Format::Json)
        })
    );
    assert_eq!(decode("/v2/boot").await, Ok(BootQuery::default()));
}

/// BOOT-02's piece-side half: the route is `self:read` (held by kind worker only, R-12, so a
/// service is 403 `missing_scope` at the chain's scope step: sophi's kind check, bound later),
/// target `None`, and it takes a query. Boot declares no scope of its own.
#[test]
fn boot_02_the_route_is_self_read_and_boot_declares_no_scope() {
    let r = Boot::new().routes(&Ports::default());
    let v: Vec<_> = r.entries().collect();
    assert_eq!(v.len(), 1);
    assert_eq!(
        (
            v[0].method,
            v[0].path,
            v[0].scope,
            v[0].target,
            v[0].write,
            v[0].query
        ),
        (
            &Method::GET,
            "/v2/boot",
            "self:read",
            TargetRule::None,
            false,
            true
        )
    );
    assert!(MANIFEST.scopes.is_empty());
    assert!(MANIFEST.provides.is_empty());
    assert_eq!(
        MANIFEST.uses,
        [Port::Identity, Port::Records, Port::Handoffs]
    );
    assert_eq!(
        MANIFEST
            .problems
            .iter()
            .map(|p| (p.code, p.status))
            .collect::<Vec<_>>(),
        [("boot_budget_too_small", 422)]
    );
    assert!(
        Boot::new().migrations().steps.is_empty(),
        "boot owns no table"
    );
}

/// No B canary, name or count anywhere, at any budget, in either form.
#[tokio::test]
async fn boot_04_no_project_b_text_reaches_project_a() {
    let w = world();
    let mut r = Reads::for_project(&w, project_a());
    let i = gather(&mut r, &principal_w1()).await.expect("gathers");
    for budget in [4_096, 5_021, 12_288, 65_536, 262_144] {
        let b = assemble(&i, budget).expect("fits");
        let json = serde_json::to_string(&b.boot).expect("json");
        for out in [&b.markdown, &json] {
            assert!(!out.contains(CANARY), "budget {budget}");
            assert!(!out.contains(&project_b().to_string()), "budget {budget}");
            assert!(!out.contains(&lead_b().to_string()), "budget {budget}");
        }
    }
}
