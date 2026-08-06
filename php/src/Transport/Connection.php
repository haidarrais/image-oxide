<?php

declare(strict_types=1);

namespace ImageOxide\Transport;

use ImageOxide\Exception\ErrorRegistry;
use ImageOxide\Exception\OxideException;
use ImageOxide\Exception\ProtocolException;

/**
 * A single daemon connection: hello/ack handshake then exactly one request
 * (IPC-06..09, IPC-19, IPC-23). A fresh connection is established per terminal
 * call (PHP-13). Paths are always absolute (PHP-14).
 */
final class Connection
{
    public const PROTOCOL_VERSION = '1.0.0';

    public const CLIENT_NAME = 'haidarrais/image-oxide-php';

    private const SOCKET_TIMEOUT_S = 30.0;

    /** @var resource|null */
    private $stream = null;

    public function __construct(private readonly string $socketPath)
    {
    }

    /**
     * Resolve the daemon socket path identically to the daemon (LIFE-01):
     * $XDG_RUNTIME_DIR/image-oxide.sock, else /tmp/image-oxide-$UID.sock.
     */
    public static function defaultSocketPath(): string
    {
        $xdg = getenv('XDG_RUNTIME_DIR');
        if (is_string($xdg) && $xdg !== '') {
            return rtrim($xdg, '/') . '/image-oxide.sock';
        }
        $uid = function_exists('posix_geteuid') ? posix_geteuid() : getmyuid();
        return '/tmp/image-oxide-' . $uid . '.sock';
    }

    /**
     * Send a request and read the reply. Retries are the caller's job (IPC-22);
     * this connection is single-shot and closed afterwards.
     *
     * @return array{status: string, id: string, output_path?: string, bytes?: int, width?: int, height?: int, duration_ms?: int, error?: array{code: string, message: string, op_index: int|null}}
     */
    public function roundTrip(array $request): array
    {
        $this->connect();
        try {
            $this->handshake();
            $this->writeRequest($request);
            $reply = $this->readReply();
        } finally {
            $this->close();
        }

        if (($reply['status'] ?? null) !== 'ok') {
            throw ErrorRegistry::from($reply['error'] ?? ['code' => 'INTERNAL', 'message' => 'daemon returned non-ok status', 'op_index' => null]);
        }
        return $reply;
    }

    /**
     * Lightweight reachability probe (PHP-05): connect, complete the handshake,
     * close. Throws on any failure.
     */
    public function probe(): void
    {
        $this->connect();
        try {
            $this->handshake();
        } finally {
            $this->close();
        }
    }

    private function connect(): void
    {
        if (!file_exists($this->socketPath)) {
            throw new \RuntimeException("daemon socket not found at {$this->socketPath}");
        }
        $errno = 0;
        $errstr = '';
        $stream = @stream_socket_client(
            'unix://' . $this->socketPath,
            $errno,
            $errstr,
            self::SOCKET_TIMEOUT_S
        );
        if ($stream === false) {
            throw new \RuntimeException("cannot connect to daemon at {$this->socketPath}: $errstr");
        }
        stream_set_timeout($stream, (int) self::SOCKET_TIMEOUT_S);
        $this->stream = $stream;
    }

    /**
     * IPC-06/07: hello with protocol_version + client_name; daemon replies ack
     * with its own versions. IPC-08: mismatch -> daemon replies PROTOCOL_VERSION_MISMATCH and closes.
     */
    private function handshake(): void
    {
        FrameCodec::write($this->stream, json_encode([
            'type' => 'hello',
            'protocol_version' => self::PROTOCOL_VERSION,
            'client_name' => self::CLIENT_NAME,
        ], JSON_THROW_ON_ERROR));

        $body = FrameCodec::read($this->stream);
        if ($body === null) {
            throw new ProtocolException('INTERNAL', 'daemon closed during handshake');
        }
        $ack = json_decode($body, true);
        if (!is_array($ack) || ($ack['type'] ?? null) !== 'ack') {
            // Daemon replied with an error frame (e.g. version mismatch).
            $error = $ack['error'] ?? null;
            if (is_array($error)) {
                throw ErrorRegistry::from($error);
            }
            throw new ProtocolException('INTERNAL', 'unexpected handshake reply');
        }
    }

    private function writeRequest(array $request): void
    {
        FrameCodec::write($this->stream, json_encode($request, JSON_THROW_ON_ERROR));
    }

    private function readReply(): array
    {
        $body = FrameCodec::read($this->stream);
        if ($body === null) {
            throw new \RuntimeException('daemon closed without a reply');
        }
        $reply = json_decode($body, true);
        if (!is_array($reply)) {
            throw new \RuntimeException('daemon returned malformed reply');
        }
        return $reply;
    }

    private function close(): void
    {
        if (is_resource($this->stream)) {
            fclose($this->stream);
        }
        $this->stream = null;
    }
}
