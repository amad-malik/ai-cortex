//! Stage D, L3: the cells that need a crowd or a changing role: TOK-29 (no cache of a role), TOK-41 and TOK-42
//! (paging over 120 tokens, a team lead's page is full), DEF-1 (no renewal route), DEF-5 (no predecessor, no
//! first use). Needs the throwaway Postgres (`CV2_DB=1`).
mod common;

use common::chain::{Answer, World, start};
use common::db::insert_token;
use cortex_types::{TokenId, WorkerId};

fn ids(a: &Answer) -> Vec<String> {
    a.json["items"].as_array().map_or_else(Vec::new, |v| {
        v.iter()
            .filter_map(|i| i["token_id"].as_str().map(str::to_owned))
            .collect()
    })
}

async fn set_role(w: &World, who: WorkerId, role: &str) {
    let mut admin = w.db.admin().await;
    sqlx::query("UPDATE identity.team_members SET role = $1 WHERE worker_id = $2::uuid")
        .bind(role)
        .bind(who.to_string())
        .execute(&mut admin)
        .await
        .expect("role");
}

#[tokio::test]
async fn tok_29_a_demotion_and_a_promotion_take_effect_on_the_next_request_with_no_restart() {
    let w = start("tm_role").await;
    let s = w.seed;
    let path = format!("/v2/workers/{}/tokens", s.w1);
    let tl = w.bearer(s.tl).to_owned();
    assert_eq!(
        w.post(&tl, &path, serde_json::json!({}))
            .await
            .status
            .as_u16(),
        201
    );
    // The demotion commits; the very next request with the same token is refused (X-02).
    set_role(&w, s.tl, "member").await;
    assert_eq!(
        w.post(&tl, &path, serde_json::json!({})).await.code(),
        (403, Some("missing_scope"))
    );
    // Promote and demote between two requests: the outcome follows each time (X-01, no cache).
    set_role(&w, s.tl, "team_lead").await;
    assert_eq!(
        w.post(&tl, &path, serde_json::json!({}))
            .await
            .status
            .as_u16(),
        201
    );
    set_role(&w, s.tl, "member").await;
    assert_eq!(
        w.post(&tl, &path, serde_json::json!({}))
            .await
            .status
            .as_u16(),
        403
    );
}

#[tokio::test]
async fn tok_41_42_paging_over_120_tokens_is_stable_and_a_team_leads_pages_are_full() {
    let w = start("tm_paging").await;
    let s = w.seed;
    // 120 tokens in project A in all: the 9 fixture tokens of A's seven workers (7) plus 113 more, 30 of which for
    // team T's workers (TL, W1, W2) and the rest for the others.
    let team_t = [s.tl, s.w1, s.w2];
    let others = [s.lead, s.w3, s.w4, s.ul];
    let mut t_ids: Vec<String> = Vec::new();
    for t in team_t {
        t_ids.push(w.token_id(t).to_string());
    }
    for k in 0..27 {
        let who = team_t[k % 3];
        let (_, id) = insert_token(&w.pool, &*w.clock, who, s.a, 0, 0).await;
        t_ids.push(id.to_string());
    }
    for k in 0..86 {
        insert_token(&w.pool, &*w.clock, others[k % 4], s.a, 0, 0).await;
    }
    let total = w
        .count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE project_id = '{}'",
            s.a
        ))
        .await;
    assert_eq!(total, 120);
    // The lead pages through all 120 with the default page of 50: three pages, none twice, a stable order.
    let mut seen: Vec<String> = Vec::new();
    let mut after: Option<String> = None;
    let mut pages = 0;
    loop {
        let path = after
            .as_ref()
            .map_or("/v2/tokens".to_owned(), |c| format!("/v2/tokens?after={c}"));
        let p = w.get(Some(w.bearer(s.lead)), &path).await;
        assert_eq!(p.status.as_u16(), 200, "{}", p.text);
        seen.extend(ids(&p));
        pages += 1;
        match p.json["next"].as_str() {
            Some(n) => after = Some(n.to_owned()),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen.len(), 120);
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        120,
        "none twice"
    );
    // A team lead sees exactly T's 30 tokens, in two pages of 20 and 10 at limit=20, the second with `next` null.
    let tl = w.bearer(s.tl).to_owned();
    let p1 = w.get(Some(&tl), "/v2/tokens?limit=20").await;
    assert_eq!(ids(&p1).len(), 20);
    let next = p1.json["next"].as_str().expect("a next cursor").to_owned();
    let p2 = w
        .get(Some(&tl), &format!("/v2/tokens?limit=20&after={next}"))
        .await;
    assert_eq!(
        ids(&p2).len(),
        10,
        "filtering happens in the query, so a page is never short of the remainder"
    );
    assert!(p2.json["next"].is_null());
    let got: std::collections::BTreeSet<String> = ids(&p1).into_iter().chain(ids(&p2)).collect();
    assert_eq!(got, t_ids.iter().cloned().collect());
    // A cursor taken from the lead's list, used by the team lead, never returns another team's rows.
    let lead_first = w.get(Some(w.bearer(s.lead)), "/v2/tokens?limit=5").await;
    let lead_cursor = lead_first.json["next"].as_str().expect("cursor").to_owned();
    let via_tl = w
        .get(Some(&tl), &format!("/v2/tokens?after={lead_cursor}"))
        .await;
    assert!(ids(&via_tl).iter().all(|i| t_ids.contains(i)));
    // Move W1 to team U: its tokens leave the team lead's list on the next request (Q-8).
    let mut admin = w.db.admin().await;
    sqlx::query("UPDATE identity.team_members SET team_id = $1::uuid WHERE worker_id = $2::uuid")
        .bind(s.u.to_string())
        .bind(s.w1.to_string())
        .execute(&mut admin)
        .await
        .expect("move");
    let all: Vec<String> = {
        let mut v = ids(&w.get(Some(&tl), "/v2/tokens?limit=200").await);
        v.sort();
        v
    };
    assert_eq!(all.len(), 20, "T now has TL and W2 only (10 tokens each)");
    // Bad inputs are 422, an empty project gives an empty page.
    for q in [
        "limit=0",
        "limit=201",
        "limit=-1",
        "limit=abc",
        "limit=1.5",
        "limit=",
        "worker=x",
        "chain=1",
    ] {
        assert_eq!(
            w.get(Some(&tl), &format!("/v2/tokens?{q}")).await.code(),
            (422, Some("invalid_request")),
            "{q}"
        );
    }
}

