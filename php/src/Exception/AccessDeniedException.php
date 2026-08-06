<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Path outside the daemon's allowed roots, or permission denied (IPC-15). */
final class AccessDeniedException extends OxideException
{
}
