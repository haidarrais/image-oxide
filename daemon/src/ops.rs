//! Op engine — pixel semantics per 003. The GD driver in the PHP client must
//! match this exactly; where they disagree, this file wins.
//!
//! Deliberate v1 gaps, documented loudly (constitution 000:3 — degradation over
//! hard failure; 000:4 — boring over clever):
//! - AVIF decode is a SHOULD (`OPS-02`) that slipped to v1.1: enabling it drags
//!   C deps (dav1d) into the cross-compile matrix (`CI-03`). Decode → `DECODE_FAILED`.
//! - AVIF encode → `UNSUPPORTED_OPERATION` (`OPS-12`), both implementations.

use std::fs;
use std::path::{Path, PathBuf};

use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Rgba, RgbaImage};

use crate::error::{ErrorCode, OxideError};
use crate::protocol::Request;

/// OPS-05: lossy encode quality range.
pub const QUALITY_MIN: u8 = 1;
pub const QUALITY_MAX: u8 = 100;
pub const QUALITY_DEFAULT: u8 = 85;

/// Container for the op chain's mutable state (`OPS-01`): each op receives the
/// previous op's output, and the format op switches the encode target.
struct State {
    img: DynamicImage,
    format: ImageFormat,
}

#[derive(Debug)]
pub struct Processed {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

// ---- path validation (`IPC-14`, `IPC-15`) ----

/// Configurable access root (`IPC-15`). Default `/`: the per-UID socket and
/// mode 0600 (`LIFE-01`/`LIFE-02`) already confine access to this user, so the
/// root is a belt-and-suspenders ceiling, not the primary control.
fn access_root() -> PathBuf {
    std::env::var("IMAGE_OXIDE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// `IPC-14` + `IPC-15`: must be absolute and must resolve inside the access root.
fn validate_input_path(path: &str) -> Result<PathBuf, OxideError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(OxideError::new(
            ErrorCode::AccessDenied,
            format!("input.path must be absolute (`IPC-14`): {path}"),
        ));
    }
    let canon = fs::canonicalize(p).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            OxideError::new(ErrorCode::InputNotFound, format!("input not found: {path}"))
        } else {
            OxideError::new(ErrorCode::InputUnreadable, format!("cannot open input: {e}"))
        }
    })?;
    if !canon.starts_with(access_root()) {
        return Err(OxideError::new(
            ErrorCode::AccessDenied,
            format!("path outside access root (`IPC-15`): {path}"),
        ));
    }
    Ok(canon)
}

/// Output path: may not exist yet, so canonicalize the parent and re-attach the
/// file name before the root check.
fn validate_output_path(path: &str) -> Result<PathBuf, OxideError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(OxideError::new(
            ErrorCode::AccessDenied,
            format!("output.path must be absolute (`IPC-14`): {path}"),
        ));
    }
    let parent = p.parent().unwrap_or(Path::new("/"));
    let file = p
        .file_name()
        .ok_or_else(|| OxideError::new(ErrorCode::AccessDenied, "invalid output path"))?;
    let canon_parent = fs::canonicalize(parent).map_err(|e| {
        OxideError::new(
            ErrorCode::OutputWriteFailed,
            format!("output directory not accessible: {e}"),
        )
    })?;
    if !canon_parent.starts_with(access_root()) {
        return Err(OxideError::new(
            ErrorCode::AccessDenied,
            format!("path outside access root (`IPC-15`): {path}"),
        ));
    }
    Ok(canon_parent.join(file))
}

// ---- decode / encode ----

