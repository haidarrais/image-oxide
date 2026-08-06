<?php

declare(strict_types=1);

namespace ImageOxide\Tests;

use ImageOxide\Driver\DaemonDriver;
use ImageOxide\Driver\GdDriver;
use ImageOxide\Exception\ErrorRegistry;
use ImageOxide\Exception\InternalException;
use ImageOxide\Exception\OverloadedException;
use ImageOxide\Exception\UnsupportedOperationException;
use ImageOxide\Oxide;
use PHPUnit\Framework\TestCase;

/**
 * Acceptance tests for 004 (AC-PHP-01..05). These spawn the real Rust daemon
 * when available (AC-PHP-01/03), mirroring daemon/tests/wire.rs.
 */
final class AcPhpTest extends TestCase
{
    private const BIN = '/Users/redcomm/Public/templates/image-oxide/daemon/target/debug/image-oxide';

    /** @var array{proc: resource, socket: string, runtime: string}|null */
    private ?array $daemon = null;

    protected function tearDown(): void
    {
        if ($this->daemon !== null) {
            if (is_resource($this->daemon['proc'])) {
                proc_terminate($this->daemon['proc']);
                proc_close($this->daemon['proc']);
            }
            @unlink($this->daemon['socket']);
            @rmdir($this->daemon['runtime']);
            $this->daemon = null;
        }
    }

    /**
     * Spawn the daemon on an isolated XDG_RUNTIME_DIR (LIFE-01) so it never
     * touches the user's real socket.
     */
    private function startDaemon(array $env = []): void
    {
        $runtime = sys_get_temp_dir() . '/oxide-php-' . bin2hex(random_bytes(6));
        mkdir($runtime, 0700);
        $socket = $runtime . '/image-oxide.sock';

        // proc_open's env must be string=>string; $_SERVER may hold arrays
        // (e.g. argv), so keep only scalar values.
        $scalarEnv = array_filter(
            array_merge($_ENV, $_SERVER),
            static fn ($v): bool => is_string($v) || is_int($v),
            ARRAY_FILTER_USE_BOTH
        );
        $fullEnv = array_merge($scalarEnv, [
            'XDG_RUNTIME_DIR' => $runtime,
            'IMAGE_OXIDE_TTL_MS' => '30000',
            'IMAGE_OXIDE_QUEUE' => '4',
        ], $env);

        $proc = proc_open(
            [self::BIN],
            [0 => ['file', '/dev/null', 'r'], 1 => ['file', '/dev/null', 'w'], 2 => ['file', '/dev/null', 'w']],
            $pipes,
            dirname(self::BIN),
            $fullEnv
        );
        if (!is_resource($proc)) {
            throw new \RuntimeException('cannot spawn daemon binary ' . self::BIN);
        }

        // Bounded wait for the socket (same approach as wire.rs).
        $deadline = microtime(true) + 5.0;
        while (microtime(true) < $deadline) {
            if (file_exists($socket)) {
                $this->daemon = ['proc' => $proc, 'socket' => $socket, 'runtime' => $runtime];
                return;
            }
            usleep(20000);
        }
        proc_terminate($proc);
        proc_close($proc);
        throw new \RuntimeException('daemon did not create its socket within 5s');
    }

    private function socket(): string
    {
        if ($this->daemon === null) {
            throw new \LogicException('daemon not started');
        }
        return $this->daemon['socket'];
    }

    // ---- fixtures ----

    private static function writePng(string $path, int $w, int $h): void
    {
        $img = imagecreatetruecolor($w, $h);
        // Opaque checkerboard so encode comparison is meaningful.
        for ($y = 0; $y < $h; $y++) {
            for ($x = 0; $x < $w; $x++) {
                $c = (($x + $y) % 2 === 0) ? [255, 0, 0] : [0, 0, 255];
                imagesetpixel($img, $x, $y, imagecolorallocate($img, $c[0], $c[1], $c[2]));
            }
        }
        imagepng($img, $path);
        imagedestroy($img);
    }

    private static function tempDir(): string
    {
        $d = sys_get_temp_dir() . '/oxide-php-fixture-' . bin2hex(random_bytes(6));
        mkdir($d, 0700);
        return $d;
    }

    // ---- AC-PHP-01: daemon + GD produce identical geometry ----

    public function testAcPhp01ResizeFormatViaDaemon(): void
    {
        $this->startDaemon();
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 1600, 1200); // PNG body; daemon infers format from header

        $out = $dir . '/out.webp';
        $result = Oxide::from($input, socketPath: $this->socket())
            ->resize(800, 600, 'cover')
            ->format('webp')
            ->to($out);

