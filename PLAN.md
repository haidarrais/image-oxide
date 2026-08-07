# image-oxide — Plan Index

Rust image daemon + PHP client. Laravel bridge lives in the sibling repo [`haidarrais/laravel-image-oxide`](https://github.com/haidarrais/laravel-image-oxide).

Everything is spec-driven. The specs are the requirements; this file is only an index.

## Principles

[specs/000-constitution.md](specs/000-constitution.md) — 10 lines. Perf claims need CI benchmarks; zero-config install never regresses; degradation over hard failure; boring over clever; protocol semver-frozen at 1.0.0.

## Spec status

| Spec | Phase | Status |
|------|-------|--------|
| [000-constitution.md](specs/000-constitution.md) | all | FROZEN |
| [001-daemon-ipc.md](specs/001-daemon-ipc.md) — wire protocol | 1 | ✅ DONE — 22+9 tests green, commit `7900765` |
| [002-daemon-lifecycle.md](specs/002-daemon-lifecycle.md) | 1 | ✅ DONE — socket 0600, pool, idle shutdown, SIGTERM |
| [003-image-ops.md](specs/003-image-ops.md) — pixel semantics (dual impl) | 1 | ✅ DONE — pipeline + EXIF, AC-OPS-03 covered |
| [004-php-client.md](specs/004-php-client.md) | 2 | ✅ DONE — published as [`haidarrais/php-image-oxide`](https://github.com/haidarrais/php-image-oxide) / Packagist `haidarrais/image-oxide` |
| [006-ci-release.md](specs/006-ci-release.md) — Rust/client half | 4 | DRAFT |
| 005-laravel-bridge.md, 006 (Laravel half) | 3 | **migrated** → repo 2 |

## Phases

| Phase | Builds | Done when |
|-------|--------|-----------|
| 1 | Rust daemon: IPC (001), lifecycle (002), ops (003) | All `IPC-`/`LIFE-`/`OPS-` MUSTs pass their ACs |
| 2 | PHP client (004) | All `PHP-`/`DL-` MUSTs pass; GD capability table matches 003 |
| 3 | Laravel bridge (005) | All `LV-` MUSTs pass (in repo 2) |
| 4 | CI + NFR benchmarks (006) | `CI-04`–`CI-07` numbers hold; coverage table full |
| 5 | Docs & launch | Launch checklist below |

A phase is done when its MUSTs pass their ACs — the status column above is the task list.

## Contract rules

1. **001 wins** — single source of truth for the wire protocol.
2. **005 consumes only 004's public API**, never raw sockets. 004 now ships from the split repo [`haidarrais/php-image-oxide`](https://github.com/haidarrais/php-image-oxide).
3. **003 is implemented twice** (Rust daemon, GD driver) — that duality is the graceful-degradation contract.

## NFR stakes

| Number | Target | Where |
|--------|--------|-------|
| Boot-to-accepting p50 | < 50 ms | 002, CI-04 |
| Resize 4000×3000→800×600 JPEG p95 | < 300 ms | 003, CI-05 |
| Daemon RSS on 24MP image | ≤ 128 MB | 002, CI-06 |
| Max frame | 64 MiB | 001 IPC-02 |
| Worker pool | `min(4, num_cpus)`, queue 32 | 002 LIFE-09/10 |

## Launch checklist

- [ ] Phase 1 daemon ships: all `IPC-`/`LIFE-`/`OPS-` ACs green in CI
- [x] Phase 2 client ships: `PHP-` ACs green, GD table matches 003 — published as `haidarrais/image-oxide` on Packagist
- [x] Phase 3 bridge ships (repo 2): `LV-` ACs green — published as `haidarrais/laravel-image-oxide` on Packagist
- [ ] Phase 4: NFR benchmarks pass `CI-04`–`CI-07`; coverage table full
- [x] Publish order: daemon binary release → `haidarrais/image-oxide` → `haidarrais/laravel-image-oxide`
