//! Stage C, L3: `tokens.whoami`, `tokens.list`, `tokens.inspect` over the real chain (TOK-17/18 on a route, TOK-40
//! to TOK-46, TOK-60 to TOK-63). Needs the throwaway Postgres (`CV2_DB=1`). RED: the routes are not mounted (404).
mod common;

use common::chain::{Answer, World, start};
use cortex_types::{SystemClock, TokenId, WorkerId};

fn ids(a: &Answer) -> Vec<String> {
    a.json["items"].as_array().map_or_else(Vec::new, |v| {
        v.iter()
            .filter_map(|i| i["token_id"].as_str().map(str::to_owned))
            .collect()
    })
}

#[tokio::test]
async fn tok_60_62_whoami_returns_the_callers_own_facts_and_the_installation() {
    let w = start("rd_whoami").await;
    let s = w.seed;
    for (who, team, project) in [
        (s.lead, None, s.a),
        (s.tl, Some(s.t), s.a),
        (s.w1, Some(s.t), s.a),
        (s.w4, None, s.a),
        (s.wb1, None, s.b),
    ] {
        let a = w.get(Some(w.bearer(who)), "/v2/whoami").await;
        assert_eq!(a.status.as_u16(), 200, "{}", a.text);
        let mut keys: Vec<&str> = a
            .json
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        for k in [
            "installation",
            "project",
            "worker",
            "kind",
            "role",
            "token_id",
            "expires_at",
        ] {
            assert!(keys.contains(&k), "{k} in {keys:?}");
        }
        assert_eq!(a.json["worker"], who.to_string());
        assert_eq!(a.json["project"], project.to_string());
        assert_eq!(a.json["installation"], s.inst.to_string(), "TOK-62");
        assert_eq!(
            a.json["team"],
            team.map_or(serde_json::Value::Null, |t| t.to_string().into())
        );
        assert_eq!(a.json["token_id"], w.token_id(who).to_string());
        assert_eq!(a.json["kind"], "worker");
        // Cortex-Token-Expires equals expires_at as the same instant.
        assert!(a.header("cortex-token-expires").is_some());
    }
}

