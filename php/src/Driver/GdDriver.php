<?php

declare(strict_types=1);

namespace ImageOxide\Driver;

use ImageOxide\Exception\AccessDeniedException;
use ImageOxide\Exception\DecodeFailedException;
use ImageOxide\Exception\OpFailedException;
use ImageOxide\Exception\UnsupportedOperationException;

/**
 * GD fallback driver (PHP-05/08/09). Implements 003's pixel semantics so that
 * the output matches the daemon's for the same chain (003:3 — the dual-impl
 * graceful-degradation contract). AVIF encode throws UnsupportedOperationException
 * even though this GD build could do it, mirroring OPS-12.
 *
 * @phpstan-type Op array{type: string, ...}
 */
final class GdDriver implements Driver
{
    private const JPEG_QUALITY_DEFAULT = 85;

    private const POSITIONS = [
        'center', 'top', 'top-left', 'top-right',
        'bottom', 'bottom-left', 'bottom-right', 'left', 'right',
    ];

    public function process(string $inputPath, array $ops, string $outputPath, ?int $quality = null): ProcessResult
    {
        $this->assertAbsolute($inputPath, 'input');
        $this->assertAbsolute($outputPath, 'output');

        $img = $this->load($inputPath);
        // Parity with the daemon: the output format is the input format plus
        // any `format` ops (`state.format`), never the output path extension.
        $format = $this->inputMime($inputPath);
        try {
            foreach ($ops as $op) {
                if (($op['type'] ?? null) === 'format') {
                    $format = $this->normalizeFormat((string) ($op['format'] ?? ''));
                }
            }
            foreach ($ops as $op) {
                $img = $this->applyOp($img, $op);
            }
            $width = imagesx($img);
            $height = imagesy($img);
            $this->save($img, $outputPath, $format, $quality);
        } finally {
            if (is_resource($img) || $img instanceof \GdImage) {
                imagedestroy($img);
            }
        }

        return new ProcessResult(
            width: $width,
            height: $height,
            bytes: (int) filesize($outputPath),
        );
    }

    public function capabilities(): Capabilities
    {
        return new Capabilities(
            ops: ['resize' => true, 'format' => true, 'rotate' => true, 'watermark' => true],
            decode: ['jpeg' => true, 'png' => true, 'webp' => true, 'gif' => true, 'avif' => false],
            encode: ['jpeg' => true, 'png' => true, 'webp' => true, 'gif' => true, 'avif' => false],
            fits: ['cover' => true, 'contain' => true, 'fill' => true],
            positions: self::POSITIONS,
        );
    }

    // ---- decode / encode ----

    private function load(string $path): \GdImage
    {
        if (!is_file($path)) {
            throw new DecodeFailedException('INPUT_NOT_FOUND', "input not found: $path");
        }
        $info = @getimagesize($path);
        if ($info === false) {
            throw new DecodeFailedException('DECODE_FAILED', "unrecognized input format: $path");
        }
        $mime = $info['mime'] ?? '';
        $create = match ($mime) {
            'image/jpeg' => 'imagecreatefromjpeg',
            'image/png' => 'imagecreatefrompng',
            'image/webp' => 'imagecreatefromwebp',
            'image/gif' => 'imagecreatefromgif',
            'image/avif' => throw new DecodeFailedException('DECODE_FAILED', 'AVIF decode is deferred to v1.1 (OPS-02)'),
            default => throw new DecodeFailedException('DECODE_FAILED', "unsupported mime: $mime"),
        };
        $img = @$create($path);
        if ($img === false) {
            throw new DecodeFailedException('DECODE_FAILED', "cannot decode: $path");
        }
        // OPS-03: EXIF auto-orientation for JPEG.
        if ($mime === 'image/jpeg') {
            $img = $this->applyExifOrientation($img, $path);
        }
        return $img;
    }

