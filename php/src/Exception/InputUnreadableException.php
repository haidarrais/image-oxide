<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** input.path exists but cannot be opened. Retryable (IPC-20, IPC-22). */
final class InputUnreadableException extends OxideException
{
}
