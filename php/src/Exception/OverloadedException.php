<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Worker pool and queue are full. Retryable, apply capped backoff (IPC-20, IPC-22, PHP-11). */
final class OverloadedException extends OxideException
{
}
