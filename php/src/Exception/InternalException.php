<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Anything else, including unknown daemon codes (IPC-21). Never retryable. */
final class InternalException extends OxideException
{
}