/// `OPS-03`: decode then auto-apply EXIF orientation. Post-rotation dimensions
/// are the ones reported in the response.
fn decode(path: &Path) -> Result<DynamicImage, OxideError> {
    let mut reader = ImageReader::open(path).map_err(|e| {
        OxideError::new(ErrorCode::InputUnreadable, format!("cannot open input: {e}"))
    })?;
    reader.no_limits();
    let format = reader.format().ok_or_else(|| {
        OxideError::new(ErrorCode::DecodeFailed, "unrecognized input format")
    })?;
    // AVIF decode deliberately disabled in v1 (see module doc).
    if format == ImageFormat::Avif {
        return Err(OxideError::new(
            ErrorCode::DecodeFailed,
            "AVIF decode is deferred to v1.1 (`OPS-02`)",
        ));
    }
    let mut decoder = reader.into_decoder().map_err(|e| {
        OxideError::new(ErrorCode::DecodeFailed, format!("cannot create decoder: {e}"))
    })?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let img = DynamicImage::from_decoder(decoder).map_err(|e| {
        OxideError::new(ErrorCode::DecodeFailed, format!("decode failed: {e}"))
    })?;
    Ok(apply_orientation(img, orientation))
}

fn apply_orientation(mut img: DynamicImage, o: Orientation) -> DynamicImage {
    img.apply_orientation(o);
    img
}

/// Composites an image with alpha onto white — needed before JPEG encode, which
/// has no alpha channel (`OPS-08` letterbox default is "transparent/white per
/// format").
fn flatten_onto_white(img: &DynamicImage) -> RgbaImage {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut canvas = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    imageops::overlay(&mut canvas, &rgba, 0, 0);
    canvas
}

