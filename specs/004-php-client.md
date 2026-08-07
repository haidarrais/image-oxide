# 004 — PHP Client

> **Status:** DRAFT · **Owner:** `laravel-image-oxide` (repo 2)
> **Consumes only 001/003.** Never talks to raw sockets directly from framework code — that's this package's job. The GD capability table here mirrors 003's `OPS-02` matrix exactly.
> Lives in repo 1 (`haidarrais/image-oxide`) and ships from [`haidarrais/php-image-oxide`](https://github.com/haidarrais/php-image-oxide) (Packagist `haidarrais/image-oxide`). Its consumer, [005](../laravel-image-oxide/specs/005-laravel-bridge.md), has migrated to repo 2 (`haidarrais/laravel-image-oxide`); the link resolves on a local sibling checkout.

- **Context & Decision** · **Definitions** · **Requirements** · **Acceptance Criteria** · **Out of Scope** · **Open Questions**

## Context & Decision

A fluent PHP API over the 001 wire protocol with a GD fallback driver that implements 003's op contract. Ships as `haidarrais/image-oxide` on Packagist, consumed by the Laravel bridge (005).

## Definitions

- **Driver** — the back-end that executes ops. `DaemonDriver` (Rust) or `GdDriver` (fallback).
- **Capability table** — a per-driver matrix of supported ops/formats, queried at runtime.

## Requirements

### Public API

- `PHP-01` — MUST. Fluent builder over ops: `Oxide::from($path)->resize(800, 600)->format('webp')->to($outPath)`. Each op mirrors the matching requirement in [003](003-image-ops.md).
- `PHP-02` — MUST. `to($path)` returns the output path on success and throws on failure.
- `PHP-03` — MUST. `get()` returns the bytes; `getUri()`/`getUrl()` supported only via the Laravel bridge (005).
- `PHP-04` — SHOULD. Ops are chainable and executed only on terminal call (`to`/`get`), so the same chain can be reused across multiple images.

### Driver selection & fallback

- `PHP-05` — MUST. The client uses `DaemonDriver` when the daemon is reachable, else `GdDriver`, per a configurable precedence. The fallback is automatic and logged.
- `PHP-06` — MUST. Capability query: `Oxide::capabilities()` returns which ops and formats each driver supports, per the table below.
- `PHP-07` — MUST. An op the active driver does not support (e.g. AVIF encode on GD) throws `UnsupportedOperationException` — never a silent no-op.

### GD capability table (mirrors 003 `OPS-02`)

- `PHP-08` — MUST. `GdDriver` supports: resize, format (jpeg/png/webp/gif), rotate, watermark. AVIF encode → `UnsupportedOperationException` (`OPS-12`).
- `PHP-09` — MUST. `GdDriver` applies EXIF auto-orientation (`OPS-03`).

### Exception taxonomy

- `PHP-10` — MUST. Exceptions map to the [001](001-daemon-ipc.md) error registry 1:1. Base `OxideException`; concrete types per error code, e.g. `OverloadedException` (retryable), `InputNotFoundException`, `UnsupportedOperationException`, `ProtocolException`.
- `PHP-11` — MUST. Retryable exceptions carry a `retryAfter`-style backoff hint; the client applies capped exponential backoff for `DAEMON_OVERLOADED` and `INPUT_UNREADABLE` (`IPC-22`).

### Transport & framing

- `PHP-12` — MUST. The client implements 001 framing (4-byte BE length + UTF-8 JSON) and the hello/ack handshake (`IPC-06`–`IPC-11`).
- `PHP-13` — MUST. One request in flight per connection; a fresh connection is (re)established per terminal call (`IPC-23`).
- `PHP-14` — MUST. Paths passed to the daemon are absolute (`IPC-14`).

## Acceptance Criteria

- `AC-PHP-01` — `Oxide::from($jpg)->resize(800, 600, 'cover')->format('webp')->to($out)` produces a 800×600 WebP with a reachable daemon, and an identical image through `GdDriver` when the daemon is down.
- `AC-PHP-02` — `Oxide::from($avif)->format('avif')` on GD throws `UnsupportedOperationException`; on a daemon that supports AVIF encode it succeeds (or throws if v1 — `OPS-12`).
- `AC-PHP-03` — With the daemon overloaded, the client surfaces `OverloadedException` after backoff, and a later call succeeds.
- `AC-PHP-04` — `capabilities()` output for `GdDriver` matches the 003 matrix cell-for-cell.
- `AC-PHP-05` — Unknown error code from the daemon maps to non-retryable `INTERNAL` (`IPC-21`).

## Out of Scope

- `OOS-PHP-01` — ImageMagick driver.
- `OOS-PHP-02` — Async/streaming byte transfer (filesystem-path contract, `OOS-IPC-03`).
- `OOS-PHP-03` — Anything beyond the 001 error registry's surface (unknown codes → `INTERNAL`).

## Open Questions

None.
