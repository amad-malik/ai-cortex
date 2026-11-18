//! `serve::shutdown_signal` and SIGTERM: the future completes. Its own test binary: a signal handler is process-wide, and the point is that the
//! signal arrives IMMEDIATELY after the call, before any future has been polled (the handlers are installed when the function is called).
//! The workspace forbids `unsafe`, so the signal is sent with `kill` to this process's OWN exact pid. DB-free.
use std::process::Command;
use std::time::Duration;

use cortex_server::serve::shutdown_signal;

#[tokio::test]
async fn sg_term_completes_the_future_even_when_it_arrives_right_after_the_call() {
    // First thing in a fresh process: install, signal at once, only then poll.
    let fut = shutdown_signal().expect("handlers install");
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(std::process::id().to_string())
        .status()
        .expect("kill runs");
    assert!(status.success(), "kill -TERM on our own pid");
    tokio::time::timeout(Duration::from_secs(2), fut)
        .await
        .expect("SIGTERM did not complete the future");

    // The control: with no further signal a fresh future stays pending.
    let quiet = shutdown_signal().expect("handlers install");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), quiet)
            .await
            .is_err(),
        "no signal, no completion"
    );
}
