# 006 — CI & Release

> **Status:** DRAFT · **Owner:** CI/CD for both repos
> **Split across repos.** Rust daemon + PHP client matrices and the publish order live here (repo 1). The Laravel-bridge half lives in repo 2's [specs/006-ci-release.md](../laravel-image-oxide/specs/006-ci-release.md). One spec, two homes — headers on both sides say so.

- **Context & Decision** · **Requirements** · **Acceptance Criteria** · **Out of Scope** · **Open Questions**

## Context & Decision

CI enforces the constitution (000) from Phase 4 onward. Matrices and publish order formalize what PLAN.md Phase 4 sketched; the manual coverage table replaces an automated requirement-ID coverage gate (grep-on-test-names is fragile — see Ponytail cuts).

## Requirements

### Matrices

- `CI-01` — MUST. Rust daemon CI matrix: `ubuntu-latest`, `macos-latest`, `windows-latest` × stable toolchain. Windows runs the TCP transport tests (`LIFE-03`).
- `CI-02` — MUST. PHP client CI matrix: PHP 8.1, 8.2, 8.3 × `ubuntu-latest`. Runs against a compiled daemon binary from the same pipeline (or a tagged release).
- `CI-03` — MUST. Cross-compile target for the daemon covers `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc` (avoids C deps for AVIF in v1 — `OPS-01`).

### NFR benchmarks (from PLAN.md, calibrated in Phase 1)

- `CI-04` — MUST. Boot-to-accepting p50 < 50ms, measured on the CI runner.
- `CI-05` — MUST. Resize 4000×3000 → 800×600 JPEG p95 < 300ms on the CI runner.
- `CI-06` — MUST. Daemon RSS ceiling 128 MB on a 24MP image (measured, asserted in CI).
- `CI-07` — MUST. Max frame 64 MiB (`IPC-02`).

### Test naming & coverage table

- `CI-08` — MUST. Tests reference requirement IDs by naming convention: `fn ipc_04_rejects_oversized_frame()`, `fn ops_13_rotate_multiple_of_90()`.
- `CI-09` — MUST. The table below is the manual requirement→test leg. A phase's MUSTs pass only when every row in the table is filled for that phase's IDs.

| Spec | Phase | ID prefix | CI leg | Benchmarks |
|------|-------|-----------|--------|-----------|
| 001 IPC | 1 | `IPC-` | daemon + PHP client integration tests | — |
| 002 Lifecycle | 1 | `LIFE-` | daemon lifecycle tests | boot p50 |
| 003 Ops | 1 | `OPS-` | daemon op tests + GD driver op tests | resize p95, RSS |
| 004 PHP client | 2 | `PHP-` | PHP client test suite (in [`haidarrais/php-image-oxide`](https://github.com/haidarrais/php-image-oxide)) | — |
| 005 Laravel bridge | 3 | `LV-` | repo 2 PHPUnit suite | — |
| NFRs | 4 | `NFR-`/`CI-` | CI benchmark job | all of the above |

### Publish order

- `CI-10` — MUST. Publish order is: daemon binary release (tagged) → `haidarrais/image-oxide` Packagist package → `haidarrais/laravel-image-oxide` Packagist package. The Laravel bridge (005) depends on 004's published package; 004's integration tests depend on the daemon release.
  - **Status (v0.1.0):** `haidarrais/image-oxide` (from [`php-image-oxide`](https://github.com/haidarrais/php-image-oxide)) ✅ published · `haidarrais/laravel-image-oxide` ✅ published · daemon binary release ⏳ not yet tagged (only local `target/debug` builds).

## Acceptance Criteria

- `AC-CI-01` — A `MUST` without at least one `AC` and one test row in the `CI-09` table fails review.
- `AC-CI-02` — The three cross-compile targets build in CI without C-dependency workarounds for AVIF.
- `AC-CI-03` — NFR benchmarks fail the build when they regress past their numbers.

## Out of Scope

- `OOS-CI-01` — Automated requirement-ID coverage gate (grep-on-test-names fragility). Add only if IDs start rotting.
- `OOS-CI-02` — Windows production daemon support beyond TCP transport tests (WSL is the documented path, `LIFE-03`).

## Open Questions

None.
