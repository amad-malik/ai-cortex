fn needs_serialize<T: serde::Serialize>() {}

fn main() {
    needs_serialize::<cortex_client::Token>();
}