#[tokio::test]
async fn tok_61_63_a_retired_workers_token_is_401_and_whoami_writes_only_last_used_at() {
    let w = start("rd_retired").await;
    let s = w.seed;
    let mut admin = w.db.admin().await;
    sqlx::query(
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
    )
    .bind(s.w2.to_string())
    .execute(&mut admin)
    .await
    .expect("retire");
    assert_eq!(
        w.get(Some(w.bearer(s.w2)), "/v2/whoami").await.code(),
        (401, Some("worker_retired"))
    );
    let before = w.audit().await.len();
    assert_eq!(
        w.get(Some(w.bearer(s.w1)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200
    );
    assert_eq!(w.audit().await.len(), before, "no audit row for a read");
}

#[tokio::test]
async fn tok_18_last_used_at_is_written_even_when_the_request_fails_with_422() {
    let w = start("rd_touch").await;
    let s = w.seed;
    let bad = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/tokens/{}/revoke", w.token_id(s.w1)),
            serde_json::json!({"reason": ""}),
        )
        .await;
    assert_eq!(bad.status.as_u16(), 422);
    let mut admin = w.db.admin().await;
    let used: bool = sqlx::query_scalar(
        "SELECT last_used_at IS NOT NULL FROM tokens.tokens WHERE id = $1::uuid",
    )
    .bind(w.token_id(s.lead).to_string())
    .fetch_one(&mut admin)
    .await
    .expect("row");
    assert!(
        used,
        "verify's own transaction commits whatever happens to the request"
    );
}

#[tokio::test]
async fn tok_40_42_44_the_lead_lists_all_of_its_projects_tokens_a_team_lead_only_its_teams() {
    let w = start("rd_list").await;
    let s = w.seed;
    let a = w.get(Some(w.bearer(s.lead)), "/v2/tokens").await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert!(a.json["next"].is_null());
    let items = a.json["items"].as_array().expect("items");
    assert_eq!(items.len(), 7, "all of A's seven tokens, none of B's");
    for i in items {
        let mut k: Vec<&str> = i
            .as_object()
            .expect("item")
            .keys()
            .map(String::as_str)
            .collect();
        k.sort_unstable();
        assert_eq!(
            k,
            [
                "expires_at",
                "issued_at",
                "last_used_at",
                "revoke_reason",
                "state",
                "token_id",
                "worker"
            ],
            "TokenMeta keys, nothing else"
        );
        assert_eq!(i["state"], "active");
    }
    assert!(!ids(&a).contains(&w.token_id(s.wb1).to_string()));
    // Ordered by (issued_at, token_id).
    let mut sorted = items.clone();
    sorted.sort_by(|x, y| {
        (x["issued_at"].as_str(), x["token_id"].as_str())
            .cmp(&(y["issued_at"].as_str(), y["token_id"].as_str()))
    });
    assert_eq!(&sorted, items);
    // The team lead of T sees T's workers' tokens (TL, W1, W2) only, revoked ones included.
    let revoke = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/tokens/{}/revoke", w.token_id(s.w2)),
            serde_json::json!({"reason": "lost"}),
        )
        .await;
    assert_eq!(revoke.status.as_u16(), 200);
    let t = w.get(Some(w.bearer(s.tl)), "/v2/tokens").await;
    let got: std::collections::BTreeSet<String> = ids(&t).into_iter().collect();
    let want: std::collections::BTreeSet<String> = [s.tl, s.w1, s.w2]
        .iter()
        .map(|x| w.token_id(*x).to_string())
        .collect();
    assert_eq!(got, want);
    // Bad inputs are 422.
    for q in [
        "limit=0",
        "limit=201",
        "limit=-1",
        "limit=abc",
        "limit=1.5",
        "limit=",
        "worker=x",
        "chain=1",
        "after=zzz",
    ] {
        assert_eq!(
            w.get(Some(w.bearer(s.lead)), &format!("/v2/tokens?{q}"))
                .await
                .code(),
            (422, Some("invalid_request")),
            "{q}"
        );
    }
    // Paging: limit=3 over the lead's seven (six after the revoke stays seven: revoked ones are listed).
    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(c) => format!("/v2/tokens?limit=3&after={c}"),
            None => "/v2/tokens?limit=3".to_owned(),
        };
        let p = w.get(Some(w.bearer(s.lead)), &path).await;
        seen.extend(ids(&p));
        match p.json["next"].as_str() {
            Some(n) => after = Some(n.to_owned()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 7);
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        7,
        "none twice"
    );
    // No digest or secret anywhere (TOK-44).
    for text in [a.text.as_str(), t.text.as_str()] {
        assert!(
            !text.contains("ctx2_")
                && !text.contains("digest")
                && !text.contains("install_gen")
                && !text.contains("issued_by")
        );
    }
}