    /**
     * The output format defaults to the input format (daemon parity: the
     * extension of the output path is irrelevant, IPC-12).
     */
    private function inputMime(string $path): string
    {
        $info = @getimagesize($path);
        return match ($info['mime'] ?? '') {
            'image/jpeg' => 'jpeg',
            'image/png' => 'png',
            'image/webp' => 'webp',
            'image/gif' => 'gif',
            default => throw new DecodeFailedException('DECODE_FAILED', "unrecognized input format: $path"),
        };
    }

    /**
     * Mirrors the daemon's `op_format` (`OPS-10`..`OPS-12`).
     */
    private function normalizeFormat(string $format): string
    {
        return match ($format) {
            'jpeg', 'jpg' => 'jpeg',
            'png' => 'png',
            'webp' => 'webp',
            'gif' => 'gif',
            'avif' => throw new UnsupportedOperationException('UNSUPPORTED_OPERATION', 'AVIF encode is out of scope in v1 (`OPS-12`)'),
            default => throw new OpFailedException('OP_FAILED', "invalid format: $format"),
        };
    }

    private function applyExifOrientation(\GdImage $img, string $path): \GdImage
    {
        $orientation = 1;
        if (function_exists('exif_read_data')) {
            $exif = @exif_read_data($path);
            if (is_array($exif) && isset($exif['Orientation'])) {
                $orientation = (int) $exif['Orientation'];
            }
        }
        // Compose the same transforms the daemon's `image` crate applies
        // (`Orientation::from_exif` + `apply_orientation`): 5 = rotate90 CW
        // then fliph; 7 = rotate270 CW then fliph. GD rotates CCW, so CW 90/270
        // are `imagerotate` 270/90.
        return match ($orientation) {
            2 => $this->flip($img, true),
            3 => $this->rotate180($img),
            4 => $this->flip($img, false),
            5 => $this->flip($this->rotate90($img), true),
            6 => $this->rotate90($img),
            7 => $this->flip($this->rotate270($img), true),
            8 => $this->rotate270($img),
            default => $img,
        };
    }

    // ---- op dispatch ----

    /**
     * @param Op $op
     */
    private function applyOp(\GdImage $img, array $op): \GdImage
    {
        switch ($op['type'] ?? null) {
            case 'resize':
                return $this->resize($img, $op);
            case 'format':
                return $img; // encoding happens at save() time
            case 'rotate':
                return $this->rotate($img, $op);
            case 'watermark':
                return $this->watermark($img, $op);
            default:
                throw new UnsupportedOperationException(
                    'UNSUPPORTED_OPERATION',
                    'unsupported op: ' . ($op['type'] ?? '(none)')
                );
        }
    }

    // ---- resize (OPS-07..09) ----

    /**
     * @param Op $op
     */
    private function resize(\GdImage $img, array $op): \GdImage
    {
        $w = $op['width'] ?? null;
        $h = $op['height'] ?? null;
        if ($w === null && $h === null) {
            throw new OpFailedException('OP_FAILED', 'resize requires at least one of width/height (OPS-09)');
        }
        $iw = imagesx($img);
        $ih = imagesy($img);
        $w = $w !== null ? (int) $w : max(1, (int) round($iw * (int) $h / $ih));
        $h = $h !== null ? (int) $h : max(1, (int) round($ih * (int) $w / $iw));
        $fit = (string) ($op['fit'] ?? 'cover');
        $position = (string) ($op['position'] ?? 'center');

        return match ($fit) {
            'cover' => $this->resizeCover($img, $w, $h, $position),
            'contain' => $this->resizeContain($img, $w, $h),
            'fill' => $this->resample($img, $w, $h),
            default => throw new OpFailedException('OP_FAILED', "invalid fit: $fit (cover|contain|fill)"),
        };
    }

