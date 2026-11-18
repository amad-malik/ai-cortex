// F1-03: Secret implements zeroize::ZeroizeOnDrop (a compile-pass trait-bound assertion).
fn needs_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

fn main() {
    needs_zeroize_on_drop::<cortex_types::Secret>();
}
