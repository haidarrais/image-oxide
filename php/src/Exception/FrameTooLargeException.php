<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Frame exceeded the 64 MiB maximum (IPC-02). */
final class FrameTooLargeException extends OxideException
{
}