    /**
     * OPS-08 cover: scale to fill, crop the excess to the anchor position.
     */
    private function resizeCover(\GdImage $img, int $w, int $h, string $position): \GdImage
    {
        $iw = imagesx($img);
        $ih = imagesy($img);
        $scale = max($w / $iw, $h / $ih);
        $sw = (int) round($iw * $scale);
        $sh = (int) round($ih * $scale);
        $sw = max($sw, $w);
        $sh = max($sh, $h);
        $scaled = $this->resample($img, $sw, $sh);
        $x = $this->cropOffset($sw, $w, $position, true);
        $y = $this->cropOffset($sh, $h, $position, false);
        $out = imagecreatetruecolor($w, $h);
        imagecopy($out, $scaled, 0, 0, $x, $y, $w, $h);
        imagedestroy($scaled);
        return $out;
    }

    /**
     * OPS-08 contain: scale to fit inside, letterbox transparent. Matches the
     * daemon: upscaling beyond 1.0 is capped (scale.min(1.0)).
     */
    private function resizeContain(\GdImage $img, int $w, int $h): \GdImage
    {
        $iw = imagesx($img);
        $ih = imagesy($img);
        $scale = min($w / $iw, $h / $ih, 1.0);
        $sw = max(1, (int) round($iw * $scale));
        $sh = max(1, (int) round($ih * $scale));
        $scaled = $this->resample($img, $sw, $sh);
        $out = imagecreatetruecolor(max(1, $w), max(1, $h));
        $bg = imagecolorallocatealpha($out, 0, 0, 0, 127);
        imagealphablending($out, false);
        imagefilledrectangle($out, 0, 0, max(1, $w) - 1, max(1, $h) - 1, $bg);
        imagealphablending($out, true);
        imagecopy($out, $scaled, (int) (($w - $sw) / 2), (int) (($h - $sh) / 2), 0, 0, $sw, $sh);
        imagedestroy($scaled);
        return $out;
    }

    private function resample(\GdImage $img, int $w, int $h): \GdImage
    {
        $out = imagecreatetruecolor($w, $h);
        imagealphablending($out, false);
        imagesavealpha($out, true);
        $transparent = imagecolorallocatealpha($out, 0, 0, 0, 127);
        imagefilledrectangle($out, 0, 0, $w - 1, $h - 1, $transparent);
        imagecopyresampled($out, $img, 0, 0, 0, 0, $w, $h, imagesx($img), imagesy($img));
        return $out;
    }

    private function cropOffset(int $scaled, int $target, string $position, bool $horizontal): int
    {
        if ($target >= $scaled) {
            return 0;
        }
        $max = $scaled - $target;
        $key = $horizontal
            ? $position
            : match ($position) {
                'top', 'top-left', 'top-right' => 'top',
                'bottom', 'bottom-left', 'bottom-right' => 'bottom',
                default => 'center',
            };
        return match ($key) {
            'left', 'top' => 0,
            'right', 'bottom' => $max,
            default => (int) ($max / 2),
        };
    }

    // ---- rotate (OPS-13, OPS-14) ----

    /**
     * @param Op $op
     */
    private function rotate(\GdImage $img, array $op): \GdImage
    {
        $degrees = (int) ($op['degrees'] ?? 0);
        return match ($degrees) {
            90 => $this->rotate90($img),
            180 => $this->rotate180($img),
            270 => $this->rotate270($img),
            default => throw new OpFailedException('OP_FAILED', "degrees must be 90/180/270 (OPS-13), got $degrees"),
        };
    }

    private function rotate180(\GdImage $img): \GdImage
    {
        $r = imagerotate($img, 180, 0);
        if ($r === false) {
            throw new OpFailedException('OP_FAILED', 'rotate failed');
        }
        return $this->fixRotationAlpha($r);
    }

    /**
     * GD rotates counter-clockwise; image-oxide rotates clockwise (OPS-13).
     * Clockwise 90 = CCW 270.
     */
    private function rotate90(\GdImage $img): \GdImage
    {
        $r = imagerotate($img, 270, 0);
        if ($r === false) {
            throw new OpFailedException('OP_FAILED', 'rotate failed');
        }
        return $this->fixRotationAlpha($r);
    }

