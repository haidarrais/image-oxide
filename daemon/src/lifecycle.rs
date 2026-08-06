//! Daemon lifecycle per 002: per-UID socket resolution, atomic bind, worker
//! pool, bounded queue, idle shutdown. The concurrency model is a fixed-size
//! worker pool over a bounded FIFO queue (`LIFE-09`–`LIFE-11`): the accept
//! thread enqueues, a fixed pool of workers consumes. No per-connection threads,
//! no unbounded buffering.

use std::collections::VecDeque;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{ErrorCode, OxideError};
use crate::frame::{read_frame, write_frame};
use crate::ops;
use crate::protocol::{parse_hello, parse_request, Ack, Failure, Success};

/// `LIFE-12`: idle shutdown timeout, overridable via `IMAGE_OXIDE_TTL_MS`
/// (e.g. tests shrink it).
pub const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(60);

/// `LIFE-11`: queue capacity, overridable via `IMAGE_OXIDE_QUEUE`.
pub const DEFAULT_QUEUE_CAPACITY: usize = 32;

// ---- socket resolution (`LIFE-01`–`LIFE-04`) ----

/// Resolve in order: `$XDG_RUNTIME_DIR/image-oxide.sock`, else
/// `/tmp/image-oxide-$UID.sock`. Must be identical to the client's resolution.
pub fn resolve_socket_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("image-oxide.sock");
        }
    }
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/image-oxide-{uid}.sock"))
}

/// The uid that owns the socket — `LIFE-05` per-UID confinement.
pub fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

// ---- bounded work queue (`LIFE-10`, `LIFE-11`) ----

/// Shared FIFO with a condvar for worker notification. `shutdown` wakes waiters
/// when set so workers exit promptly.
struct WorkQueue {
    queue: Mutex<VecDeque<UnixStream>>,
    signal: Condvar,
    shutdown: AtomicBool,
}

impl WorkQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            signal: Condvar::new(),
            shutdown: AtomicBool::new(false),
        }
    }

    fn push(&self, stream: UnixStream) {
        let mut q = self.queue.lock().unwrap();
        q.push_back(stream);
        self.signal.notify_one();
    }

    /// Block until an item is available or shutdown is requested.
    fn pop(&self) -> Option<UnixStream> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(s) = q.pop_front() {
                return Some(s);
            }
            if self.shutdown.load(Ordering::SeqCst) {
                return None;
            }
            q = self.signal.wait(q).unwrap();
        }
    }

    fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.signal.notify_all();
    }
}

// ---- worker pool (`LIFE-09`–`LIFE-11`) ----