fn encode(img: &DynamicImage, format: ImageFormat, quality: u8, out: &mut Vec<u8>) -> Result<(), OxideError> {
    match format {
        ImageFormat::Jpeg => {
            let rgb = if img.color().has_alpha() {
                image::DynamicImage::ImageRgba8(flatten_onto_white(img)).to_rgb8()
            } else {
                img.to_rgb8()
            };
            // mozjpeg-rs (pure-Rust mozjpeg port) — smaller output at equal
            // quality than the image crate's encoder (CI-03 keeps cross-compile
            // free of C deps). `BaselineFastest` is the speed tier; the
            // progressive/trellis presets trade 2-3× encode time for a few more
            // percent, which loses the resize race against GD. mozjpeg's quality
            // scale differs from GD's `imagejpeg($q)` (`OPS-05`).
            let bytes = mozjpeg_rs::Encoder::new(mozjpeg_rs::Preset::BaselineFastest)
                .quality(quality)
                .encode_rgb(&rgb, rgb.width(), rgb.height())
                .map_err(|e| encode_err(format, e))?;
            out.extend_from_slice(&bytes);
            Ok(())
        }
        ImageFormat::WebP => {
            let rgba = img.to_rgba8();
            let buffer = webp_rust::ImageBuffer {
                width: rgba.width() as usize,
                height: rgba.height() as usize,
                rgba: rgba.into_raw(),
            };
            if quality >= 90 {
                // `OPS-05`: high quality → lossless VP8L (webp-rust default
                // lossless config); preserves alpha exactly.
                let config = webp_rust::LosslessEncodingConfig::default();
                let bytes = webp_rust::encode_lossless_with_config(&buffer, &config, None)
                    .map_err(|e| encode_err(format, e))?;
                out.extend_from_slice(&bytes);
            } else {
                // Lossy VP8 (pure-Rust, keeps CI-03's no-C-deps cross-compile).
                // image 0.25's encoder is lossless-only — this replaces it.
                let mut config = webp_rust::LossyEncodingConfig::default();
                config.quality = quality as f32;
                let bytes = webp_rust::encode_lossy_with_config(&buffer, &config, None)
                    .map_err(|e| encode_err(format, e))?;
                out.extend_from_slice(&bytes);
            }
            Ok(())
        }
        ImageFormat::Png => {
            let rgba = img.to_rgba8();
            image::codecs::png::PngEncoder::new(out)
                .write_image(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|e| encode_err(format, e))
        }
        ImageFormat::Gif => {
            let rgba = img.to_rgba8();
            image::codecs::gif::GifEncoder::new(out)
                .encode(&rgba, rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
                .map_err(|e| encode_err(format, e))
        }
        ImageFormat::Avif => Err(OxideError::new(
            ErrorCode::UnsupportedOperation,
            "AVIF encode is out of scope in v1 (`OPS-12`)",
        )),
        other => Err(OxideError::new(
            ErrorCode::UnsupportedOperation,
            format!("unsupported encode format: {other:?}"),
        )),
    }
}

fn encode_err(format: ImageFormat, e: impl std::fmt::Display) -> OxideError {
    OxideError::new(ErrorCode::OpFailed, format!("{format:?} encode failed: {e}"))
}

// ---- op dispatch (`OPS-01`, `OPS-06`) ----

pub fn process_request(req: &Request) -> Result<Processed, OxideError> {
    let input = validate_input_path(&req.input.path)?;
    let output = validate_output_path(&req.output.path)?;
    let quality = validate_quality(req.quality)?;

    let img = decode(&input)?;
    let mut state = State {
        img,
        format: input_format(&input)?,
    };

    for (i, op) in req.ops.iter().enumerate() {
        apply_op(&mut state, op).map_err(|e| e.with_op_index(i))?;
    }

    let (width, height) = state.img.dimensions();
    let mut buf = Vec::new();
    encode(&state.img, state.format, quality, &mut buf)?;

    write_atomic(&output, &buf)?;

    Ok(Processed {
        width,
        height,
        bytes: buf.len() as u64,
    })
}

fn apply_op(state: &mut State, op: &serde_json::Value) -> Result<(), OxideError> {
    let name = op
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or_else(|| OxideError::new(ErrorCode::OpFailed, "op missing `type` field"))?;
    match name {
        "resize" => op_resize(state, op),
        "format" => op_format(state, op),
        "rotate" => op_rotate(state, op),
        "watermark" => op_watermark(state, op),
        other => Err(OxideError::new(
            ErrorCode::UnsupportedOperation,
            format!("unsupported op: {other}"),
        )),
    }
}

fn validate_quality(q: Option<u8>) -> Result<u8, OxideError> {
    match q {
        None => Ok(QUALITY_DEFAULT),
        Some(q) if (QUALITY_MIN..=QUALITY_MAX).contains(&q) => Ok(q),
        Some(q) => Err(OxideError::new(
            ErrorCode::OpFailed,
            format!("quality must be {QUALITY_MIN}–{QUALITY_MAX} (`OPS-05`), got {q}"),
        )),
    }
}

fn input_format(input: &Path) -> Result<ImageFormat, OxideError> {
    let reader = ImageReader::open(input)
        .map_err(|e| OxideError::new(ErrorCode::InputUnreadable, e.to_string()))?;
    reader.format().ok_or_else(|| {
        OxideError::new(ErrorCode::DecodeFailed, "unrecognized input format")
    })
}

// ---- op: resize (`OPS-07`–`OPS-09`) ----

fn op_resize(state: &mut State, op: &serde_json::Value) -> Result<(), OxideError> {
    let width = op.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
    let height = op.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);
    if width.is_none() && height.is_none() {
        return Err(OxideError::new(
            ErrorCode::OpFailed,
            "resize requires at least one of width/height (`OPS-09`)",
        ));
    }
    let fit = op
        .get("fit")
        .and_then(|v| v.as_str())
        .unwrap_or("cover");
    let position = op
        .get("position")
        .and_then(|v| v.as_str())
        .unwrap_or("center");
    validate_position(position)?;

    let (w, h) = resolve_dimensions(state.img.dimensions(), width, height);
    state.img = match fit {
        "cover" => resize_cover(state.img.clone(), w, h, position),
        "contain" => resize_contain(state.img.clone(), w, h),
        "fill" => state.img.resize_exact(w, h, FilterType::Triangle),
        other => {
            return Err(OxideError::new(
                ErrorCode::OpFailed,
                format!("invalid fit: {other} (cover|contain|fill)"),
            ))
        }
    };
    Ok(())
}

fn resolve_dimensions((iw, ih): (u32, u32), w: Option<u32>, h: Option<u32>) -> (u32, u32) {
    match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let h = ((ih as f64 * w as f64 / iw as f64).round() as u32).max(1);
            (w, h)
        }
        (None, Some(h)) => {
            let w = ((iw as f64 * h as f64 / ih as f64).round() as u32).max(1);
            (w, h)
        }
        (None, None) => unreachable!("guarded by caller"),
    }
}

