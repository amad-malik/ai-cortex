//! `Sha256`: a 32-byte digest; hex Display; constant-time equality (CONTRACTS §2).

#[derive(Clone, Copy)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    /// Lowercase hex, 64 characters.
    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
        s
    }

    /// Equality through `subtle::ConstantTimeEq` (mike's F1A-R2-03). That is the `subtle` crate's
    /// best effort: it compares every byte and routes the result through its optimisation barrier.
    /// The compiler gives no timing guarantee, and no timing test is claimed here; the tests check
    /// correctness and that this primitive is the one used.
    pub fn ct_eq(&self, other: &Self) -> bool {
        subtle::ConstantTimeEq::ct_eq(self, other).into()
    }
}

impl subtle::ConstantTimeEq for Sha256 {
    fn ct_eq(&self, other: &Self) -> subtle::Choice {
        // subtle's slice impl: every byte, folded into a `Choice` (lengths are equal, 32).
        subtle::ConstantTimeEq::ct_eq(&self.0[..], &other.0[..])
    }
}

/// `==` is the constant-time comparison.
impl PartialEq for Sha256 {
    fn eq(&self, other: &Self) -> bool {
        Sha256::ct_eq(self, other)
    }
}

impl Eq for Sha256 {}

impl std::fmt::Display for Sha256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl std::fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sha256({})", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_64_characters() {
        let mut b = [0_u8; 32];
        b[0] = 0xab;
        b[31] = 0x0f;
        let h = Sha256(b).to_hex();
        assert_eq!(h.len(), 64);
        assert!(h.starts_with("ab00"));
        assert!(h.ends_with("000f"));
        assert_eq!(format!("{}", Sha256(b)), h);
    }

    #[test]
    fn ct_eq_finds_a_difference_in_any_byte() {
        let a = Sha256([7; 32]);
        assert!(a.ct_eq(&Sha256([7; 32])));
        assert_eq!(a, Sha256([7; 32]));
        for i in 0..32 {
            let mut b = [7_u8; 32];
            b[i] ^= 1;
            assert!(!a.ct_eq(&Sha256(b)), "a difference at byte {i} was missed");
        }
    }
}