fn run_worker_pool(
    incoming: Receiver<UnixStream>,
    queue_capacity: usize,
    idle_ttl: Duration,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    let pool = std::cmp::min(4, available_parallelism());
    let queue = Arc::new(WorkQueue::new());
    let active = Arc::new(AtomicUsize::new(0));

    let mut workers: Vec<thread::JoinHandle<()>> = Vec::with_capacity(pool);
    for _ in 0..pool {
        let queue = Arc::clone(&queue);
        let active = Arc::clone(&active);
        workers.push(thread::spawn(move || {
            while let Some(stream) = queue.pop() {
                active.fetch_add(1, Ordering::SeqCst);
                handle_connection(stream);
                active.fetch_sub(1, Ordering::SeqCst);
            }
        }));
    }

    let mut last_activity = Instant::now();
    loop {
        match incoming.recv_timeout(Duration::from_millis(100)) {
            Ok(stream) => {
                if queue_len(&queue) >= queue_capacity {
                    reject_overloaded(stream);
                } else {
                    queue.push(stream);
                }
                last_activity = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {
                let idle = last_activity.elapsed() >= idle_ttl;
                if shutdown.load(Ordering::Relaxed)
                    || (idle && active.load(Ordering::SeqCst) == 0 && queue.is_empty())
                {
                    queue.shutdown();
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                queue.shutdown();
                break;
            }
        }
    }

    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn queue_len(queue: &Arc<WorkQueue>) -> usize {
    queue.queue.lock().unwrap().len()
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// `LIFE-10`: full queue → `DAEMON_OVERLOADED`, never unbounded buffering.
fn reject_overloaded(mut stream: UnixStream) {
    let err = OxideError::new(
        ErrorCode::DaemonOverloaded,
        "worker pool and queue are full (`LIFE-10`)",
    );
    let failure = Failure {
        id: "".into(),
        status: "error",
        error: err,
    };
    if let Ok(body) = serde_json::to_vec(&failure) {
        let _ = write_frame(&mut stream, &body);
    }
}

// ---- connection handling (`IPC-06`–`IPC-24`) ----

fn handle_connection(mut stream: UnixStream) {
    // `IPC-06`: first message must be `hello`. A pre-ack request is
    // `INVALID_REQUEST` (`IPC-09`).
    let hello_body = match read_frame(&mut stream) {
        Ok(Some(body)) => body,
        Ok(None) | Err(_) => return,
    };
    let hello = match parse_hello(&hello_body) {
        Ok(h) => h,
        Err(e) => return write_failure(&mut stream, "", e),
    };

    // `IPC-07`/`IPC-08`: ack with our version; mismatch → `PROTOCOL_VERSION_MISMATCH`.
    if hello.protocol_version != crate::error::PROTOCOL_VERSION {
        let err = OxideError::new(
            ErrorCode::ProtocolVersionMismatch,
            format!(
                "client protocol {} != daemon protocol {}",
                hello.protocol_version,
                crate::error::PROTOCOL_VERSION
            ),
        );
        return write_failure(&mut stream, "", err);
    }
    let ack = Ack::new();
    if let Ok(body) = serde_json::to_vec(&ack) {
        let _ = write_frame(&mut stream, &body);
    }

    // `IPC-09` onward: exactly one request in flight per connection (`IPC-23`);
    // a second request is `INVALID_REQUEST`.
    let req_body = match read_frame(&mut stream) {
        Ok(Some(body)) => body,
        Ok(None) | Err(_) => return,
    };
    let req = match parse_request(&req_body) {
        Ok(r) => r,
        Err(e) => return write_failure(&mut stream, "", e),
    };

    let started = Instant::now();
    let result = ops::process_request(&req);
    let duration_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(processed) => {
            let success = Success {
                id: req.id.clone(),
                status: "ok",
                output_path: req.output.path.clone(),
                bytes: processed.bytes,
                width: processed.width,
                height: processed.height,
                duration_ms,
            };
            if let Ok(body) = serde_json::to_vec(&success) {
                let _ = write_frame(&mut stream, &body);
            }
        }
        Err(err) => write_failure(&mut stream, &req.id, err),
    }
    // `IPC-19`: exactly one reply per request — this function returns once.
}

fn write_failure(stream: &mut UnixStream, id: &str, err: OxideError) {
    let failure = Failure {
        id: id.to_string(),
        status: "error",
        error: err,
    };
    if let Ok(body) = serde_json::to_vec(&failure) {
        let _ = write_frame(stream, &body);
    }
}

// ---- listener setup (`LIFE-02`) ----

/// `LIFE-02`: the socket is created mode 0600 owned by the daemon's UID. A
/// stale socket from a crash is removed before bind.
fn bind_with_permissions() -> io::Result<(UnixListener, PathBuf)> {
    let path = resolve_socket_path();
    if let Some(parent) = path.parent() {
        if !parent.exists() && parent != Path::new("/tmp") {
            // XDG_RUNTIME_DIR may not exist; fail loudly (`LIFE-05`) rather than
            // silently binding nowhere.
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("socket parent directory does not exist: {}", parent.display()),
            ));
        }
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, path))
}

/// The daemon's run loop — blocks until the idle timer fires or `shutdown` is
/// set, then removes the socket and returns (`LIFE-13`).
pub fn run_forever(
    queue_capacity: usize,
    idle_ttl: Duration,
    shutdown: &AtomicBool,
) -> io::Result<()> {
    let (listener, socket_path) = bind_with_permissions()?;
    let (tx, rx) = mpsc::channel::<UnixStream>();
    let _accept_thread = thread::spawn(move || {
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    if tx.send(stream).is_err() {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
    });
    run_worker_pool(rx, queue_capacity, idle_ttl, shutdown)?;
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn life_01_xdg_runtime_dir_wins() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1234");
        assert_eq!(
            resolve_socket_path(),
            PathBuf::from("/run/user/1234/image-oxide.sock")
        );
        std::env::remove_var("XDG_RUNTIME_DIR");
    }

    #[test]
    fn life_01_fallback_tmp_per_uid() {
        std::env::remove_var("XDG_RUNTIME_DIR");
        let p = resolve_socket_path();
        let expect = PathBuf::from(format!("/tmp/image-oxide-{}.sock", current_uid()));
        assert_eq!(p, expect);
    }

    #[test]
    fn life_02_socket_is_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        drop(listener);
    }
}