    private function rotate270(\GdImage $img): \GdImage
    {
        $r = imagerotate($img, 90, 0);
        if ($r === false) {
            throw new OpFailedException('OP_FAILED', 'rotate failed');
        }
        return $this->fixRotationAlpha($r);
    }

    private function flip(\GdImage $img, bool $horizontal): \GdImage
    {
        $w = imagesx($img);
        $h = imagesy($img);
        $out = imagecreatetruecolor($w, $h);
        imagealphablending($out, false);
        imagesavealpha($out, true);
        for ($y = 0; $y < $h; $y++) {
            for ($x = 0; $x < $w; $x++) {
                $sx = $horizontal ? $w - 1 - $x : $x;
                $sy = $horizontal ? $y : $h - 1 - $y;
                imagesetpixel($out, $x, $y, imagecolorat($img, $sx, $sy));
            }
        }
        imagedestroy($img);
        return $out;
    }

    private function fixRotationAlpha(\GdImage $img): \GdImage
    {
        imagealphablending($img, false);
        imagesavealpha($img, true);
        return $img;
    }

    // ---- watermark (OPS-15..17) ----

    /**
     * @param Op $op
     */
    private function watermark(\GdImage $img, array $op): \GdImage
    {
        $wmPath = (string) ($op['image'] ?? '');
        if ($wmPath === '') {
            throw new OpFailedException('OP_FAILED', 'watermark op missing `image`');
        }
        $position = (string) ($op['position'] ?? 'bottom-right');
        if (!in_array($position, self::POSITIONS, true)) {
            throw new OpFailedException('OP_FAILED', "invalid position: $position");
        }
        $offsetX = $this->nonNeg($op, 'offset_x');
        $offsetY = $this->nonNeg($op, 'offset_y');
        $opacity = (float) ($op['opacity'] ?? 1.0);
        if ($opacity < 0.0 || $opacity > 1.0) {
            throw new OpFailedException('OP_FAILED', 'opacity must be 0.0-1.0 (OPS-15)');
        }

        $wm = $this->load($wmPath);
        try {
            return $this->composite($img, $wm, $position, $offsetX, $offsetY, $opacity);
        } finally {
            imagedestroy($wm);
        }
    }

    private function composite(\GdImage $base, \GdImage $wm, string $position, int $offsetX, int $offsetY, float $opacity): \GdImage
    {
        $bw = imagesx($base);
        $bh = imagesy($base);
        $ww = imagesx($wm);
        $wh = imagesy($wm);
        if ($ww === 0 || $wh === 0 || $ww >= $bw || $wh >= $bh) {
            return $base; // daemon: no-op when the watermark fills/overflows the base
        }

        [$bx, $by] = $this->gridOrigin($bw, $bh, $ww, $wh, $position);
        // OPS-17: offsets move inward from the grid edge.
        if (in_array($position, ['right', 'top-right', 'bottom-right'], true)) {
            $bx = max(0, $bx - $offsetX);
        } elseif (in_array($position, ['left', 'top-left', 'bottom-left'], true)) {
            $bx = min($bw - $ww, $bx + $offsetX);
        }
        if (in_array($position, ['bottom', 'bottom-left', 'bottom-right'], true)) {
            $by = max(0, $by - $offsetY);
        } elseif (in_array($position, ['top', 'top-left', 'top-right'], true)) {
            $by = min($bh - $wh, $by + $offsetY);
        }

        $out = imagecreatetruecolor($bw, $bh);
        imagealphablending($out, false);
        imagesavealpha($out, true);
        imagecopy($out, $base, 0, 0, 0, 0, $bw, $bh);

        // Straight source-over with opacity applied to the watermark alpha (OPS-16).
        imagealphablending($out, true);
        for ($y = 0; $y < $wh; $y++) {
            for ($x = 0; $x < $ww; $x++) {
                $c = imagecolorat($wm, $x, $y);
                $wa = (($c >> 24) & 0x7F) * 2; // GD alpha 0..127 -> 0..254
                $a = (int) round($wa * $opacity);
                if ($a === 0) {
                    continue;
                }
                $wr = ($c >> 16) & 0xFF;
                $wg = ($c >> 8) & 0xFF;
                $wb = $c & 0xFF;

                $d = imagecolorat($out, $bx + $x, $by + $y);
                $dr = ($d >> 16) & 0xFF;
                $dg = ($d >> 8) & 0xFF;
                $db = $d & 0xFF;
                $da = (($d >> 24) & 0x7F) * 2;

                $inv = 255 - $a;
                $nr = (int) (($wr * $a + $dr * $inv) / 255);
                $ng = (int) (($wg * $a + $dg * $inv) / 255);
                $nb = (int) (($wb * $a + $db * $inv) / 255);
                $na = min(255, (int) ($a + ($da * $inv) / 255));
                $na = (int) round($na / 2);

                $col = imagecolorallocatealpha($out, $nr, $ng, $nb, $na);
                if ($col !== false) {
                    imagesetpixel($out, $bx + $x, $by + $y, $col);
                }
            }
        }
        imagedestroy($base);
        return $out;
    }

