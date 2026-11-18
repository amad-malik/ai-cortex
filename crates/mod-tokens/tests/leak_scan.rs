//! Stage D, L3: the SECRET LEAK SCAN (TOK-70) and the proof that the scanner can fail (TOK-71). Needs the throwaway
//! Postgres (`CV2_DB=1`).
//!
//! **Sinks:** every HTTP response (status line, headers, body), every captured log line, and every row of every
//! table of every schema as text (`platform.idempotency.response` and the audit `details` included).
//! **Searched:** each full token, each 43-character secret, three 16-character slices of each secret, and the
//! token pattern `ctx2_[0-9a-f]{32}[.][A-Za-z0-9_-]{43}`.
//! **Expectation:** each issued secret occurs EXACTLY ONCE in total, in the BODY of its one 201 response (a response
//! is two sinks, its headers and its body; for the host path, in `issue_as_host`'s return value, so zero times in
//! any sink). Any other hit fails and names the
//! sink, never printing the secret.
mod common;

use axum::http::Method;
use common::chain::{Answer, World, start};
use cortex_db::HostContext;
use cortex_types::{RequestId, SystemClock};
use mod_tokens::host::issue_as_host;
use mod_tokens::{BootstrapRun, OsEntropy};

/// `ctx2_[0-9a-f]{32}[.][A-Za-z0-9_-]{43}`, written out, searched for in `text`: how many token-shaped strings.
fn token_shaped(text: &str) -> usize {
    let b = text.as_bytes();
    let mut n = 0;
    let mut i = 0;
    while let Some(at) = text[i..].find("ctx2_") {
        let s = i + at;
        let ok = b.get(s..s + 81).is_some_and(|w| {
            w[5..37]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
                && w[37] == b'.'
                && w[38..]
                    .iter()
                    .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
        });
        if ok {
            n += 1;
        }
        i = s + 5;
    }
    n
}

/// A named piece of text where a secret could end up.
type Sink = (String, String);

/// A response is TWO sinks: its status line with its headers, and its body (vera's TOKENS-L1). The allowed place
/// of a secret is the BODY of its own 201, and nowhere in any header: one sink per part, so a secret planted in the
/// 201's own header is a second hit and not the same allowed occurrence.
fn response_sinks(a: &Answer) -> [Sink; 2] {
    let mut head = format!("{}\n", a.status);
    for (k, v) in &a.headers {
        head.push_str(&format!("{}: {}\n", k, v.to_str().unwrap_or("")));
    }
    let code = a.status.as_u16();
    [
        (format!("response {code} headers"), head),
        (format!("response {code} body"), a.text.clone()),
    ]
}

async fn table_sinks(w: &World) -> Vec<Sink> {
    let mut admin = w.db.admin().await;
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_schema::text, table_name::text FROM information_schema.tables \
         WHERE table_schema NOT IN ('pg_catalog', 'information_schema') AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&mut admin)
    .await
    .expect("tables");
    let mut out = Vec::new();
    for (schema, table) in tables {
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT t::text FROM {schema}.{table} t"
        )))
        .fetch_all(&mut admin)
        .await
        .expect("rows");
        for r in rows {
            out.push((format!("table {schema}.{table}"), r));
        }
    }
    out
}

fn log_sinks(w: &World) -> Vec<Sink> {
    w.logs
        .lock()
        .expect("lock")
        .iter()
        .map(|l| ("log line".to_owned(), l.clone()))
        .collect()
}

/// The needles of one token: the whole string, the 43-character secret, and three 16-character slices of it.
fn needles(token: &str) -> Vec<String> {
    let secret = &token[38..];
    vec![
        token.to_owned(),
        secret.to_owned(),
        secret[0..16].to_owned(),
        secret[13..29].to_owned(),
        secret[27..43].to_owned(),
    ]
}

/// The sinks (by name) that contain any needle of any of `tokens`, without printing a needle.
fn hits(sinks: &[Sink], tokens: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, text) in sinks {
        for t in tokens {
            if needles(t).iter().any(|n| text.contains(n.as_str())) {
                out.push(name.clone());
                break;
            }
        }
    }
    out
}

