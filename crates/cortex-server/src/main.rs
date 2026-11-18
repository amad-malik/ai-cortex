//! The `cortex-server` binary. It parses the command line and runs `serve` (piece 11, mira); `migrate` and the real `bootstrap` store are
//! cox's increments of piece 11.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cortex_server::cli::parse(&args) {
        Ok(cortex_server::cli::Command::Version) => {
            println!(
                "{} {}",
                env!("CARGO_PKG_VERSION"),
                option_env!("CORTEX_GIT_SHA").unwrap_or("unknown")
            );
        }
        Ok(cortex_server::cli::Command::Serve { config }) => {
            std::process::exit(cortex_server::serve::run_command(&config));
        }
        Ok(cortex_server::cli::Command::Migrate { config }) => {
            std::process::exit(cortex_server::migrate_cmd::run_command(&config));
        }
        Ok(cortex_server::cli::Command::Bootstrap(args)) => {
            std::process::exit(cortex_server::bootstrap_cmd::run_command(&args));
        }
        Err(e) => {
            eprintln!("cortex-server: {}", e.0);
            std::process::exit(cortex_server::cli::EXIT_USAGE);
        }
    }
}
