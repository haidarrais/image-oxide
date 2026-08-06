<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

use RuntimeException;

/**
 * Base exception for all image-oxide errors (PHP-10).
 *
 * Maps 1:1 onto the 001 error registry. Concrete types are derived from the
 * daemon's SCREAMING_SNAKE_CASE error codes; retryable codes carry a backoff
 * hint (PHP-11).
 */
class OxideException extends RuntimeException
{
    /** The daemon error code (SCREAMING_SNAKE_CASE), or INTERNAL for unknown codes (IPC-21). */
    public readonly string $errorCode;

    /** Index of the failing op in ops[], or null when not op-specific (IPC-18). */
    public readonly ?int $opIndex;

    /**
     * @param string          $errorCode daemon error code
     * @param string          $message   human-readable message
     * @param int|null        $opIndex   op index, if op-specific
     * @param int<0, 60000>   $backoffMs retry backoff hint in ms; 0 = not retryable
     */
    public function __construct(
        string $errorCode,
        string $message,
        ?int $opIndex = null,
        public readonly int $backoffMs = 0,
        ?\Throwable $previous = null,
    ) {
        parent::__construct($message, 0, $previous);
        $this->errorCode = $errorCode;
        $this->opIndex = $opIndex;
    }

    /** True when the error is retryable per IPC-20 (INPUT_UNREADABLE, OUTPUT_WRITE_FAILED, DAEMON_OVERLOADED). */
    public function isRetryable(): bool
    {
        return in_array($this->errorCode, ['INPUT_UNREADABLE', 'OUTPUT_WRITE_FAILED', 'DAEMON_OVERLOADED'], true);
    }
}
