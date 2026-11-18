//! BOOT-40 to BOOT-43, BOOT-45, BOOT-46 (`mvp/tests/mod-boot.md`, Budget, determinism, encoding).
//! DB-free. BOOT-44 is in `boot_orientation.rs`.
mod support;

use axum::extract::FromRequestParts;
use axum::http::Request;
use cortex_module::StrictQuery;
use mod_boot::args::{MAX_BUDGET, MIN_BUDGET, check};
use mod_boot::{BootQuery, Format, Inputs, assemble};
use support::*;

/// The invariants of one briefing (BOOT-40).
fn invariants(i: &Inputs, budget: u32) {
    let b = assemble(i, budget).unwrap_or_else(|n| panic!("budget {budget}: {n:?}"));
    let used = b.markdown.len();
    assert!(used <= budget as usize, "budget {budget}: used {used}");
    assert_eq!(b.boot.used, used, "budget {budget}");
    assert_eq!(b.boot.budget, budget);
    assert_eq!(
        headings(&b.markdown).len(),
        12,
        "budget {budget}: every section is present"
    );
    let s = &b.boot.sections;
    assert_eq!(s.len(), 12);
    assert!(
        s.iter().map(|x| x.used).sum::<usize>() <= used,
        "budget {budget}"
    );
    for (k, n) in [(0usize, 1u32), (1, 2), (2, 3), (3, 4), (4, 5)] {
        assert_eq!(
            s[k].used,
            section_text(&b.markdown, n).len(),
            "budget {budget}: section {n}"
        );
    }
    let claimed = i
        .handoffs
        .iter()
        .filter(|h| h.state == mod_boot::HState::Claimed)
        .count();
    let pending = i.handoffs.len() - claimed;
    assert_eq!(
        s[3].omitted,
        claimed - b.boot.claimed.len(),
        "budget {budget}"
    );
    assert_eq!(
        s[4].omitted,
        pending - b.boot.pending.len(),
        "budget {budget}"
    );
    assert_eq!(
        s[0].omitted,
        i.identity.job_roles.len() - b.boot.identity.job_roles.len()
    );
    if claimed > 0 {
        assert!(
            !b.boot.claimed.is_empty(),
            "budget {budget}: the claimed list keeps one item"
        );
    }
}

/// Every budget from 4 KiB to 256 KiB, on the worst case and on W1's ordinary state.
///
/// Byte by byte up to `stop`, the budget from which the output can no longer change: the whole
/// state fits (the size at 256 KiB) and the 40 % orientation cap has reached the 3 KiB body cap
/// (7,680 bytes). Above `stop`, every budget's invariants still hold because the Markdown is the
/// same bytes as at 256 KiB; that equality is checked on a stride of 509 bytes and at 256 KiB.
/// (Every one of the 258,049 budgets in a debug build took 254 s on build-host-1, 6 Nov 12:12Z.)
#[test]
fn boot_40_every_budget_from_4_to_256_kib() {
    for i in [worst_case(), inputs_w1()] {
        let top = assemble(&i, MAX_BUDGET).expect("fits").markdown;
        let stop = u32::try_from(top.len())
            .expect("small")
            .clamp(7_680, MAX_BUDGET);
        for budget in MIN_BUDGET..=stop {
            invariants(&i, budget);
        }
        for budget in (stop..=MAX_BUDGET).step_by(509).chain([MAX_BUDGET]) {
            invariants(&i, budget);
            assert_eq!(
                assemble(&i, budget).expect("fits").markdown,
                top,
                "budget {budget}: the output stopped changing at {stop}"
            );
        }
    }
}

#[test]
fn boot_41_out_of_range_budgets_are_422() {
    let below = check(&BootQuery {
        budget: Some(4_095),
        format: None,
    })
    .expect_err("below the minimum");
    assert_eq!(
        (below.status, below.code, below.detail.as_ref()),
        (
            422,
            "boot_budget_too_small",
            "needed 4096 bytes, budget 4095 bytes"
        )
    );
    let zero = check(&BootQuery {
        budget: Some(0),
        format: None,
    })
    .expect_err("zero");
    assert_eq!(zero.detail.as_ref(), "needed 4096 bytes, budget 0 bytes");
    let over = check(&BootQuery {
        budget: Some(262_145),
        format: None,
    })
    .expect_err("over the maximum");
    assert_eq!((over.status, over.code), (422, "invalid_request"));
    let huge = check(&BootQuery {
        budget: Some(i64::MAX),
        format: None,
    })
    .expect_err("far over the maximum");
    assert_eq!((huge.status, huge.code), (422, "invalid_request"));
    for ok in [4_096i64, 262_144] {
        assert_eq!(
            check(&BootQuery {
                budget: Some(ok),
                format: Some(Format::Json)
            })
            .map(|a| (a.budget, a.format)),
            Ok((ok as u32, Format::Json))
        );
    }
}

