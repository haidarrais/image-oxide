# 001 — Daemon IPC

> **Status:** DRAFT · **Owner:** Rust daemon + PHP client
> **Single source of truth.** If the Rust daemon and the PHP client disagree on the wire format, this file wins.
> Migrates nowhere — lives in repo 1 (`haidarrais/image-oxide`) for the lifetime of the project.

- **Context & Decision** · **Definitions** · **Requirements** · **Acceptance Criteria** · **Out of Scope** · **Open Questions**

## Context & Decision

Two codebases in two languages implement against one contract: a Rust daemon that owns the image pipeline, and a PHP client that speaks to it over a local socket. The wire protocol is the seam between them, so it gets pinned first and frozen at 1.0.0.

Decisions (from PLAN.md, resolved conservative):

| # | Question | Decision |
|---|----------|----------|
| D-1 | Wire format | JSON, 4-byte big-endian length-prefixed frames. MessagePack deferred (OOS) — revisit only if profiling shows JSON parse is a real cost. |
| D-2 | Socket tenancy | Per-UID socket path (see `IPC-` requirements below). Kills shared-host cross-user access and stale-socket races. |
| D-3 | Transport | Unix domain socket on POSIX; loopback TCP on Windows. No named pipes (OOS). |
| D-4 | Byte transfer | Filesystem paths only — the daemon runs on the same machine, shared FS is the default. Inline bytes OOS v1. |
| D-5 | Versioning | `hello`/`ack` handshake with `protocol_version`. Additive-only changes after 1.0.0. Unknown fields ignored both directions. |

## Definitions

- **Frame** — a 4-byte big-endian uint32 length `N`, followed by exactly `N` bytes of UTF-8 JSON. One frame = one message.
- **Message** — the JSON payload of a frame.
- **Connection** — one client socket. One request in flight per connection; no multiplexing.
- **Op** — a single image transformation in a request's `ops[]` chain. Op semantics live in [003-image-ops.md](003-image-ops.md).
- **Daemon** — the long-lived Rust process owning the socket and the worker pool. Lifecycle in [002-daemon-lifecycle.md](002-daemon-lifecycle.md).

## Requirements

### Framing

- `IPC-01` — MUST. Messages are framed as 4-byte big-endian uint32 length + UTF-8 JSON payload. MUST NOT use newline framing.
- `IPC-02` — MUST. The daemon MUST reject any frame whose declared length exceeds `MAX_FRAME` (64 MiB) with error `FRAME_TOO_LARGE`, then close the connection.
- `IPC-03` — MUST. The daemon MUST reject frames whose byte length does not match the declared length, closing the connection.
- `IPC-04` — MUST. A frame's JSON MUST be a single object; malformed JSON is rejected with error `INVALID_REQUEST`, connection stays open.
- `IPC-05` — SHOULD. The client MUST NOT send frames larger than 64 MiB; `MAX_FRAME` is both sides' limit.

### Versioning & handshake

- `IPC-06` — MUST. On connect, the client MUST send a `hello` message containing `protocol_version` and `client_name`.
- `IPC-07` — MUST. The daemon MUST reply with `ack` containing `protocol_version` (its own) and `server_version`.
- `IPC-08` — MUST. On version mismatch the daemon MUST reply `PROTOCOL_VERSION_MISMATCH` and close. The daemon MUST survive the disconnect.
- `IPC-09` — MUST. After `ack`, the client MAY send requests. Requests sent before `ack` are treated as `INVALID_REQUEST`.
- `IPC-10` — SHOULD. Both sides MUST ignore unknown fields in any message (forward-compatible additive evolution).
- `IPC-11` — MUST. After 1.0.0, the protocol is additive-only: no field renamed, removed, or reinterpreted without a version bump.

### Request

