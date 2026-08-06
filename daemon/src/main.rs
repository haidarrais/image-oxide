//! image-oxide daemon entrypoint. Owns the socket lifecycle and the worker
//! pool; all protocol and pixel work lives in the library.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image_oxide::lifecycle;

/// `LIFE-14`: SIGTERM flips this flag; the run loop observes it and shuts down
/// gracefully (drain in-flight work, remove the socket, exit 0).
static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

fn install_sigterm_handler() {
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        act.sa_sigaction = handle_sigterm as extern "C" fn(libc::c_int) as usize;
        act.sa_flags = 0;
        libc::sigemptyset(&mut act.sa_mask);
        libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
    }
}

fn parse_ttl() -> Duration {
    std::env::var("IMAGE_OXIDE_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(lifecycle::DEFAULT_IDLE_TTL)
}

fn parse_queue() -> usize {
    std::env::var("IMAGE_OXIDE_QUEUE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(lifecycle::DEFAULT_QUEUE_CAPACITY)
}

fn main() -> ExitCode {
    install_sigterm_handler();
    let ttl = parse_ttl();
    let queue = parse_queue();

    match lifecycle::run_forever(queue, ttl, &TERMINATE) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("image-oxide: fatal: {e}");
            ExitCode::FAILURE
        }
    }
}