fn validate_position(p: &str) -> Result<(), OxideError> {
    const POSITIONS: [&str; 9] = [
        "center", "top", "top-left", "top-right", "bottom", "bottom-left",
        "bottom-right", "left", "right",
    ];
    if POSITIONS.contains(&p) {
        Ok(())
    } else {
        Err(OxideError::new(
            ErrorCode::OpFailed,
            format!("invalid position: {p}"),
        ))
    }
}

/// `OPS-08` cover: scale to fill target, crop excess to `position`.
fn resize_cover(img: DynamicImage, w: u32, h: u32, position: &str) -> DynamicImage {
    let (iw, ih) = img.dimensions();
    let scale = ((w as f64 / iw as f64).max(h as f64 / ih as f64)).max(f64::MIN_POSITIVE);
    let sw = ((iw as f64 * scale).round() as u32).max(w);
    let sh = ((ih as f64 * scale).round() as u32).max(h);
    let scaled = img.resize_exact(sw, sh, FilterType::Triangle);
    let x = crop_offset(sw, w, position, true);
    let y = crop_offset(sh, h, position, false);
    imageops::crop_imm(&scaled, x, y, w, h).to_image().into()
}

/// `OPS-08` contain: scale to fit inside, letterbox (transparent, flattened to
/// white at JPEG encode time).
fn resize_contain(img: DynamicImage, w: u32, h: u32) -> DynamicImage {
    let (iw, ih) = img.dimensions();
    let scale = (w as f64 / iw as f64).min(h as f64 / ih as f64).min(1.0);
    let sw = ((iw as f64 * scale).round() as u32).max(1);
    let sh = ((ih as f64 * scale).round() as u32).max(1);
    let scaled = img.resize_exact(sw, sh, FilterType::Triangle);
    let mut canvas = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([0, 0, 0, 0]));
    let x = (w - sw) / 2;
    let y = (h - sh) / 2;
    imageops::overlay(&mut canvas, &scaled, x as i64, y as i64);
    DynamicImage::ImageRgba8(canvas)
}

/// Horizontal → `horizontal=true`: left=0, center=middle, right=max.
/// Vertical → `horizontal=false`: top=0, center=middle, bottom=max.
fn crop_offset(scaled: u32, target: u32, position: &str, horizontal: bool) -> u32 {
    if target >= scaled {
        return 0;
    }
    let max = scaled - target;
    let key = if horizontal {
        position
    } else {
        match position {
            "top" | "top-left" | "top-right" => "top",
            "bottom" | "bottom-left" | "bottom-right" => "bottom",
            _ => "center",
        }
    };
    match key {
        "left" | "top" => 0,
        "right" | "bottom" => max,
        _ => max / 2,
    }
}

// ---- op: format (`OPS-10`–`OPS-12`) ----

fn op_format(state: &mut State, op: &serde_json::Value) -> Result<(), OxideError> {
    let ty = op
        .get("format")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OxideError::new(ErrorCode::OpFailed, "format op missing `format`"))?;
    let format = match ty {
        "jpeg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "webp" => ImageFormat::WebP,
        "gif" => ImageFormat::Gif,
        "avif" => {
            return Err(OxideError::new(
                ErrorCode::UnsupportedOperation,
                "AVIF encode is out of scope in v1 (`OPS-12`)",
            ))
        }
        other => {
            return Err(OxideError::new(
                ErrorCode::OpFailed,
                format!("invalid format: {other}"),
            ))
        }
    };
    state.format = format;
    Ok(())
}

// ---- op: rotate (`OPS-13`, `OPS-14`) ----

fn op_rotate(state: &mut State, op: &serde_json::Value) -> Result<(), OxideError> {
    let degrees = op
        .get("degrees")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| OxideError::new(ErrorCode::OpFailed, "rotate op missing `degrees`"))?;
    let o = match degrees {
        90 => Orientation::Rotate90,
        180 => Orientation::Rotate180,
        270 => Orientation::Rotate270,
        other => {
            return Err(OxideError::new(
                ErrorCode::OpFailed,
                format!("degrees must be 90/180/270 (`OPS-13`), got {other}"),
            ))
        }
    };
    state.img = apply_orientation(std::mem::take(&mut state.img), o);
    Ok(())
}

