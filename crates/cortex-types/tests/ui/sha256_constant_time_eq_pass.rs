// F1A-R2-03: Sha256 implements subtle::ConstantTimeEq, and `==` agrees with it.
use subtle::ConstantTimeEq;

fn needs_ct_eq<T: ConstantTimeEq>() {}

fn main() {
    needs_ct_eq::<cortex_types::Sha256>();
    let a = cortex_types::Sha256([1; 32]);
    let b = cortex_types::Sha256([2; 32]);
    // The trait's method, called explicitly: the inherent `ct_eq` must not stand in for it.
    assert!(bool::from(ConstantTimeEq::ct_eq(&a, &a)));
    assert!(!bool::from(ConstantTimeEq::ct_eq(&a, &b)));
    assert!(a == a && a != b);
}
