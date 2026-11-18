//! mike's F-1a R0 r2 carry item 3: the module-code grammar `^[a-z][a-z0-9_]{0,63}$`, at its
//! boundaries, and the platform-code refusal for each of the 13. This binary is its own process,
//! so its one test takes the minter.
use cortex_types::{ProblemCodeError, ProblemCodeMinter, ProblemType};

static LEN_64: &str = "abcdefghijklmnopqrstuvwxyz0123456789_abcdefghijklmnopqrstuvwxyz0";
static LEN_65: &str = "abcdefghijklmnopqrstuvwxyz0123456789_abcdefghijklmnopqrstuvwxyz01";

#[test]
fn the_code_grammar_at_its_boundaries() {
    assert_eq!((LEN_64.len(), LEN_65.len()), (64, 65));
    let m = ProblemCodeMinter::take().expect("this process's minter");
    for ok in ["a", "a1", "a_", "z9_x", "target_retired", LEN_64] {
        let t = m.mint(ok).unwrap_or_else(|e| panic!("{ok:?} refused: {e}"));
        assert_eq!(t.code(), ok, "the code keeps its exact text");
        assert!(!t.is_platform());
    }
    for bad in [
        "", "A", "aB", "1a", "_a", "a-b", "a b", "a.b", "é", "aé", "a\u{0}", LEN_65,
    ] {
        assert!(
            matches!(m.mint(bad), Err(ProblemCodeError::BadCode(_))),
            "{bad:?} was not refused as a bad code"
        );
    }
    for p in ProblemType::PLATFORM {
        assert_eq!(
            m.mint(p.code()),
            Err(ProblemCodeError::PlatformCode(p.code())),
            "the platform code {} was minted",
            p.code()
        );
    }
}