    /** @return array{0: int, 1: int} */
    private function gridOrigin(int $bw, int $bh, int $ww, int $wh, string $position): array
    {
        $x = str_contains($position, 'right') ? $bw - $ww : (str_contains($position, 'left') ? 0 : (int) (($bw - $ww) / 2));
        $y = str_contains($position, 'bottom') ? $bh - $wh : (str_contains($position, 'top') ? 0 : (int) (($bh - $wh) / 2));
        return [$x, $y];
    }

    private function nonNeg(array $op, string $key): int
    {
        $v = $op[$key] ?? 0;
        $v = (int) $v;
        if ($v < 0) {
            throw new OpFailedException('OP_FAILED', "$key must be a non-negative integer (OPS-17)");
        }
        return $v;
    }

    // ---- encode (OPS-10..12) ----

    private function save(\GdImage $img, string $outputPath, string $format, ?int $quality): void
    {
        $q = $quality ?? self::JPEG_QUALITY_DEFAULT;
        if ($q < 1 || $q > 100) {
            throw new OpFailedException('OP_FAILED', "quality must be 1-100 (OPS-05), got $q");
        }

        // The target format is resolved from input mime + format ops (daemon
        // parity); the output path extension is irrelevant (IPC-12).
        $ok = match ($format) {
            'jpeg' => $this->saveJpeg($img, $outputPath, $q),
            'png' => imagepng($img, $outputPath, 9),
            'webp' => imagewebp($img, $outputPath, $q),
            'gif' => imagegif($img, $outputPath),
            'avif' => throw new UnsupportedOperationException('UNSUPPORTED_OPERATION', 'AVIF encode is out of scope in v1 (`OPS-12`)'),
            default => throw new UnsupportedOperationException('UNSUPPORTED_OPERATION', "unsupported output format: $format"),
        };
        if ($ok === false) {
            throw new OpFailedException('OP_FAILED', "cannot write output: $outputPath");
        }
    }

    private function saveJpeg(\GdImage $img, string $outputPath, int $quality): bool
    {
        // OPS-11: JPEG has no alpha; flatten onto white exactly like the daemon.
        $w = imagesx($img);
        $h = imagesy($img);
        $rgb = imagecreatetruecolor($w, $h);
        $white = imagecolorallocate($rgb, 255, 255, 255);
        imagefilledrectangle($rgb, 0, 0, $w - 1, $h - 1, $white);
        imagealphablending($rgb, true);
        imagesavealpha($rgb, false);
        imagecopy($rgb, $img, 0, 0, 0, 0, $w, $h);
        $ok = imagejpeg($rgb, $outputPath, $quality);
        imagedestroy($rgb);
        return $ok;
    }

    private function assertAbsolute(string $path, string $which): void
    {
        if ($path === '' || $path[0] !== '/') {
            throw new AccessDeniedException('ACCESS_DENIED', "$which.path must be absolute (IPC-14): $path");
        }
    }
}