#[tokio::test]
async fn tok_70_each_secret_occurs_exactly_once_in_the_body_of_its_one_201() {
    let w = start("lk_scan").await;
    let s = w.seed;
    let mut answers: Vec<Answer> = Vec::new();
    // The host path returns the lead's secret to the caller only.
    let lead = issue_as_host(
        &w.pool,
        &*w.identity,
        &*w.audit_sink,
        &*w.clock,
        &OsEntropy,
        HostContext {
            uid: 1000,
            project: Some(s.a),
            request_id: RequestId::new(&SystemClock),
        },
        s.lead,
        BootstrapRun::First,
    )
    .await
    .expect("host issue");
    let lead_token = lead.token.expose().to_owned();
    // L issues for W1 and W2: the two 201 bodies carry the two secrets.
    let i1 = w
        .post(
            &lead_token,
            &format!("/v2/workers/{}/tokens", s.w1),
            serde_json::json!({}),
        )
        .await;
    let i2 = w
        .post(
            &lead_token,
            &format!("/v2/workers/{}/tokens", s.w2),
            serde_json::json!({}),
        )
        .await;
    let (t1, t2) = (
        i1.json["token"].as_str().expect("t1").to_owned(),
        i2.json["token"].as_str().expect("t2").to_owned(),
    );
    answers.extend([i1, i2]);
    let issued = [lead_token.clone(), t1.clone(), t2.clone()];
    // Every path: success, wrong scope, a malformed bearer that holds 16 characters of a secret, 404, 422, 413,
    // 428, a replay, already_issued, a revocation, list, inspect, whoami.
    let key = "key-scan-issue-1";
    answers.push(
        w.post_keyed(
            &lead_token,
            &format!("/v2/workers/{}/tokens", s.w3),
            key,
            "{}",
        )
        .await,
    );
    answers.push(
        w.post_keyed(
            &lead_token,
            &format!("/v2/workers/{}/tokens", s.w3),
            key,
            "{}",
        )
        .await,
    ); // already_issued
    answers.push(w.get(Some(&t1), "/v2/whoami").await);
    answers.push(
        w.post(
            &t1,
            &format!("/v2/workers/{}/tokens", s.w2),
            serde_json::json!({}),
        )
        .await,
    ); // missing_scope
    let partial = format!("not-a-token-{}", &t2[38..54]);
    answers.push(w.get(Some(&partial), "/v2/whoami").await); // malformed, holds 16 characters of a secret
    answers.push(
        w.get(
            Some(&format!("ctx2_{}.{}", &t2[5..37], &t1[38..])),
            "/v2/whoami",
        )
        .await,
    ); // wrong secret
    answers.push(
        w.post(
            &lead_token,
            "/v2/workers/not-a-uuid/tokens",
            serde_json::json!({}),
        )
        .await,
    ); // 404
    answers.push(
        w.post(
            &lead_token,
            &format!("/v2/tokens/{}/revoke", s.w1),
            serde_json::json!({"reason": "lost"}),
        )
        .await,
    ); // 404 (a worker id)
    answers.push(
        w.post(
            &lead_token,
            "/v2/tokens/00000000-0000-7000-8000-000000000000/revoke",
            serde_json::json!({"reason": ""}),
        )
        .await,
    ); // 422
    answers.push(
        w.call(
            Method::POST,
            &format!("/v2/workers/{}/tokens", s.w4),
            Some(&lead_token),
            Some("key-scan-big-1"),
            Some(&"x".repeat(20_000)),
        )
        .await,
    ); // 413
    answers.push(
        w.call(
            Method::POST,
            &format!("/v2/workers/{}/tokens", s.w4),
            Some(&lead_token),
            None,
            Some("{}"),
        )
        .await,
    ); // keyed by next_key; 201
    answers.push(w.get(Some(&lead_token), "/v2/tokens").await);
    let t1_id = t1[5..37].to_owned();
    let hyph = format!(
        "{}-{}-{}-{}-{}",
        &t1_id[0..8],
        &t1_id[8..12],
        &t1_id[12..16],
        &t1_id[16..20],
        &t1_id[20..32]
    );
    answers.push(
        w.post(
            &lead_token,
            &format!("/v2/tokens/{hyph}/revoke"),
            serde_json::json!({"reason": "lost"}),
        )
        .await,
    );
    answers.push(
        w.get(Some(&lead_token), &format!("/v2/tokens/{hyph}"))
            .await,
    );
    answers.push(w.get(Some(&t1), "/v2/whoami").await); // revoked
    // A replay of a stored 200 and the 428 (no key).
    answers.push(
        w.post_keyed(
            &lead_token,
            &format!("/v2/tokens/{hyph}/revoke"),
            "key-scan-rev-001",
            r#"{"reason":"lost"}"#,
        )
        .await,
    );
    answers.push(
        w.post_keyed(
            &lead_token,
            &format!("/v2/tokens/{hyph}/revoke"),
            "key-scan-rev-001",
            r#"{"reason":"lost"}"#,
        )
        .await,
    );
    // Every other 201 of this scenario is an issued token too.
    let mut all_issued = issued.to_vec();
    for a in &answers {
        if a.status.as_u16() == 201 {
            all_issued.push(a.json["token"].as_str().expect("token").to_owned());
        }
    }
    all_issued.sort();
    all_issued.dedup();
    // Collect the sinks.
    let mut sinks: Vec<Sink> = answers.iter().flat_map(response_sinks).collect();
    sinks.extend(log_sinks(&w));
    sinks.extend(table_sinks(&w).await);
    // Each secret occurs exactly once in total: in the body of its own 201 (the host's, in no sink at all).
    for t in &all_issued {
        let found = hits(&sinks, std::slice::from_ref(t));
        if *t == lead_token {
            assert!(
                found.is_empty(),
                "the host path's secret is in a sink: {found:?}"
            );
        } else {
            assert_eq!(
                found,
                vec!["response 201 body".to_owned()],
                "a secret is in another sink than the body of its 201: {found:?}"
            );
        }
    }
    // The token pattern matches nothing but the 201 bodies that carry a token.
    for (name, text) in &sinks {
        let n = token_shaped(text);
        if name == "response 201 body" {
            assert!(n <= 1, "a 201 body holds at most its token");
        } else {
            assert_eq!(n, 0, "a token-shaped string in `{name}`");
        }
    }
}

