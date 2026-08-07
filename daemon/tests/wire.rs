//! End-to-end wire tests against the real daemon lifecycle: spawn the listener
//! on an isolated runtime dir, drive it over a Unix socket, assert the exact
//! JSON shapes of 001 and the error mapping of IPC-20.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use image::ImageEncoder;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_image-oxide");

struct Daemon {
    child: Child,
    socket: PathBuf,
    _runtime: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        let runtime = tempfile::tempdir().unwrap();
        let socket = runtime.path().join("image-oxide.sock");
        // Deterministic per-test socket: point XDG_RUNTIME_DIR at a fresh temp
        // dir; the daemon binds `<xdg>/image-oxide.sock` there (`LIFE-01`).
        let xdg = runtime.path();
        let mut child = Command::new(BIN)
            .env("XDG_RUNTIME_DIR", xdg)
            .env("IMAGE_OXIDE_TTL_MS", "3000")
            .env("IMAGE_OXIDE_QUEUE", "4")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        // Wait for the listener socket to appear (bounded — fail loudly, not hang).
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if socket.exists() {
                return Self {
                    child,
                    socket,
                    _runtime: runtime,
                };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon did not create socket within 5s");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn connect(d: &Daemon) -> UnixStream {
    UnixStream::connect(&d.socket).unwrap()
}

/// Handshake (`IPC-06`–`IPC-08`): hello → ack.
fn hello(stream: &mut UnixStream) -> Value {
    let hello = serde_json::json!({
        "type": "hello",
        "protocol_version": "1.0.0",
        "client_name": "tests"
    });
    write_frame(stream, &serde_json::to_vec(&hello).unwrap());
    read_msg(stream)
}

fn write_frame(stream: &mut UnixStream, body: &[u8]) {
    let len = u32::try_from(body.len()).unwrap();
    stream.write_all(&len.to_be_bytes()).unwrap();
    stream.write_all(body).unwrap();
}

fn read_msg(stream: &mut UnixStream) -> Value {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).unwrap();
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn request(id: &str, input: &str, output: &str) -> Value {
    serde_json::json!({
        "id": id,
        "ops": [{"type": "format", "format": "png"}],
        "input": {"path": input},
        "output": {"path": output}
    })
}

fn write_png(path: &PathBuf, w: u32, h: u32) {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x * 3 % 256) as u8, (y * 5 % 256) as u8, 200, 255])
    });
    let (w, h) = img.dimensions();
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(&img.into_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
    std::fs::write(path, buf).unwrap();
}

// ---- IPC: handshake & framing ----

#[test]
fn ipc_06_08_hello_ack_roundtrip() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    let ack = hello(&mut stream);
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["protocol_version"], "1.0.0");
    assert!(ack["server_version"].is_string());
}

#[test]
fn ipc_09_pre_ack_request_rejected() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    write_frame(&mut stream, &serde_json::to_vec(&request("x", "/a", "/b")).unwrap());
    let resp = read_msg(&mut stream);
    assert_eq!(resp["status"], "error");
    assert_eq!(resp["error"]["code"], "INVALID_REQUEST");
}

#[test]
fn ipc_02_oversized_frame_connection_closed() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    // Declared length > 64 MiB without sending the body.
    let mut hdr = Vec::new();
    hdr.extend_from_slice(&(64u32 * 1024 * 1024 + 1).to_be_bytes());
    stream.write_all(&hdr).unwrap();
    let mut probe = [0u8; 1];
    // Server must close the connection without a response frame.
    stream.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match stream.read(&mut probe) {
            Ok(0) => break, // EOF: closed
            Ok(_) => panic!("server sent bytes for an oversized frame"),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    panic!("server did not close the connection");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

#[test]
fn ipc_12_request_response_id_matches() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    let _ = hello(&mut stream);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    write_png(&input, 40, 30);
    let output = dir.path().join("out.png");
    let body = request("req-abc", input.to_str().unwrap(), output.to_str().unwrap());
    write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
    let resp = read_msg(&mut stream);
    assert_eq!(resp["id"], "req-abc");
    assert_eq!(resp["status"], "ok");
    assert_eq!(resp["width"], 40);
    assert_eq!(resp["height"], 30);
    assert!(output.exists());
}

