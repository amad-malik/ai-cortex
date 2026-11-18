fn needs_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

fn main() {
    needs_zeroize_on_drop::<cortex_client::Token>();
}
