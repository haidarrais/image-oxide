# Image Oxide

A per-UID local image-processing **daemon** in Rust, plus a zero-setup
[PHP client](https://github.com/haidarrais/php-image-oxide) and
[Laravel bridge](https://github.com/haidarrais/laravel-image-oxide).

Processes JPEG/PNG/WebP/GIF over a Unix socket: resize, format, rotate,
watermark. Built for the upload use case — a full-size user photo → resized +
compressed lossy JPEG — where it beats in-process GD on speed and output size
while freeing the PHP-FPM worker.

## Principles

1. **Perf claims need CI benchmarks** — no number is real until `bench.sh` measures it.
2. **Zero-config install never regresses** — `composer require` + first call just works.
3. **Degradation over hard failure** — when the daemon is down, the GD driver serves.
4. **Boring over clever.**
5. **Protocol semver-frozen at 1.0.0** — additive-only after 1.0.0.

## Layouts

| Path | What |
|------|------|
| `daemon/` | Rust daemon (`image-oxide` binary) — socket lifecycle, worker pool, pixel ops |
| `php-image-oxide/` (repo) | PHP client — fluent API, `DaemonManager` zero-setup auto-spawn, GD fallback |
| `laravel-image-oxide/` (repo) | Laravel bridge — provider, config, `Oxide` facade, Intervention shim, `Storage::oxideResize` macros |
| `laravel-image-oxide-demo/` | Demo + committed NFR benchmark (`bench.sh` / `BENCHMARK.md`) |

## Daemon

```bash
cargo build --release --manifest-path daemon/Cargo.toml
daemon/target/release/image-oxide        # listens on $XDG_RUNTIME_DIR/image-oxide.sock, else /tmp/image-oxide-$UID.sock
```

Env vars: `IMAGE_OXIDE_TTL_MS` (idle shutdown, default 60s),
`IMAGE_OXIDE_QUEUE` (worker queue capacity, default 32).

## NFR targets

| Number | Target |
|--------|--------|
| Boot-to-accepting p50 | < 50 ms |
| Resize 4000×3000 → 800×600 JPEG p95 | < 300 ms |
| Daemon RSS on 24MP image | ≤ 128 MB |
| Max frame | 64 MiB |
| Worker pool | `min(4, num_cpus)`, queue 32 |

## Encoder decisions (measured)

- **JPEG**: `mozjpeg-rs` `BaselineFastest` — beats GD on resize rtt (0.74–0.99×)
  and output size (−13% at q85). Pure-Rust, keeps the no-C-deps cross-compile.
  mozjpeg's quality scale differs from GD's `imagejpeg($q)`.
- **WebP**: `webp-rust` — `quality ≥ 90` lossless VP8L, else lossy VP8.

## Publish order

Daemon binary release (this repo's `release.yml`) → `haidarrais/image-oxide`
Packagist → `haidarrais/laravel-image-oxide` Packagist.

## Licenses

MIT — see [LICENSE](LICENSE).
