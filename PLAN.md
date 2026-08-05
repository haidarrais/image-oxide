# Plan: Transform PLAN.md into Spec-Driven Development

## Context

The directory contains only `PLAN.md` — a 5-phase linear execution plan for `haidarrais/image-oxide` (Rust image daemon + PHP client + Laravel bridge). No code exists. Goal: rebuild the planning stage around Spec-Driven Development — requirements specified first (numbered, testable, RFC-2119), implementation derives from specs, CI tests trace to requirement IDs.

The plan-as-written has one genuinely dangerous gap: the IPC protocol is a single line ("JSON-over-Socket or MessagePack") yet it's the contract two codebases in two languages implement against. SDD's real value here is pinning that contract and forcing ~10 product decisions before code — not process ceremony. Effort allocation follows that: the IPC and ops-semantics specs get written fully; everything else stays thin.

## Decisions (open questions resolved — the conservative call on each)

| # | Question | Decision |
|---|----------|----------|
| 1 | GD fallback depth | Per-op capability table (resize/format/rotate/watermark supported; AVIF throws `UnsupportedOperationException`). "Graceful degradation" becomes a defined contract, not a shrug. |
| 2 | Wire format | JSON, length-prefixed framing. MessagePack → OOS, revisit if profiling shows parse cost. |
| 3 | Socket tenancy | **Per-UID**: `$XDG_RUNTIME_DIR/image-oxide.sock` → fallback `/tmp/image-oxide-$UID.sock`, mode 0600. Windows: 127.0.0.1 TCP. Kills shared-host cross-user access + stale-socket fights at zero extra cost. |
| 4 | AVIF | Decode: SHOULD (may slip to v1.1 if it drags C deps into the cross-compile matrix). Encode: OOS for v1 (rav1e is minutes-per-image). |
| 5 | Watermark | 9-grid positions + px offsets. Opacity 0.0–1.0 multiplies watermark alpha. Tiling OOS. |
| 6 | Concurrency | One shared daemon; worker pool `min(4, num_cpus)`; bounded queue; `DAEMON_OVERLOADED` + client backoff when full. Pure-serial rejected (FPM bursts torpedo p95). |
| 7 | Windows | TCP fallback only; WSL documented as the recommended path. Named pipes OOS. |
| 8 | Byte transfer | Filesystem paths only (daemon is spawned on the same machine; shared-FS holds). Remote-disk workflows (S3) go through temp files — pattern documented in 005. Inline bytes → OOS v1 (33% base64 inflation in JSON frames). |
| 9 | EXIF orientation | Auto-apply on decode (matches user expectation for phone photos; Intervention-like). Documented loudly in 003 since it silently swaps pixel dimensions. |
| 10 | NFR numbers | Starting stakes, calibrated in Phase 1 and CI-enforced from Phase 4: boot-to-accepting p50 <50ms (PLAN's "<5ms" is cold-cache fantasy); resize 4000×3000→800×600 JPEG p95 <300ms on CI runner; daemon RSS ceiling 128MB on a 24MP image; max frame 64 MiB. |

## What gets built

7 spec files + PLAN.md rewritten as a pure index:

```
PLAN.md                        # REWRITTEN: index — phase table, spec status, principles pointer, launch checklist
specs/
├── 000-constitution.md        # 10 lines: perf claims need CI benchmarks; zero-config install never regresses;
│                              #   degradation > hard failure; boring > clever; protocol semver-frozen at 1.0.0
├── 001-daemon-ipc.md          # ★ THE contract. Full treatment (see below)
├── 002-daemon-lifecycle.md    # spawn race, socket resolution order, idle shutdown, zombies, worker pool
├── 003-image-ops.md           # ★ pixel semantics; ALSO the GD fallback's contract (dual-implementation)
├── 004-php-client.md          # fluent API + exception taxonomy + binary downloader + GD capability table
├── 005-laravel-bridge.md      # provider, config keys, Intervention shim mapping table, disk macros
└── 006-ci-release.md          # matrices (mostly moved from PLAN.md Phase 4) + manual coverage table + publish order
```

**ID scheme:** `<AREA>-<NN>` per file (`IPC-`, `LIFE-`, `OPS-`, `PHP-`+`DL-`, `LV-`, `CI-`, `NFR-`), never reused/renumbered; superseded = struck through. Acceptance criteria `AC-<AREA>-<NN>`, out-of-scope `OOS-<AREA>-<NN>` (numbered so future PRs cite instead of re-litigate). Tests reference IDs by naming convention (`fn ipc_04_rejects_oversized_frame()`).

**Uniform file skeleton:** Context & Decision → Definitions → Requirements (RFC-2119) → Acceptance Criteria → Out of Scope → Open Questions (must be empty before that phase's implementation starts).

**Contract rules (written into every spec header):**
1. 001 is the single source of truth for the wire protocol; if Rust and PHP disagree, 001 wins.
2. 005 consumes only 004's public API, never raw sockets.
3. 003 is implemented twice (Rust daemon, GD driver) — that duality is what makes the fallback a contract.

## 001 skeleton (the file that matters most — write first, with 003)

- **Framing:** 4-byte BE u32 length + UTF-8 JSON payload (newline framing rejected — binary payloads). MAX_FRAME 64 MiB → `FRAME_TOO_LARGE` + close.
- **Versioning:** hello/ack handshake with `protocol_version`; mismatch → `PROTOCOL_VERSION_MISMATCH`, close, daemon survives. Unknown fields ignored both ways (forward-compat); additive-only after 1.0.0.
- **Request:** `{id, ops[], input{path}, output{path}}` with full JSON examples per op (resize/format/rotate/watermark).
- **Response:** success `{id, status:ok, output_path, bytes, width, height, duration_ms}`; failure `{id, status:error, error{code, message, op_index}}` — `op_index` pinpoints the failing op in a chain.
- **Error registry:** table of 12 codes with retryable? column; unknown codes treated as non-retryable `INTERNAL`.
- **Concurrency:** one request in flight per connection, no multiplexing; client disconnect mid-job → abort + free.

## Ponytail cuts (deliberate, reversible)

- **No automated coverage gate** ("CI fails if a requirement ID has zero tests") — grep-on-test-names is fragile; keep the naming convention + a manual requirement→CI-leg table in 006 instead. Add the gate only if IDs start rotting.
- **No TASKS.md / task-breakdown templates** — PLAN.md's index gets a status column: "phase done when its MUSTs pass ACs." That *is* the task list.
- **No spec for Phase 5** (docs/launch) — it stays a checklist in PLAN.md.
- **No clarify/checklist templates** from spec-kit.

## Repo note

Specs 005 and the Laravel half of 006 belong to `haidarrais/laravel-image-oxide` (repo 2, doesn't exist yet). They live in this repo now and **migrate when repo 2 is created** — noted in both files' headers so repo 1 doesn't ship dead specs.

## Execution order (one sitting, no code)

1. Write `001-daemon-ipc.md` + `003-image-ops.md` fully (they pin everything downstream).
2. Write `002`, `004`, `005` at requirements+AC depth (no gold-plating).
3. Write `000` (10 lines) and `006` (move Phase 4 matrices, add coverage table).
4. Rewrite `PLAN.md` as the index: phase → specs → status table, principles pointer, launch checklist. Old PLAN.md content survives only where moved into specs.

## Verification

- Every requirement has unique ID + MUST/SHOULD/MAY + at least one AC; every OOS item numbered.
- 001 is complete enough that a Rust dev and a PHP dev each implement without talking (framing, handshake, schemas, error registry all present).
- 003's format matrix (decode/encode × JPEG/PNG/GIF/WebP/AVIF) matches 004's GD capability table exactly where they overlap.
- PLAN.md index links every phase to its specs; no content lives in both PLAN.md and a spec (single home per fact).
- Open Questions section empty in 001/002/003 before Phase 1 code; empty in 004 before Phase 2; 005/006 before their phases.
