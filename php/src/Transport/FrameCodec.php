<?php

declare(strict_types=1);

namespace ImageOxide\Transport;

use ImageOxide\Exception\FrameTooLargeException;

/**
 * 001 framing (IPC-01): a 4-byte big-endian uint32 length N followed by exactly
 * N bytes of UTF-8 JSON. No newline framing.
 */
final class FrameCodec
{
    /** IPC-02/05: maximum frame size, both sides. */
    public const MAX_FRAME = 64 * 1024 * 1024;

    private const HEADER_LEN = 4;

    /**
     * @param resource $stream
     */
    public static function write($stream, string $body): void
    {
        $len = strlen($body);
        if ($len > self::MAX_FRAME) {
            throw new FrameTooLargeException('FRAME_TOO_LARGE', "frame of $len bytes exceeds MAX_FRAME (64 MiB)");
        }
        $header = pack('N', $len);
        // A single fwrite of header+body is atomic enough on a stream socket for
        // one small message; still guard against partial writes.
        $written = 0;
        $out = $header . $body;
        $total = strlen($out);
        while ($written < $total) {
            $n = @fwrite($stream, substr($out, $written));
            if ($n === false || $n === 0) {
                throw new \RuntimeException('failed to write frame to daemon socket');
            }
            $written += $n;
        }
    }

    /**
     * Read exactly one frame.
     *
     * @param resource $stream
     * @return string|null the JSON body, or null on clean EOF before any byte
     * @throws FrameTooLargeException when the declared length exceeds MAX_FRAME
     * @throws \RuntimeException when the stream is truncated mid-frame
     */
    public static function read($stream): ?string
    {
        $header = self::readExact($stream, self::HEADER_LEN);
        if ($header === null) {
            return null;
        }
        $unpacked = unpack('Nlen', $header);
        $len = (int) $unpacked['len'];
        if ($len > self::MAX_FRAME) {
            throw new FrameTooLargeException('FRAME_TOO_LARGE', "frame declares $len bytes, exceeds MAX_FRAME");
        }
        $body = self::readExact($stream, $len);
        if ($body === null) {
            throw new \RuntimeException('daemon closed mid-frame');
        }
        return $body;
    }

    /**
     * @param resource $stream
     * @return string|null null only if EOF occurs before the first byte
     */
    private static function readExact($stream, int $n): ?string
    {
        $buf = '';
        while (strlen($buf) < $n) {
            $chunk = @fread($stream, $n - strlen($buf));
            if ($chunk === false) {
                throw new \RuntimeException('failed to read from daemon socket');
            }
            if ($chunk === '') {
                // EOF: clean only if nothing has been read yet.
                return $buf === '' ? null : throw new \RuntimeException('daemon closed mid-frame');
            }
            $buf .= $chunk;
        }
        return $buf;
    }
}
