# 003 — Image Operations

> **Status:** DRAFT · **Owner:** Rust daemon (`image-oxide`) + GD fallback driver (`laravel-image-oxide`)
> **Dual-implementation contract.** This file is implemented twice: once by the Rust daemon, once by the GD driver behind the PHP client. That duality is what makes graceful degradation a contract, not a shrug. Where the two disagree, this file wins.
> Lives in repo 1; both halves reference it.

- **Context & Decision** · **Definitions** · **Requirements** · **Acceptance Criteria** · **Out of Scope** · **Open Questions**

## Context & Decision

Pixel semantics must be identical whether the Rust daemon or the GD fallback serves a request, so the format matrix and per-op behavior are pinned here. GD's capability table in [004-php-client.md](004-php-client.md) mirrors the matrix below exactly where the two overlap.

Decisions (from PLAN.md):

| # | Question | Decision |
|---|----------|----------|
| D-1 | GD fallback depth | Per-op capability table. Resize/format/rotate/watermark supported; AVIF encode throws `UnsupportedOperationException`. Graceful degradation is a defined contract. |
| D-2 | EXIF orientation | Auto-applied on decode, both implementations. Documented loudly: it silently swaps pixel dimensions on phone photos. |
| D-3 | Watermark | 9-grid positions + px offsets. Opacity 0.0–1.0 multiplies the watermark's alpha. Tiling OOS. |
| D-4 | AVIF | Decode: SHOULD (may slip to v1.1 if it drags C deps into the cross-compile matrix). Encode: OOS for v1. |

## Definitions

- **Format matrix** — decode/encode support × JPEG/PNG/GIF/WebP/AVIF. See `OPS-02`.
- **Op** — one transformation in a request's `ops[]` chain (see [001-daemon-ipc.md](001-daemon-ipc.md), `IPC-12`).
- **EXIF orientation** — the `Orientation` tag (tag 274). `0x00` = not present/unknown.

## Requirements

### General pixel semantics

- `OPS-01` — MUST. Ops apply in array order; each op receives the previous op's output. Output dimensions of one op are the input of the next.
- `OPS-02` — MUST. The format matrix is:

| Format | Decode | Encode |
|--------|--------|--------|
| JPEG   | yes    | yes    |
| PNG    | yes    | yes    |
| GIF    | yes    | yes (first frame only) |
| WebP   | yes    | yes    |
| AVIF   | SHOULD | OOS (v1) |

- `OPS-03` — MUST. EXIF orientation is auto-applied on decode. Both implementations MUST report the post-rotation dimensions in the response (`width`, `height`).
- `OPS-04` — MUST. Unsupported decode → `DECODE_FAILED`; unsupported encode or unsupported op for this build → `UNSUPPORTED_OPERATION`.
- `OPS-05` — SHOULD. Lossy encode quality is a request field `quality` (1–100, default 85) where the format is lossy; ignored for lossless formats.
- `OPS-06` — MUST. All ops validate their parameters; invalid parameters → `OP_FAILED` with `op_index` set.

### Op: resize

- `OPS-07` — MUST. `resize` fields: `width`, `height`, `fit` (`cover` | `contain` | `fill`), optional `position` (`center` | `top` | `top-left` | `top-right` | `bottom` | `bottom-left` | `bottom-right` | `left` | `right`, default `center`).
- `OPS-08` — MUST. `cover` scales to fill `width`×`height`, cropping excess to `position`. `contain` scales to fit inside `width`×`height`, letterboxing to the current background (default transparent/white per format). `fill` stretches to exactly `width`×`height` (distortion allowed).
- `OPS-09` — MUST. Omitting `width` or `height` scales that axis to preserve aspect ratio. Omitting both is invalid (`OP_FAILED`).

### Op: format

- `OPS-10` — MUST. `format` fields: `type` (`jpeg` | `png` | `webp` | `gif` | `avif`). Re-encodes to the target format.
- `OPS-11` — MUST. Converting a GIF to a non-GIF format yields the first frame.
- `OPS-12` — MUST. AVIF encode → `UNSUPPORTED_OPERATION` in v1 (both implementations).

### Op: rotate

- `OPS-13` — MUST. `rotate` field: `degrees` (multiples of 90: 90, 180, 270). Any other value → `OP_FAILED`.
- `OPS-14` — MUST. Rotation is clockwise; dimensions swap on 90°/270°.

### Op: watermark

- `OPS-15` — MUST. `watermark` fields: `image` (path to the watermark image), `position` (9-grid value from `OPS-07`'s list, default `bottom-right`), `offset_x`, `offset_y` (px, default 0), `opacity` (0.0–1.0, default 1.0).
- `OPS-16` — MUST. `opacity` multiplies the watermark's alpha channel. 0.0 = invisible, 1.0 = as-is.
- `OPS-17` — MUST. Offsets move the watermark inward from the grid edge (positive = toward image center). Negative offsets → `OP_FAILED`.

### GD fallback contract

- `OPS-18` — MUST. The GD driver MUST support: `resize`, `format` (jpeg/png/webp/gif), `rotate`, `watermark`.
- `OPS-19` — MUST. GD fallback for AVIF encode MUST throw `UnsupportedOperationException` (`OPS-12`).
- `OPS-20` — MUST. `OPS-03` (EXIF auto-orient) applies to the GD driver too. A GD build without EXIF support MUST surface this at capability-query time, not silently.

## Acceptance Criteria

- `AC-OPS-01` — A 4000×3000 JPEG resized to `{width: 800, height: 600, fit: "cover"}` yields exactly 800×600 with correct crop position, identical output from Rust daemon and GD driver.
- `AC-OPS-02` — `{fit: "contain"}` into 800×600 preserves aspect ratio; one axis equals its target, the other is letterboxed.
- `AC-OPS-03` — A JPEG with EXIF orientation 6 (rotate 90°) decodes to swapped dimensions in both implementations; the response `width`/`height` reflect post-rotation.
- `AC-OPS-04` — `rotate: {degrees: 90}` on a 800×600 image yields 600×800.
- `AC-OPS-05` — Watermark at `bottom-right` with `offset_x: 10, offset_y: 10` sits 10px inside the bottom-right corner; `opacity: 0.5` halves the visible alpha.
- `AC-OPS-06` — AVIF encode returns `UNSUPPORTED_OPERATION` in both implementations.
- `AC-OPS-07` — The GD capability table in 004 matches the `OPS-02` matrix on every overlapping cell.

## Out of Scope

- `OOS-OPS-01` — AVIF encoding in v1 (`OPS-12`). Revisit for v1.1+.
- `OOS-OPS-02` — Watermark tiling/repeat.
- `OOS-OPS-03` — Arbitrary-angle rotation (only 90° multiples).
- `OOS-OPS-04` — Animated GIF encode (first frame only).
- `OOS-OPS-05` — Color-space management / ICC profile preservation across ops. Preserved on decode, dropped on encode (documented, deliberate).

## Open Questions

None.