        self::assertSame($out, $result);
        self::assertFileExists($out);
        $info = getimagesize($out);
        self::assertSame(800, $info[0]);
        self::assertSame(600, $info[1]);
        self::assertSame('image/webp', $info['mime']);
    }

    public function testAcPhp01SameChainThroughGd(): void
    {
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 1600, 1200);

        $out = $dir . '/out.webp';
        Oxide::from($input, socketPath: '/nonexistent.sock')
            ->resize(800, 600, 'cover')
            ->format('webp')
            ->to($out);

        $info = getimagesize($out);
        self::assertSame(800, $info[0]);
        self::assertSame(600, $info[1]);
        self::assertSame('image/webp', $info['mime']);
    }

    public function testAcPhp01GetReturnsBytes(): void
    {
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 100, 80);

        $bytes = Oxide::from($input, socketPath: '/nonexistent.sock')
            ->resize(50, 40)
            ->get();

        self::assertNotSame('', $bytes);
        $tmp = $dir . '/probe.bin';
        file_put_contents($tmp, $bytes);
        $info = getimagesize($tmp);
        // No format op: output format = input format (PNG), daemon parity.
        self::assertSame('image/png', $info['mime']);
        self::assertSame(50, $info[0]);
        self::assertSame(40, $info[1]);
        @unlink($tmp);
    }

    // ---- AC-PHP-02: AVIF ----

    public function testAcPhp02AvifEncodeThrowsOnGd(): void
    {
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 10, 10);

        $this->expectException(UnsupportedOperationException::class);
        Oxide::from($input, socketPath: '/nonexistent.sock')
            ->format('avif')
            ->to($dir . '/out.avif');
    }

    // ---- AC-PHP-03: overloaded daemon ----

    public function testAcPhp03OverloadedSurfacesAfterBackoff(): void
    {
        $this->startDaemon(['IMAGE_OXIDE_QUEUE' => '0']);
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 10, 10);

        // Drive the daemon directly (bypass Oxide::from's auto-fallback, which
        // would see the overload-reject probe fail and silently use GD).
        $start = microtime(true);
        try {
            (new DaemonDriver($this->socket()))->process(
                $input,
                [['type' => 'resize', 'width' => 5, 'height' => 5]],
                $dir . '/out.png'
            );
            self::fail('expected OverloadedException');
        } catch (OverloadedException $e) {
            self::assertSame('DAEMON_OVERLOADED', $e->errorCode);
            self::assertTrue($e->isRetryable());
        }
        $elapsed = microtime(true) - $start;
        // Base backoff 50ms + 100ms + 200ms >= 350ms total.
        self::assertGreaterThan(0.35, $elapsed, 'retries must back off, not hammer');
    }

    public function testAcPhp03RecoversOnceDaemonIsUp(): void
    {
        $this->startDaemon();
        $dir = self::tempDir();
        $input = $dir . '/in.png';
        self::writePng($input, 20, 20);

        $out = $dir . '/out.png';
        Oxide::from($input, socketPath: $this->socket())
            ->resize(10, 10)
            ->to($out);
        self::assertFileExists($out);
    }

    // ---- AC-PHP-04: capabilities matrix matches 003 ----

    public function testAcPhp04GdCapabilitiesMatrix(): void
    {
        $caps = (new GdDriver())->capabilities();

        self::assertSame(
            ['resize' => true, 'format' => true, 'rotate' => true, 'watermark' => true],
            $caps->ops
        );
        // OPS-02: decode jpeg/png/webp/gif, avif deferred to v1.1.
        self::assertSame(
            ['jpeg' => true, 'png' => true, 'webp' => true, 'gif' => true, 'avif' => false],
            $caps->decode
        );
        // OPS-10..12: encode same set; avif out of scope in v1.
        self::assertSame(
            ['jpeg' => true, 'png' => true, 'webp' => true, 'gif' => true, 'avif' => false],
            $caps->encode
        );
        self::assertSame(['cover' => true, 'contain' => true, 'fill' => true], $caps->fits);
        self::assertSame(
            ['center', 'top', 'top-left', 'top-right', 'bottom', 'bottom-left', 'bottom-right', 'left', 'right'],
            $caps->positions
        );
    }

    public function testAcPhp04StaticCapabilitiesReturnsBothDrivers(): void
    {
        $caps = Oxide::capabilities();
        self::assertArrayHasKey('daemon', $caps);
        self::assertArrayHasKey('gd', $caps);
        self::assertInstanceOf(\ImageOxide\Driver\Capabilities::class, $caps['daemon']);
        self::assertInstanceOf(\ImageOxide\Driver\Capabilities::class, $caps['gd']);
    }

    // ---- AC-PHP-05: unknown error code -> non-retryable INTERNAL ----

    public function testAcPhp05UnknownCodeMapsToInternal(): void
    {
        $e = ErrorRegistry::from([
            'code' => 'SOME_FUTURE_CODE',
            'message' => 'we do not know it yet',
            'op_index' => null,
        ]);

        self::assertInstanceOf(InternalException::class, $e);
        self::assertSame('INTERNAL', $e->errorCode);
        self::assertFalse($e->isRetryable(), 'unknown codes must be non-retryable (IPC-21)');
    }

    // ---- PHP-05: reachability probe ----

    public function testPhp05ReachabilityProbeRejectsStaleSocket(): void
    {
        // A leftover socket file with no daemon behind it must not be
        // considered "reachable" (PHP-05).
        $dir = self::tempDir();
        $stale = $dir . '/image-oxide.sock';
        file_put_contents($stale, 'not a real socket');

        self::assertFalse(DaemonDriver::isReachable($stale));
        self::assertFileExists($stale, 'probe must not delete the file');
    }

    public function testPhp05ReachabilityProbeAcceptsLiveDaemon(): void
    {
        $this->startDaemon();
        self::assertTrue(DaemonDriver::isReachable($this->socket()));
    }
}
