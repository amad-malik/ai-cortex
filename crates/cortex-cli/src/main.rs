//! The `cortex` binary: arguments from the process, the environment through `cortex_client::env::process_env`.
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let code = cortex_cli::run(
        args,
        &cortex_client::env::process_env,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    std::process::exit(i32::from(code));
}
