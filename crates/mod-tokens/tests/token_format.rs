//! Stage A (DB-free) cells for the token's format, generation and digest: TOK-01 to TOK-03, TOK-72, TOK-73 and
//! the OS-randomness-only condition (cox W1-32). Written from `mvp/tests/mod-tokens.md` r5. RED: every function
//! under test is `todo!()`.
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cortex_types::{Clock, Secret, TestClock, Timestamp, TokenId};
use mod_tokens::token::{self, EntropyError, TOKEN_LEN};
use sha2::{Digest, Sha256};

const NOW: i64 = 1_800_000_000_123_456;

fn clock() -> TestClock {
    TestClock::at(Timestamp::from_unix_micros(NOW))
}

/// The gitleaks rule `ctx2_[0-9a-f]{32}[.][A-Za-z0-9_-]{43}`, written out as an oracle that shares no code with
/// the crate: the whole string, nothing before or after.
fn is_token_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == TOKEN_LEN
        && s.starts_with("ctx2_")
        && b[5..37]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        && b[37] == b'.'
        && b[38..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

fn hex_to_hyphenated(h: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[test]
fn tok_01_generated_tokens_have_the_format_the_id_the_clock_and_a_32_byte_secret() {
    let c = clock();
    let mut secrets = std::collections::HashSet::new();
    for _ in 0..2000 {
        let g = token::generate(&c).expect("the OS supplies randomness");
        let full = g.token().expose().to_owned();
        assert!(is_token_shaped(&full), "format");
        let hex = &full[5..37];
        assert_eq!(
            hex_to_hyphenated(hex),
            g.id().to_string(),
            "the hex part IS the token id"
        );
        let id = TokenId::parse(&g.id().to_string()).expect("a UUIDv7");
        assert_eq!(
            id.timestamp().unix_millis(),
            c.now().unix_millis(),
            "the id's time prefix is the injected clock's milliseconds"
        );
        let secret = &full[38..];
        let bytes = URL_SAFE_NO_PAD
            .decode(secret)
            .expect("the secret is base64url without padding");
        assert_eq!(bytes.len(), 32, "exactly 32 bytes");
        assert!(
            secrets.insert(secret.to_owned()),
            "no two secrets are equal"
        );
    }
}

#[test]
fn tok_01b_the_secret_is_exactly_the_bytes_the_source_gave() {
    let c = clock();
    let g = token::generate_with(&c, &mut |buf: &mut [u8]| {
        assert_eq!(
            buf.len(),
            32,
            "the generator asks for 32 bytes, no more, no less"
        );
        buf.fill(0xAB);
        Ok(())
    })
    .expect("generated");
    let full = g.token().expose();
    assert_eq!(
        &full[38..],
        URL_SAFE_NO_PAD.encode([0xAB_u8; 32]),
        "the secret is the source's bytes, base64url, with no mixing and no padding"
    );
}

#[test]
fn tok_01c_a_failing_source_refuses_and_there_is_no_fallback() {
    let c = clock();
    let mut calls = 0;
    let r = token::generate_with(&c, &mut |_buf: &mut [u8]| {
        calls += 1;
        Err(EntropyError)
    });
    assert!(matches!(r, Err(EntropyError)), "fail closed");
    assert_eq!(
        calls, 1,
        "one request to the source, and no second source tried"
    );
}

#[test]
fn tok_02_malformed_bearers_are_malformed() {
    let c = clock();
    let good = token::generate(&c)
        .expect("generated")
        .token()
        .expose()
        .to_owned();
    let (head, secret) = good.split_once('.').expect("a dot");
    let hex = &head[5..];
    let mut table: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("prefix alone", "ctx2_".into()),
        ("ctx1", good.replacen("ctx2_", "ctx1_", 1)),
        ("uppercase prefix", good.replacen("ctx2_", "CTX2_", 1)),
        ("hex of 31", format!("ctx2_{}.{secret}", &hex[..31])),
        ("hex of 33", format!("ctx2_{}0.{secret}", hex)),
        (
            "uppercase hex",
            format!("ctx2_{}.{secret}", hex.to_uppercase()),
        ),
        ("non-hex", format!("ctx2_{}g.{secret}", &hex[..31])),
        ("no dot", format!("ctx2_{hex}{secret}")),
        ("two dots", format!("ctx2_{hex}.{secret}.{secret}")),
        ("secret of 42", format!("ctx2_{hex}.{}", &secret[..42])),
        ("secret of 44", format!("ctx2_{hex}.{secret}A")),
        ("secret with +", format!("ctx2_{hex}.{}+", &secret[..42])),
        ("secret with /", format!("ctx2_{hex}.{}/", &secret[..42])),
        ("secret with =", format!("ctx2_{hex}.{}=", &secret[..42])),
        ("embedded space", good.replacen('.', ". ", 1)),
        ("embedded tab", good.replacen('.', ".\t", 1)),
        ("embedded newline", format!("{good}\n")),
        ("embedded NUL", format!("{}\0{}", &good[..20], &good[20..])),
        ("leading space", format!(" {good}")),
        ("trailing space", format!("{good} ")),
        ("cyrillic look-alike", good.replacen('c', "\u{0441}", 1)),
        ("10,000 bytes", "A".repeat(10_000)),
    ];
    // A version nibble that is not 7: flip the 13th hex digit (the UUID's version) of a good id.
    let mut v = hex.to_owned();
    v.replace_range(12..13, "4");
    table.push(("version nibble 4", format!("ctx2_{v}.{secret}")));
    for (what, bearer) in table {
        assert!(token::parse(&bearer).is_err(), "{what} must be Malformed");
    }
    let ok = token::parse(&good).expect("the well-formed token parses");
    assert_eq!(ok.digest(), token::digest_of(&good));
}

#[test]
fn tok_02b_a_non_canonical_spelling_of_the_secret_is_malformed() {
    // 32 bytes encode to 43 characters whose last one carries 4 bits and 2 stray bits. The canonical token has
    // the stray bits zero; the next symbol of the alphabet sets one, and decodes to the same 32 bytes.
    let g = token::generate_with(&clock(), &mut |b: &mut [u8]| {
        b.fill(0xAB);
        Ok(())
    })
    .expect("generated");
    let full = g.token().expose().to_owned();
    assert!(token::parse(&full).is_ok());
    const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let last = full.chars().last().expect("a last character");
    let at = ALPHABET.find(last).expect("in the alphabet");
    assert_eq!(
        at % 4,
        0,
        "the canonical last symbol has two zero stray bits"
    );
    let other = ALPHABET.chars().nth((at + 1) % 64).expect("next symbol");
    let tampered = format!("{}{other}", &full[..full.len() - 1]);
    assert!(token::parse(&tampered).is_err(), "one spelling per secret");
}

#[test]
fn tok_03_the_digest_is_sha256_of_the_whole_string() {
    // The FIPS 180 vector, so the digest function is SHA-256 and not something that merely agrees with a crate.
    let abc = token::digest_of("abc");
    assert_eq!(
        abc.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let c = clock();
    let g = token::generate(&c).expect("generated");
    let full = g.token().expose();
    let want: [u8; 32] = Sha256::digest(full.as_bytes()).into();
    assert_eq!(g.digest(), want, "the digest of the returned full string");
    let secret_only: [u8; 32] = Sha256::digest(&full.as_bytes()[38..]).into();
    let id_only: [u8; 32] = Sha256::digest(&full.as_bytes()[5..37]).into();
    assert_ne!(
        g.digest(),
        secret_only,
        "not the digest of the secret alone"
    );
    assert_ne!(g.digest(), id_only, "not the digest of the id alone");
    let h = token::generate(&c).expect("generated");
    assert_ne!(g.digest(), h.digest(), "two tokens have different digests");
}

#[test]
fn tok_72_debug_output_never_shows_a_secret() {
    let c = clock();
    let g = token::generate(&c).expect("generated");
    let full = g.token().expose().to_owned();
    let parsed = token::parse(&full).expect("parses");
    let digest_hex: String = g.digest().iter().map(|b| format!("{b:02x}")).collect();
    for (what, text) in [
        ("Generated {:?}", format!("{g:?}")),
        ("Generated {:#?}", format!("{g:#?}")),
        ("Parsed {:?}", format!("{parsed:?}")),
        ("Parsed {:#?}", format!("{parsed:#?}")),
        (
            "Error {:?}",
            format!("{:?} {:?}", EntropyError, token::parse("x").err()),
        ),
    ] {
        assert!(!text.contains(&full), "{what} shows the token");
        assert!(!text.contains(&full[38..]), "{what} shows the secret");
        for k in [0, 10, 25] {
            assert!(
                !text.contains(&full[38 + k..38 + k + 16]),
                "{what} shows a slice of the secret"
            );
        }
        assert!(!text.contains(&digest_hex), "{what} shows the digest");
    }
    // A bare `Secret` has no Debug or Display at all (F1-03 pins that in cortex-types); here it only has to exist.
    let _: &Secret = g.token();
}

#[test]
fn tok_73_the_gitleaks_pattern_matches_generated_tokens_and_nothing_near_misses() {
    let c = clock();
    for _ in 0..1000 {
        let g = token::generate(&c).expect("generated");
        assert!(is_token_shaped(g.token().expose()));
    }
    let good = token::generate(&c)
        .expect("generated")
        .token()
        .expose()
        .to_owned();
    assert!(
        !is_token_shaped(&good[..good.len() - 1]),
        "a 42-character secret"
    );
    assert!(
        !is_token_shaped(&format!("ctx2_{}.{}", "A".repeat(32), &good[38..])),
        "uppercase hex"
    );
    assert!(!is_token_shaped(&good[5..]), "a missing prefix");
}
