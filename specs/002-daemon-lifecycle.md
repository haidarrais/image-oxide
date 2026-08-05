# 002 — Daemon Lifecycle

> **Status:** DRAFT · **Owner:** Rust daemon
> Lives in repo 1 (`haidarrais/image-oxide`).
> Requirements at requirements+AC depth — the hard edges (spawn race, socket resolution, shutdown, worker pool) are specified; the rest follows 001/003.

- **Context & Decision** · **Definitions** · **Requirements** · **Acceptance Criteria** · **Out of Scope** · **Open Questions**

## Context & Decision

The daemon is spawned lazily by the client on first use, so spawn races and socket staleness are the two failure modes that will actually page someone at 3am. This spec pins both.

Decisions:

| # | Question | Decision |
|---|----------|----------|
| D-1 | Socket tenancy | **Per-UID**: `$XDG_RUNTIME_DIR/image-oxide.sock`, falling back to `/tmp/image-oxide-$UID.sock`. Mode 0600. Kills shared-host cross-user access and stale-socket fights at zero extra cost. |
| D-2 | Windows | Loopback TCP fallback only; WSL documented as the recommended path. |
| D-3 | Concurrency | One shared daemon per UID; worker pool `min(4, num_cpus)`; bounded queue; `DAEMON_OVERLOADED` + client backoff when full. Pure-serial rejected (FPM bursts torpedo p95). |
| D-4 | Idle shutdown | Daemon exits after configurable idle timeout (default 60s). |

## Requirements

### Socket resolution & permissions

- `LIFE-01` — MUST. Resolve the socket path in order: `$XDG_RUNTIME_DIR/image-oxide.sock`, else `/tmp/image-oxide-$UID.sock`.
- `LIFE-02` — MUST. On POSIX, the socket file is created mode 0600, owned by the daemon's UID.
- `LIFE-03` — MUST. On Windows, the daemon listens on loopback TCP (127.0.0.1) on an ephemeral port; WSL is the documented recommended path.
- `LIFE-04` — MUST. The client MUST use the same resolution order to locate the daemon.

### Spawn & attach

- `LIFE-05` — MUST. If the socket does not exist, the client MUST spawn the daemon binary before connecting.
- `LIFE-06` — MUST. If the socket exists but connect fails (stale socket), the client MUST remove the stale socket and respawn, rather than erroring out.
- `LIFE-07` — MUST. A single UID spawns at most one daemon. Concurrent first-use spawns MUST NOT create two daemons (spawn must be atomic per UID — bind or fail).
- `LIFE-08` — MUST. On spawn failure, the client surfaces a clear error containing the daemon's stderr tail and the exit code.

### Worker pool & queue

- `LIFE-09` — MUST. Worker pool size is `min(4, num_cpus)`.
- `LIFE-10` — MUST. Requests beyond the pool sit in a bounded queue. A full queue is rejected with `DAEMON_OVERLOADED` (`IPC-20`) rather than unbounded buffering.
- `LIFE-11` — MUST. The queue capacity is configurable, default 32.

### Idle shutdown

- `LIFE-12` — MUST. The daemon exits after `LIFE-12_TTL` (default 60s) of no requests. In-flight requests are allowed to complete.
- `LIFE-13` — MUST. On shutdown the daemon removes its socket file. A stale socket left by a crash is handled by `LIFE-06`.
- `LIFE-14` — SHOULD. SIGTERM triggers graceful shutdown: stop accepting, drain in-flight, remove socket, exit 0.

### Zombies & cleanup

- `LIFE-15` — MUST. The client MUST NOT leave orphaned daemon processes on exit; a daemon with no live connections and no pending work exits on its own idle timer.
- `LIFE-16` — SHOULD. A daemon that crashes mid-job leaves no partial output file that could be mistaken for a valid result (write-to-temp-then-rename).

## Acceptance Criteria

- `AC-LIFE-01` — Two concurrent first-use clients in the same UID result in exactly one daemon; both attach successfully.
- `AC-LIFE-02` — Killing the daemon (SIGKILL), leaving a stale socket, then starting a client results in the client clearing the socket, respawning, and serving a request.
- `AC-LIFE-03` — A burst of N > (pool + queue) requests returns `DAEMON_OVERLOADED` for the excess, not a crash and not unbounded memory.
- `AC-LIFE-04` — After 60s idle, the daemon exits 0 and removes its socket.
- `AC-LIFE-05` — The socket file is mode 0600; a different UID cannot connect.

## Out of Scope

- `OOS-LIFE-01` — Named pipes / Windows-native transports (see [001](001-daemon-ipc.md) `OOS-IPC-02`).
- `OOS-LIFE-02` — Multi-daemon orchestration or daemon auto-restart supervisors.
- `OOS-LIFE-03` — Cross-user request routing.

## Open Questions

None.