// ---- op: watermark (`OPS-15`–`OPS-17`) ----

fn op_watermark(state: &mut State, op: &serde_json::Value) -> Result<(), OxideError> {
    let wm_path = op
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OxideError::new(ErrorCode::OpFailed, "watermark op missing `image`"))?;
    let position = op
        .get("position")
        .and_then(|v| v.as_str())
        .unwrap_or("bottom-right");
    validate_position(position)?;
    let offset_x = nonneg_u32(op, "offset_x")?;
    let offset_y = nonneg_u32(op, "offset_y")?;
    let opacity = match op.get("opacity") {
        None => 1.0f32,
        Some(v) => v.as_f64().map(|f| f as f32).ok_or_else(|| {
            OxideError::new(ErrorCode::OpFailed, "opacity must be a number")
        })?,
    };
    if !(0.0..=1.0).contains(&opacity) {
        return Err(OxideError::new(
            ErrorCode::OpFailed,
            "opacity must be 0.0–1.0 (`OPS-15`)",
        ));
    }

    let wm_path = validate_input_path(wm_path)?;
    let wm = decode(&wm_path)?;
    state.img = composite_watermark(&state.img, &wm, position, offset_x, offset_y, opacity);
    Ok(())
}

/// `OPS-17`: offsets move the watermark inward from the grid edge. Negative
/// values are rejected before this point.
fn nonneg_u32(op: &serde_json::Value, key: &str) -> Result<u32, OxideError> {
    match op.get(key) {
        None => Ok(0),
        Some(v) => v.as_u64().map(|n| n as u32).ok_or_else(|| {
            OxideError::new(ErrorCode::OpFailed, format!("{key} must be a non-negative integer"))
        }),
    }
}

/// `OPS-16`: opacity multiplies the watermark's alpha before a straight
/// source-over composite.
fn composite_watermark(
    base: &DynamicImage,
    wm: &DynamicImage,
    position: &str,
    offset_x: u32,
    offset_y: u32,
    opacity: f32,
) -> DynamicImage {
    let (bw, bh) = base.dimensions();
    let (ww, wh) = wm.dimensions();
    if ww == 0 || wh == 0 || ww >= bw || wh >= bh {
        return base.clone();
    }
    let (bx, by) = grid_origin(bw, bh, ww, wh, position);
    // Offset moves toward center: on a right-edge anchor, x decreases.
    let bx = match position {
        "right" | "top-right" | "bottom-right" => bx.saturating_sub(offset_x),
        "left" | "top-left" | "bottom-left" => (bx + offset_x).min(bw.saturating_sub(ww)),
        _ => bx,
    };
    let by = match position {
        "bottom" | "bottom-left" | "bottom-right" => by.saturating_sub(offset_y),
        "top" | "top-left" | "top-right" => (by + offset_y).min(bh.saturating_sub(wh)),
        _ => by,
    };

    let wm_rgba = wm.to_rgba8();
    let mut out = base.to_rgba8();
    for y in 0..wh {
        for x in 0..ww {
            let wp = wm_rgba.get_pixel(x, y).0;
            let a = ((wp[3] as f32) * opacity).round() as u32;
            if a == 0 {
                continue;
            }
            let mut dp = *out.get_pixel_mut(bx + x, by + y);
            let inv = 255 - a;
            dp[0] = ((wp[0] as u32 * a + dp[0] as u32 * inv) / 255) as u8;
            dp[1] = ((wp[1] as u32 * a + dp[1] as u32 * inv) / 255) as u8;
            dp[2] = ((wp[2] as u32 * a + dp[2] as u32 * inv) / 255) as u8;
            dp[3] = (a + (dp[3] as u32 * inv) / 255).min(255) as u8;
            out.put_pixel(bx + x, by + y, dp);
        }
    }
    DynamicImage::ImageRgba8(out)
}

