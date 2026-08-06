<?php

declare(strict_types=1);

namespace ImageOxide;

use ImageOxide\Driver\Capabilities;
use ImageOxide\Driver\DaemonDriver;
use ImageOxide\Driver\Driver;
use ImageOxide\Driver\GdDriver;
use ImageOxide\Exception\OxideException;
use ImageOxide\Exception\UnsupportedOperationException;
use Psr\Log\LoggerInterface;

/**
 * Fluent API over the 001 wire protocol with a GD fallback (PHP-01..07).
 *
 * Ops are accumulated on the chain and executed only on the terminal call
 * (to()/get()), so one chain can be reused across many images (PHP-04).
 */
final class Oxide
{
    /** @var list<array<string, mixed>> */
    private array $ops = [];

    private ?Driver $driver = null;

    private function __construct(
        private readonly string $inputPath,
        private readonly ?LoggerInterface $logger,
        private readonly ?string $socketPath,
    ) {
    }

    public static function from(string $path, ?LoggerInterface $logger = null, ?string $socketPath = null): self
    {
        return new self($path, $logger, $socketPath);
    }

    // ---- terminal calls ----

    /**
     * PHP-02: write the result to $path and return it.
     */
    public function to(string $outputPath): string
    {
        if ($this->ops === []) {
            throw new OxideException('INVALID_REQUEST', 'no ops to execute; call an op before to()');
        }
        $this->resolveDriver()->process($this->inputPath, $this->ops, $outputPath, $this->qualityValue());
        return $outputPath;
    }

    /**
     * PHP-03: return the output bytes.
     */
    public function get(): string
    {
        $tmp = tempnam(sys_get_temp_dir(), 'oxide');
        if ($tmp === false) {
            throw new OxideException('INTERNAL', 'cannot create temp file');
        }
        try {
            $this->to($tmp);
            $bytes = file_get_contents($tmp);
            if ($bytes === false) {
                throw new OxideException('INTERNAL', 'cannot read output bytes');
            }
            return $bytes;
        } finally {
            @unlink($tmp);
        }
    }

    // ---- ops (mirror 003) ----

    public function resize(int|string $width, int|string $height = 0, string $fit = 'cover', string $position = 'center'): self
    {
        $op = ['type' => 'resize', 'fit' => $fit, 'position' => $position];
        if ($width !== 0) {
            $op['width'] = (int) $width;
        }
        if ($height !== 0) {
            $op['height'] = (int) $height;
        }
        $this->assertSupported('resize');
        $this->ops[] = $op;
        return $this;
    }

    public function format(string $format): self
    {
        $format = strtolower($format);
        $this->assertSupported('format');
        $this->ops[] = ['type' => 'format', 'format' => $format];
        return $this;
    }

    public function rotate(int $degrees): self
    {
        $this->assertSupported('rotate');
        $this->ops[] = ['type' => 'rotate', 'degrees' => $degrees];
        return $this;
    }

    /**
     * OPS-15..17: watermark with a grid position, inward offset, and opacity.
     */
    public function watermark(
        string $imagePath,
        string $position = 'bottom-right',
        int $offsetX = 0,
        int $offsetY = 0,
        float $opacity = 1.0,
    ): self {
        $this->assertSupported('watermark');
        $this->ops[] = [
            'type' => 'watermark',
            'image' => $imagePath,
            'position' => $position,
            'offset_x' => $offsetX,
            'offset_y' => $offsetY,
            'opacity' => $opacity,
        ];
        return $this;
    }

    /**
     * OPS-05: lossy encode quality (1-100). Applied at the terminal call.
     */
    public function quality(int $quality): self
    {
        $this->qualityOverride = $quality;
        return $this;
    }

    // ---- capability query (PHP-06, AC-PHP-04) ----

    /**
     * @return array{daemon: Capabilities, gd: Capabilities}
     */
    public static function capabilities(): array
    {
        return [
            'daemon' => (new DaemonDriver())->capabilities(),
            'gd' => (new GdDriver())->capabilities(),
        ];
    }

    // ---- internals ----

    private ?int $qualityOverride = null;

    private function qualityValue(): ?int
    {
        return $this->qualityOverride;
    }

    private function resolveDriver(): Driver
    {
        if ($this->driver !== null) {
            return $this->driver;
        }
        $socket = $this->socketPath ?? \ImageOxide\Transport\Connection::defaultSocketPath();
        if (DaemonDriver::isReachable($socket)) {
            $this->driver = new DaemonDriver($socket);
            return $this->driver;
        }
        $this->logger?->info('image-oxide: daemon not reachable at {socket}, falling back to GD', ['socket' => $socket]);
        return $this->driver = new GdDriver();
    }

    /**
     * PHP-07: an op the active driver does not support must throw
     * UnsupportedOperationException, never silently no-op.
     */
    private function assertSupported(string $op): void
    {
        $caps = $this->resolveDriver()->capabilities();
        if (!($caps->ops[$op] ?? false)) {
            throw new UnsupportedOperationException('UNSUPPORTED_OPERATION', "op '$op' not supported by the active driver");
        }
    }
}