- `IPC-12` — MUST. Request shape: `{ "id", "ops": [Op...], "input": {"path"}, "output": {"path"} }`. `ops` is a non-empty array of exactly the shapes defined in [003-image-ops.md](003-image-ops.md).
- `IPC-13` — MUST. `id` is an opaque string echoed back verbatim in the response. Used for correlation only.
- `IPC-14` — MUST. `input.path` and `output.path` are absolute filesystem paths on the host running the daemon. MUST NOT be relative.
- `IPC-15` — MUST. Path traversal outside the daemon's accessible roots is rejected with `ACCESS_DENIED`.
- `IPC-16` — MUST. Ops execute in array order; each op's output feeds the next op's input. The final op's output is written to `output.path`.

### Response

- `IPC-17` — MUST. Success shape: `{ "id", "status": "ok", "output_path", "bytes", "width", "height", "duration_ms" }`.
- `IPC-18` — MUST. Failure shape: `{ "id", "status": "error", "error": {"code", "message", "op_index"} }`. `op_index` is the index of the failing op in `ops[]`, or `null` when the failure is not op-specific.
- `IPC-19` — MUST. The daemon MUST reply to every request exactly once. No retries at the wire level.

### Error registry

- `IPC-20` — MUST. Error codes are exactly:

| Code | Meaning | Retryable |
|------|---------|-----------|
| `INVALID_REQUEST` | Malformed frame/JSON or pre-ack request | no |
| `FRAME_TOO_LARGE` | Frame exceeds 64 MiB | no |
| `PROTOCOL_VERSION_MISMATCH` | Hello version ≠ daemon version | no |
| `ACCESS_DENIED` | Path outside allowed roots / permission denied | no |
| `INPUT_NOT_FOUND` | `input.path` does not exist | no |
| `INPUT_UNREADABLE` | `input.path` exists but cannot be opened | yes |
| `DECODE_FAILED` | Input format not decodable by the daemon | no |
| `UNSUPPORTED_OPERATION` | Op or format not supported by this build | no |
| `OP_FAILED` | Op ran but produced an error (see `op_index`) | no |
| `OUTPUT_WRITE_FAILED` | Could not write `output.path` | yes |
| `DAEMON_OVERLOADED` | Worker pool and queue are full | yes |
| `INTERNAL` | Anything else | no |

- `IPC-21` — MUST. A client MUST treat an unknown error code as non-retryable `INTERNAL`.
- `IPC-22` — SHOULD. A client MUST back off (exponential, capped) before retrying a retryable error.

### Concurrency

- `IPC-23` — MUST. One request in flight per connection. A second request on the same connection before the first completes is rejected with `INVALID_REQUEST`.
- `IPC-24` — MUST. Client disconnect mid-job MUST abort the job and free its worker slot.

## Acceptance Criteria

- `AC-IPC-01` — A Rust implementation and a PHP implementation, built from this file alone, interoperate: hello/ack, one resize request, one error request (missing input) round-trip correctly.
- `AC-IPC-02` — A frame with declared length > 64 MiB is closed with `FRAME_TOO_LARGE`; the daemon accepts the next connection.
- `AC-IPC-03` — A frame whose length field and body disagree is closed without crashing the daemon.
- `AC-IPC-04` — A request with a corrupt JSON body gets `INVALID_REQUEST`; connection stays usable.
- `AC-IPC-05` — A request with `protocol_version` ≠ daemon's gets `PROTOCOL_VERSION_MISMATCH` and the daemon survives.
- `AC-IPC-06` — Unknown fields in a request are ignored; the request succeeds.

## Out of Scope

- `OOS-IPC-01` — MessagePack or any binary payload encoding. Revisit if JSON parse shows up in profiling.
- `OOS-IPC-02` — Named pipes / any transport beyond Unix socket (POSIX) and loopback TCP (Windows).
- `OOS-IPC-03` — Inline byte transfer in frames (base64 inflation). Remote-disk workflows go through temp files (see [005-laravel-bridge.md](../laravel-image-oxide/specs/005-laravel-bridge.md)).
- `OOS-IPC-04` — Connection multiplexing / request pipelining.
- `OOS-IPC-05` — Compression of frames.

## Open Questions

None. All decisions resolved above; unresolved items are in OOS.