fn grid_origin(bw: u32, bh: u32, ww: u32, wh: u32, position: &str) -> (u32, u32) {
    let x = if position.contains("right") {
        bw - ww
    } else if position.contains("left") {
        0
    } else {
        (bw - ww) / 2
    };
    let y = if position == "top" || position.contains("top") {
        0
    } else if position == "bottom" || position.contains("bottom") {
        bh - wh
    } else {
        (bh - wh) / 2
    };
    (x, y)
}

// ---- atomic write (`LIFE-16`) ----

/// Write-to-temp-then-rename so a crash mid-write never leaves a partial file
/// that could be mistaken for a valid result.
fn write_atomic(output: &Path, bytes: &[u8]) -> Result<(), OxideError> {
    let parent = output.parent().unwrap_or(Path::new("/"));
    let file_name = output
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "out".into());
    let tmp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let write_result = fs::write(&tmp, bytes)
        .and_then(|()| fs::rename(&tmp, output));
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(OxideError::new(
            ErrorCode::OutputWriteFailed,
            format!("cannot write output: {e}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse_request;
    use std::io::Write;
    use tempfile::tempdir;

    /// In-memory request builder — the JSON shapes here are the wire shapes of
    /// 001/003 and double as the interop reference.
    fn build(body: serde_json::Value) -> Request {
        parse_request(&serde_json::to_vec(&body).unwrap()).unwrap()
    }

    fn write_input(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p.to_string_lossy().to_string()
    }

    fn png_pixels(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut buf = Vec::new();
        let img = RgbaImage::from_pixel(w, h, Rgba(rgba));
        let rgba = img.into_raw();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn jpeg_pixels(w: u32, h: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        let img = RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 7 % 256) as u8, (y * 13 % 256) as u8, 128, 255])
        });
        image::codecs::jpeg::JpegEncoder::new(&mut buf)
            .encode_image(&image::DynamicImage::ImageRgba8(img).to_rgb8())
            .unwrap();
        buf
    }

    /// Wrap a JPEG in an APP1 EXIF segment carrying orientation `o`.
    /// The TIFF body is parsed by `Orientation::from_exif_chunk`, which reads
    /// the orientation entry directly — minimal single-entry IFD is enough.
    fn jpeg_with_orientation(w: u32, h: u32, o: u16) -> Vec<u8> {
        let mut jpeg = jpeg_pixels(w, h);
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        // Exif orientation = 6 => `Rotate90` (`OPS-03`, `AC-OPS-03`).
        // Little-endian TIFF: header (8 bytes) + 1 IFD entry (12 bytes) + next-IFD offset (4).
        let mut tiff = Vec::new();
        tiff.extend_from_slice(&[0x49, 0x49, 42, 0]); // TIFF magic, LE
        tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
        tiff.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // tag 274 = Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // format SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&o.to_le_bytes()); // value
        tiff.extend_from_slice(&0u16.to_le_bytes()); // padding
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        let mut app1 = vec![0xFF, 0xE1]; // APP1 marker
        let seg_len = (2 + 6 + tiff.len()) as u16;
        app1.extend_from_slice(&seg_len.to_be_bytes());
        app1.extend_from_slice(b"Exif\0\0");
        app1.extend_from_slice(&tiff);
        // Insert APP1 right after the SOI marker.
        jpeg.splice(2..2, app1);
        jpeg
    }

    #[test]
    fn ops_03_exif_orientation_applied_on_decode() {
        // AC-OPS-03: a JPEG with EXIF orientation 6 (rotate 90°) decodes to
        // swapped dimensions; the response width/height reflect post-rotation.
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_with_orientation(800, 600, 6));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "9",
            "ops": [{"type": "format", "format": "png"}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let processed = process_request(&build(body)).unwrap();
        assert_eq!((processed.width, processed.height), (600, 800));
    }

    #[test]
    fn ops_01_ops_execute_in_order() {
        // resize then rotate: final dims 400x200 (90° swaps 200x400 → 400x200).
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(800, 600));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "1",
            "ops": [
                {"type": "resize", "width": 200, "height": 400, "fit": "fill"},
                {"type": "rotate", "degrees": 90}
            ],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let processed = process_request(&build(body)).unwrap();
        assert_eq!((processed.width, processed.height), (400, 200));
    }

    #[test]
    fn ops_07_08_cover_crops_to_800x600() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(4000, 3000));
        let out = dir.path().join("out.webp");
        let body = serde_json::json!({
            "id": "2",
            "ops": [{"type": "resize", "width": 800, "height": 600, "fit": "cover"}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let processed = process_request(&build(body)).unwrap();
        assert_eq!((processed.width, processed.height), (800, 600));
        assert!(out.exists());
    }

    #[test]
    fn ops_09_omitting_both_axes_is_op_failed() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(10, 10));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "3",
            "ops": [{"type": "resize"}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OpFailed);
        assert_eq!(err.op_index, Some(0));
    }

    #[test]
    fn ops_10_format_converts_png() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(64, 64));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "4",
            "ops": [{"type": "format", "format": "png"}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        process_request(&build(body)).unwrap();
        assert_eq!(
            ImageReader::open(&out).unwrap().format(),
            Some(ImageFormat::Png)
        );
    }

    #[test]
    fn ops_12_avif_encode_unsupported() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(16, 16));
        let out = dir.path().join("out.avif");
        let body = serde_json::json!({
            "id": "5",
            "ops": [{"type": "format", "format": "avif"}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnsupportedOperation);
    }

    #[test]
    fn ops_13_14_rotate_90_swaps_dims() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(800, 600));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "6",
            "ops": [{"type": "rotate", "degrees": 90}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let processed = process_request(&build(body)).unwrap();
        assert_eq!((processed.width, processed.height), (600, 800));
    }

    #[test]
    fn ops_13_invalid_degrees_rejected() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(10, 10));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "7",
            "ops": [{"type": "rotate", "degrees": 45}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OpFailed);
    }

    #[test]
    fn ops_15_16_17_watermark_blends_with_opacity() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(200, 200));
        let wm = write_input(dir.path(), "wm.png", &png_pixels(20, 20, [255, 0, 0, 200]));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "8",
            "ops": [
                {"type": "format", "format": "png"},
                {"type": "watermark", "image": wm, "position": "bottom-right",
                 "offset_x": 10, "offset_y": 10, "opacity": 0.5}
            ],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let processed = process_request(&build(body)).unwrap();
        // Dims unchanged by watermark.
        assert_eq!((processed.width, processed.height), (200, 200));
        let out_img = ImageReader::open(&out).unwrap().decode().unwrap();
        // Watermark sits 10px inside bottom-right; sample that pixel — red should
        // be blended (0.5 opacity) toward the white-ish base, not pure red.
        let px = out_img.to_rgba8().get_pixel(200 - 10 - 10, 200 - 10 - 10).0;
        assert!(px[0] > 100, "expected blended red, got {px:?}");
    }

    #[test]
    fn ops_17_negative_offset_rejected() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(100, 100));
        let wm = write_input(dir.path(), "wm.png", &png_pixels(10, 10, [255, 0, 0, 255]));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "9",
            "ops": [{"type": "watermark", "image": wm, "offset_x": -5}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OpFailed);
    }

    #[test]
    fn ipc_14_15_relative_path_denied() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "10",
            "ops": [{"type": "format", "format": "png"}],
            "input": {"path": "relative/in.jpg"},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::AccessDenied);
    }

    #[test]
    fn ipc_20_input_not_found_mapped() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("out.png");
        let missing = dir.path().join("nope.jpg");
        let body = serde_json::json!({
            "id": "11",
            "ops": [{"type": "format", "format": "png"}],
            "input": {"path": missing.to_string_lossy()},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::InputNotFound);
    }

    #[test]
    fn life_16_no_partial_output_on_failure() {
        let dir = tempdir().unwrap();
        let input = write_input(dir.path(), "in.jpg", &jpeg_pixels(16, 16));
        let out = dir.path().join("out.png");
        let body = serde_json::json!({
            "id": "12",
            "ops": [{"type": "rotate", "degrees": 45}],
            "input": {"path": input},
            "output": {"path": out.to_string_lossy()}
        });
        let err = process_request(&build(body)).unwrap_err();
        assert_eq!(err.code, ErrorCode::OpFailed);
        assert!(!out.exists(), "no partial output must be left behind");
    }
}