#[test]
fn ipc_20_missing_input_maps_to_input_not_found() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    let _ = hello(&mut stream);
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.png");
    let output = dir.path().join("out.png");
    let body = request("r2", missing.to_str().unwrap(), output.to_str().unwrap());
    write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
    let resp = read_msg(&mut stream);
    assert_eq!(resp["status"], "error");
    assert_eq!(resp["error"]["code"], "INPUT_NOT_FOUND");
}

#[test]
fn ipc_14_relative_input_denied() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    let _ = hello(&mut stream);
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.png");
    let body = request("r3", "relative.png", output.to_str().unwrap());
    write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
    let resp = read_msg(&mut stream);
    assert_eq!(resp["status"], "error");
    assert_eq!(resp["error"]["code"], "ACCESS_DENIED");
}

#[test]
fn ipc_17_19_exactly_one_reply_per_request() {
    let d = Daemon::start();
    let mut stream = connect(&d);
    let _ = hello(&mut stream);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.png");
    write_png(&input, 10, 10);
    let output = dir.path().join("out.png");
    let body = request("r4", input.to_str().unwrap(), output.to_str().unwrap());
    write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
    let resp = read_msg(&mut stream);
    assert_eq!(resp["status"], "ok");
    // Second read must not yield another message. Nonblocking read proves
    // silence (WouldBlock) or close (Ok(0)) instead of a second frame.
    stream.set_nonblocking(true).unwrap();
    let mut probe = [0u8; 4];
    match stream.read(&mut probe) {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok(0) => {}
        other => panic!("expected silence after single reply, got {other:?}"),
    }
}

#[test]
fn ipc_23_sequential_requests_on_one_connection() {
    // IPC-23 allows one request *in flight* at a time; a keep-alive client can
    // reuse the connection for the next request after the first completes.
    let d = Daemon::start();
    let mut stream = connect(&d);
    let _ = hello(&mut stream);
    let dir = tempfile::tempdir().unwrap();
    for i in 0..3 {
        let input = dir.path().join(format!("in{i}.png"));
        write_png(&input, 8 + i, 8 + i);
        let output = dir.path().join(format!("out{i}.png"));
        let body = request(&format!("k{i}"), input.to_str().unwrap(), output.to_str().unwrap());
        write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
        let resp = read_msg(&mut stream);
        assert_eq!(resp["status"], "ok");
        assert_eq!(resp["id"], format!("k{i}"));
    }
}

// ---- LIFE: lifecycle behavior ----

#[test]
fn life_06_11_daemon_survives_sequential_connections() {
    let d = Daemon::start();
    for i in 0..3 {
        let mut stream = connect(&d);
        let _ = hello(&mut stream);
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.png");
        write_png(&input, 8 + i, 8 + i);
        let output = dir.path().join("out.png");
        let body = request(&format!("s{i}"), input.to_str().unwrap(), output.to_str().unwrap());
        write_frame(&mut stream, &serde_json::to_vec(&body).unwrap());
        let resp = read_msg(&mut stream);
        assert_eq!(resp["status"], "ok");
    }
}

#[test]
fn life_13_socket_removed_after_idle_shutdown() {
    let runtime = tempfile::tempdir().unwrap();
    let target = runtime.path().join("image-oxide.sock");
    let mut child = Command::new(BIN)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("IMAGE_OXIDE_TTL_MS", "500")
        .env("IMAGE_OXIDE_QUEUE", "4")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if target.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(target.exists(), "socket should appear");
    // After idle TTL the daemon must exit and remove the socket.
    let waited = Instant::now();
    loop {
        // Poll the child's exit status manually — the socket removal is the
        // observable signal, not a std::process status handle.
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if waited.elapsed() > Duration::from_secs(5) {
                    panic!("daemon did not exit after idle timeout");
                }
            }
            Err(_) => panic!("try_wait failed"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!target.exists(), "socket must be removed on shutdown (`LIFE-13`)");
}