#[tokio::test]
async fn tok_71_the_scanner_can_fail_a_planted_secret_in_each_sink_is_found_and_named() {
    let w = start("lk_plant").await;
    let s = w.seed;
    let r = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/workers/{}/tokens", s.w1),
            serde_json::json!({}),
        )
        .await;
    let token = r.json["token"].as_str().expect("token").to_owned();
    let [own_head, own_body] = response_sinks(&r);
    // The baseline: only the BODY of the 201 carries it, and its headers carry nothing.
    let mut base: Vec<Sink> = vec![own_head.clone(), own_body.clone()];
    base.extend(log_sinks(&w));
    base.extend(table_sinks(&w).await);
    assert_eq!(
        hits(&base, std::slice::from_ref(&token)),
        vec!["response 201 body".to_owned()]
    );
    // (a) a log line, (c) a response header: planted in the collected sinks.
    let mut with_log = base.clone();
    with_log.push((
        "log line".into(),
        format!("cortex-http: leaked {}", &token[38..]),
    ));
    assert!(hits(&with_log, std::slice::from_ref(&token)).contains(&"log line".to_owned()));
    let mut with_header = base.clone();
    with_header.push((
        "response 403 headers".into(),
        format!("403\nx-debug: {token}\n"),
    ));
    assert_eq!(
        hits(&with_header, std::slice::from_ref(&token))
            .iter()
            .filter(|n| *n == "response 403 headers")
            .count(),
        1
    );
    // (c2) vera's TOKENS-L1: the secret, or a 16-character fragment of it, in the header of the token's OWN 201.
    // Before the split this was the same allowed sink and the scan could not tell; now it is a second hit.
    for planted in [
        token[38..].to_owned(),
        token[38..54].to_owned(),
        token[13 + 38..29 + 38].to_owned(),
        token.clone(),
    ] {
        let mut own_header_leak = vec![
            (
                own_head.0.clone(),
                format!("{}x-debug: {planted}\n", own_head.1),
            ),
            own_body.clone(),
        ];
        own_header_leak.extend(log_sinks(&w));
        let found = hits(&own_header_leak, std::slice::from_ref(&token));
        assert_eq!(
            found,
            vec![
                "response 201 headers".to_owned(),
                "response 201 body".to_owned()
            ],
            "a planted own-201 header is found and named"
        );
    }
    // (b) an audit row and (d) an idempotency row: planted in the real tables.
    let mut admin = w.db.admin().await;
    sqlx::query("INSERT INTO plug_audit.events (project_id, action, actor, target, details) VALUES ($1::uuid, 'x', 'x', 'x', $2)")
        .bind(s.a.to_string())
        .bind(format!("{{\"reason\":\"{}\"}}", &token[38..60]))
        .execute(&mut admin)
        .await
        .expect("plant audit");
    let sinks = table_sinks(&w).await;
    assert!(
        hits(&sinks, std::slice::from_ref(&token)).contains(&"table plug_audit.events".to_owned()),
        "(b)"
    );
    // (d) The issue left an idempotency row (a marker); plant the secret in it. A missing row or a failing update
    // is a failure of this control, not a reason to skip it (vera's TOKENS-L1, second part).
    let planted = sqlx::query("UPDATE platform.idempotency SET response = to_jsonb($1::text)")
        .bind(&token[38..])
        .execute(&mut admin)
        .await
        .expect("plant idempotency")
        .rows_affected();
    assert!(planted > 0, "no idempotency row was there to plant in");
    let sinks = table_sinks(&w).await;
    assert!(
        hits(&sinks, std::slice::from_ref(&token))
            .contains(&"table platform.idempotency".to_owned()),
        "(d)"
    );
    // The pattern matcher finds a token-shaped string in any text, and not a near miss.
    assert_eq!(token_shaped(&format!("a {token} b")), 1);
    assert_eq!(token_shaped(&token[..token.len() - 1]), 0);
}
