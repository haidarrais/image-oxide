# 000 — Constitution

> **Status:** FROZEN · **Owner:** everyone
> Ten lines. Everything below derives from here.

1. **Perf claims need CI benchmarks.** No number is real until a benchmark in this repo measures it (see [006](006-ci-release.md)).
2. **Zero-config install never regresses.** `composer require` + provider auto-discovery must always just work.
3. **Degradation over hard failure.** When the daemon is down, the GD driver serves. When an op is unsupported, it throws loudly — never silently no-ops.
4. **Boring over clever.** The first lazy solution that works wins; anything clever gets a comment naming its ceiling.
5. **The protocol is semver-frozen at 1.0.0.** [001](001-daemon-ipc.md) is the single source of truth; additive-only after 1.0.0 (`IPC-11`).
6. **003 is implemented twice** (Rust + GD) — that duality is the graceful-degradation contract, not an implementation detail.
7. **One home per fact.** No requirement lives in both a spec and PLAN.md; PLAN.md is an index.
8. **A spec is done when its MUSTs pass their ACs.** Status column in PLAN.md is the task list.
9. **Migrations are noted in-file.** A spec that moves repos (like 005) says so in its header.
10. **If two implementations disagree, the spec wins.**