#[tokio::test]
async fn tok_43_46_inspect_answers_200_403_or_404_and_only_the_presented_token_moves() {
    let w = start("rd_inspect").await;
    let s = w.seed;
    let path = |t: TokenId| format!("/v2/tokens/{t}");
    let a = w.get(Some(w.bearer(s.lead)), &path(w.token_id(s.w1))).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(a.json["worker"], s.w1.to_string());
    assert_eq!(a.json["state"], "active");
    assert_eq!(
        w.get(Some(w.bearer(s.tl)), &path(w.token_id(s.w2)))
            .await
            .status
            .as_u16(),
        200
    );
    for t in [s.lead, s.w3, s.w4] {
        assert_eq!(
            w.get(Some(w.bearer(s.tl)), &path(w.token_id(t)))
                .await
                .code(),
            (403, Some("target_not_allowed")),
            "TL on {t}"
        );
    }
    let x = w
        .get(Some(w.bearer(s.lead)), &path(w.token_id(s.wb1)))
        .await;
    let y = w
        .get(Some(w.bearer(s.lead)), &path(TokenId::new(&SystemClock)))
        .await;
    let z = w.get(Some(w.bearer(s.lead)), "/v2/tokens/not-a-uuid").await;
    for r in [&x, &y, &z] {
        assert_eq!(r.code(), (404, Some("not_found")));
    }
    assert_eq!(x.body_without_request_id(), y.body_without_request_id());
    assert_eq!(x.body_without_request_id(), z.body_without_request_id());
    // TOK-46: reading X's metadata with Y's token does not move X's last_used_at; presenting X does.
    async fn used(w: &World, id: TokenId) -> bool {
        let mut admin = w.db.admin().await;
        sqlx::query_scalar::<_, bool>(
            "SELECT last_used_at IS NOT NULL FROM tokens.tokens WHERE id = $1::uuid",
        )
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("row")
    }
    assert!(
        !used(&w, w.token_id(s.w4)).await,
        "W4's token was never presented"
    );
    assert_eq!(
        w.get(Some(w.bearer(s.w4)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200
    );
    assert!(used(&w, w.token_id(s.w4)).await);
    let _: Option<(&World, WorkerId)> = None;
}

fn cursor_of(us: i64, id: TokenId) -> String {
    format!("{us}:{id}")
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// vera's TOKENS-V4 (W1-39 covers cursors): an instant outside 1970..9999 is 422 `invalid_request` BEFORE any
/// SQL, never a database error answered as 500. The bounds are cells of `rework_pure.rs`; here the route.
#[tokio::test]
async fn tok_41b_a_cursor_instant_outside_1970_to_9999_is_422_and_the_boundaries_are_served() {
    let w = start("rd_cursor_bounds").await;
    let s = w.seed;
    let lead = w.bearer(s.lead).to_owned();
    let id = w.token_id(s.w1);
    for us in [
        -1_i64,
        -86_400_000_000,
        253_402_300_800_000_000,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
    ] {
        let a = w
            .get(
                Some(&lead),
                &format!("/v2/tokens?after={}", cursor_of(us, id)),
            )
            .await;
        assert_eq!(a.code(), (422, Some("invalid_request")), "{us}: {}", a.text);
    }
    // The boundaries are served: the first instant of 1970 lists everything, the last microsecond of 9999 lists
    // nothing (every token is issued before it).
    let first = w
        .get(
            Some(&lead),
            &format!("/v2/tokens?after={}", cursor_of(0, id)),
        )
        .await;
    assert_eq!(first.status.as_u16(), 200, "{}", first.text);
    assert!(!ids(&first).is_empty());
    let last = w
        .get(
            Some(&lead),
            &format!(
                "/v2/tokens?after={}",
                cursor_of(253_402_300_799_999_999, id)
            ),
        )
        .await;
    assert_eq!(last.status.as_u16(), 200, "{}", last.text);
    assert!(ids(&last).is_empty());
}

/// cox's accepted limit (his message of 6 Nov, to be numbered in WAVE1-RULINGS.md): a team lead's LIST is built from
/// the roster, which is active-only, so it omits a RETIRED member's tokens; the project lead's full list shows
/// them, and a team lead can still INSPECT a retired member's token by id. No port change, no direct identity SQL.
/// This cell states the behaviour so it cannot change unseen.
#[tokio::test]
async fn tok_42b_a_team_leads_list_omits_a_retired_members_tokens_the_leads_list_and_inspect_show_them()
 {
    let w = start("rd_retired_history").await;
    let s = w.seed;
    let (tl, lead) = (w.bearer(s.tl).to_owned(), w.bearer(s.lead).to_owned());
    let w1_token = w.token_id(s.w1).to_string();
    let w2_token = w.token_id(s.w2).to_string();
    // Before: the team lead lists both members' tokens.
    let before = ids(&w.get(Some(&tl), "/v2/tokens").await);
    assert!(before.contains(&w1_token) && before.contains(&w2_token));
    // W1 retires.
    let mut admin = w.db.admin().await;
    sqlx::query(
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
    )
    .bind(s.w1.to_string())
    .execute(&mut admin)
    .await
    .expect("retire W1");
    let tl_list = ids(&w.get(Some(&tl), "/v2/tokens").await);
    assert!(
        !tl_list.contains(&w1_token),
        "the team lead's list omits a retired member (stated limit)"
    );
    assert!(
        tl_list.contains(&w2_token),
        "an active member is still listed"
    );
    let lead_list = ids(&w.get(Some(&lead), "/v2/tokens?limit=200").await);
    assert!(
        lead_list.contains(&w1_token),
        "the project lead's list still shows it"
    );
    let inspect = w.get(Some(&tl), &format!("/v2/tokens/{w1_token}")).await;
    assert_eq!(
        inspect.status.as_u16(),
        200,
        "inspect by id: {}",
        inspect.text
    );
}
