<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/**
 * Builds the concrete exception type for a daemon error code (PHP-10).
 *
 * The registry mirrors 001 IPC-20 exactly. Codes not listed (or unknown) map to
 * a non-retryable InternalException per IPC-21.
 */
final class ErrorRegistry
{
    private const MAP = [
        'INVALID_REQUEST' => InvalidRequestException::class,
        'FRAME_TOO_LARGE' => FrameTooLargeException::class,
        'PROTOCOL_VERSION_MISMATCH' => ProtocolException::class,
        'ACCESS_DENIED' => AccessDeniedException::class,
        'INPUT_NOT_FOUND' => InputNotFoundException::class,
        'INPUT_UNREADABLE' => InputUnreadableException::class,
        'DECODE_FAILED' => DecodeFailedException::class,
        'UNSUPPORTED_OPERATION' => UnsupportedOperationException::class,
        'OP_FAILED' => OpFailedException::class,
        'OUTPUT_WRITE_FAILED' => OutputWriteFailedException::class,
        'DAEMON_OVERLOADED' => OverloadedException::class,
        'INTERNAL' => InternalException::class,
    ];

    /**
     * @param array{code: string, message: string, op_index: int|null} $error
     */
    public static function from(array $error): OxideException
    {
        $code = (string) $error['code'];
        $message = (string) ($error['message'] ?? '');
        $opIndex = $error['op_index'] ?? null;
        $opIndex = is_int($opIndex) ? $opIndex : null;

        $class = self::MAP[$code] ?? InternalException::class;

        // IPC-21: unknown codes are non-retryable INTERNAL.
        $effectiveCode = isset(self::MAP[$code]) ? $code : 'INTERNAL';

        /** @var OxideException $e */
        $e = new $class($effectiveCode, $message, $opIndex);

        return $e;
    }
}
