fn needs_clone<T: Clone>() {}

fn main() {
    needs_clone::<cortex_client::Token>();
}
