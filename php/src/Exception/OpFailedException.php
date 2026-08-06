<?php

declare(strict_types=1);

namespace ImageOxide\Exception;

/** Op ran but produced an error; op_index identifies the failing op (IPC-18). */
final class OpFailedException extends OxideException
{
}