/// The worst case at the floor: the claimed list is the newest handoff, its summary cut to 120
/// bytes ending `…`, then `9 more`; the job roles are the first plus `(+9 more)`; both rules
/// layers stay whole. At 12 KiB the claimed list is the newest items that fit, whole.
#[test]
fn boot_42_the_floor_forms_on_the_worst_case() {
    let i = worst_case();
    let newest = uuid(T_HANDOFF, 109);
    for budget in (4_096..=5_100).chain([12_288]) {
        let b = assemble(&i, budget).expect("fits");
        let s3 = section_text(&b.markdown, 3);
        assert!(
            s3.contains(&format!(
                "{}\n",
                i.rules_project
                    .as_ref()
                    .map(|r| r.fields["source_path"].as_str().expect("path").to_owned())
                    .expect("set")
            )),
            "budget {budget}"
        );
        assert!(
            s3.contains(
                i.rules_worker
                    .as_ref()
                    .map(|r| r.fields["source_path"].as_str().expect("path"))
                    .expect("set")
            ),
            "budget {budget}"
        );
        let c = &b.boot.claimed;
        assert_eq!(c[0].id, newest, "budget {budget}: the newest first");
        let s4 = section_text(&b.markdown, 4);
        assert!(
            s4.ends_with(&format!(
                "{} more: cortex handoff list --mine\n",
                10 - c.len()
            )),
            "budget {budget}"
        );
        if budget <= 5_100 {
            assert_eq!(c.len(), 1, "budget {budget}");
            assert!(c[0].summary_cut && c[0].summary.len() <= 120 && c[0].summary.ends_with('…'));
            let first = &i.identity.job_roles[0];
            assert!(
                section_text(&b.markdown, 1).contains(&format!("job roles: {first} (+9 more)\n"))
            );
            assert_eq!(b.boot.identity.job_roles, std::slice::from_ref(first));
        } else {
            assert!(
                (1..10).contains(&c.len()),
                "budget {budget}: cut to the newest that fit"
            );
            assert!(
                c.iter().all(|h| !h.summary_cut),
                "budget {budget}: whole summaries"
            );
        }
    }
}

#[test]
fn boot_43_the_same_state_gives_the_same_bytes() {
    for i in [inputs_w1(), worst_case()] {
        for budget in [4_096, 12_288, 262_144] {
            let a = assemble(&i, budget).expect("fits");
            let b = assemble(&i, budget).expect("fits");
            assert_eq!(a.markdown.as_bytes(), b.markdown.as_bytes());
            assert_eq!(
                serde_json::to_vec(&a.boot).expect("json"),
                serde_json::to_vec(&b.boot).expect("json")
            );
        }
    }
}

/// The seam: `assemble` itself takes any budget, so one below the floor shows the guard. It is
/// 422 with `needed` (the smallest briefing) and `budget` in `detail`, never a partial briefing;
/// and `needed` is exact: that budget fits and one byte less does not.
#[test]
fn boot_45_the_floor_guard_refuses_and_names_what_it_needs() {
    let i = worst_case();
    let n = assemble(&i, 2_048).expect_err("the floor is above 2048");
    assert_eq!(n.budget, 2_048);
    assert!(n.needed > 2_048);
    let p = n.problem();
    assert_eq!((p.status, p.code), (422, "boot_budget_too_small"));
    assert_eq!(
        p.detail.as_ref(),
        format!("needed {} bytes, budget 2048 bytes", n.needed)
    );
    let exact = u32::try_from(n.needed).expect("small");
    assert!(assemble(&i, exact).is_ok(), "needed fits");
    assert_eq!(
        assemble(&i, exact - 1).map(|_| ()),
        Err(mod_boot::Needed {
            needed: n.needed,
            budget: i64::from(exact - 1)
        })
    );
    assert!(
        n.needed < MIN_BUDGET as usize,
        "the admitted worst case fits at 4 KiB"
    );
}

/// sam's BOOT12-02: S3 types `budget` as an integer with no schema minimum, and below the floor
/// is `boot_budget_too_small` naming the minimum: a negative integer decodes, then the range check
/// answers. A value that is not an integer is still 422 `invalid_request` at decoding.
#[tokio::test]
async fn boot_41_a_negative_budget_is_below_the_floor() {
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
    let q = decode("/v2/boot?budget=-1")
        .await
        .expect("a negative integer decodes (S3: integer, no schema minimum)");
    let e = check(&q).expect_err("below the floor");
    assert_eq!(
        (e.status, e.code, e.detail.as_ref()),
        (
            422,
            "boot_budget_too_small",
            "needed 4096 bytes, budget -1 bytes"
        )
    );
    for bad in [
        "/v2/boot?budget=1.5",
        "/v2/boot?budget=abc",
        "/v2/boot?budget=",
    ] {
        assert_eq!(decode(bad).await, Err((422, "invalid_request")), "{bad}");
    }
}

#[test]
fn boot_46_no_budget_means_12288() {
    assert_eq!(
        check(&BootQuery::default()).map(|a| (a.budget, a.format)),
        Ok((12_288, Format::Md))
    );
}