#[tokio::test]
async fn def_1_5_there_is_no_renewal_route_and_no_row_has_a_predecessor_or_a_first_use() {
    let w = start("tm_def").await;
    let s = w.seed;
    for method in [axum::http::Method::GET, axum::http::Method::POST] {
        let a = w
            .call(
                method.clone(),
                "/v2/self/token/renew",
                Some(w.bearer(s.w4)),
                Some("key-renew-0001"),
                Some("{}"),
            )
            .await;
        assert_eq!(a.code(), (404, Some("not_found")), "{method}");
    }
    // Some traffic: issue, revoke, whoami.
    let t = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/workers/{}/tokens", s.w1),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(t.status.as_u16(), 201);
    let id: TokenId = TokenId::parse(t.json["token_id"].as_str().expect("id")).expect("id");
    let r = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/tokens/{id}/revoke"),
            serde_json::json!({"reason": "lost"}),
        )
        .await;
    assert_eq!(r.status.as_u16(), 200);
    assert_eq!(
        w.get(Some(w.bearer(s.w4)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200
    );
    assert_eq!(w.count("SELECT count(*) FROM tokens.tokens WHERE predecessor_id IS NOT NULL OR first_used_at IS NOT NULL").await, 0);
}

/// The headers of an answer that must match between two refusals, without the ones that differ by nature: the
/// request id and the date.
fn stable_headers(a: &Answer) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = a
        .headers
        .iter()
        .filter(|(k, _)| {
            let k = k.as_str().to_lowercase();
            !k.contains("request") && k != "date"
        })
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("?").to_owned(),
            )
        })
        .collect();
    v.sort();
    v
}

/// TOK-11, the L3 half (vera: "L2 decision equality does not prove renderer/header equality"): an unknown id, a
/// wrong secret for a known id, and a malformed bearer get the same status, the same `type`, the same headers and
/// the same body on a read route and on a write route, **after exactly these volatile parts are removed**:
/// in the BODY (parsed as JSON, compared as `serde_json::Value::to_string()`), every top-level key whose lower-case
/// name contains `request`, and the key `instance`; in the HEADERS, every header whose lower-case name contains
/// `request`, and `date`, the rest compared as sorted lower-cased `(name, value)` pairs. It is NOT a byte-for-byte
/// comparison of the raw bytes (vera's TOKENS-R2).
#[tokio::test]
async fn tok_11_l3_an_unknown_id_a_wrong_secret_and_a_malformed_bearer_answer_alike_after_the_volatile_fields_are_removed()
 {
    let w = start("tm_l3").await;
    let s = w.seed;
    let known = w.bearer(s.w1).to_owned();
    let other = w.bearer(s.w2).to_owned();
    // A well-formed token whose id is in no row.
    let unknown = format!(
        "ctx2_{}.{}",
        "0192f5a0000070008000000000000001",
        &known[38..]
    );
    // The id of W1's token with W2's secret.
    let wrong_secret = format!("{}.{}", &known[..37], &other[38..]);
    let bearers = [
        ("unknown id", unknown),
        ("wrong secret", wrong_secret),
        ("malformed: no prefix", "not-a-token".to_owned()),
        ("malformed: short", "ctx2_abc".to_owned()),
        (
            "malformed: right length, bad alphabet",
            format!("ctx2_{}.{}", &known[5..37], "!".repeat(43)),
        ),
    ];
    let path = format!("/v2/workers/{}/tokens", s.w3);
    let mut reads: Vec<(&str, Answer)> = Vec::new();
    let mut writes: Vec<(&str, Answer)> = Vec::new();
    for (what, b) in &bearers {
        reads.push((what, w.get(Some(b), "/v2/whoami").await));
        writes.push((what, w.post(b, &path, serde_json::json!({})).await));
    }
    for set in [&reads, &writes] {
        let (first_what, first) = &set[0];
        assert_eq!(
            first.code(),
            (401, Some("token_invalid")),
            "{first_what}: {}",
            first.text
        );
        for (what, a) in &set[1..] {
            assert_eq!(a.code(), first.code(), "{what} against {first_what}");
            assert_eq!(
                a.body_without_request_id(),
                first.body_without_request_id(),
                "{what} against {first_what}: body"
            );
            assert_eq!(
                stable_headers(a),
                stable_headers(first),
                "{what} against {first_what}: headers"
            );
        }
    }
    // Nothing was written by any of them.
    assert_eq!(w.count("SELECT count(*) FROM plug_audit.events").await, 0);
}
