fn needs_display<T: std::fmt::Display>() {}

fn main() {
    needs_display::<cortex_client::Token>();
}
