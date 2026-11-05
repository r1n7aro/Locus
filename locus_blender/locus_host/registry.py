"""Registration files Locus watches to discover running Blender instances.

`%LOCALAPPDATA%/Locus/blender/instances/<pid>-<process start ms>.json`. The
start time lets Locus reject a file whose PID was reused by another process.
Writes are atomic (temp file + replace) so a reader never sees half a file.
"""

import ctypes
import json
import os
import tempfile
import time


def locus_data_dir():
    base = os.environ.get("LOCALAPPDATA") or tempfile.gettempdir()
    return os.path.join(base, "Locus", "blender")


def registry_dir():
    return os.environ.get("LOCUS_BLENDER_REGISTRY_DIR") or os.path.join(locus_data_dir(), "instances")


def disabled_marker():
    """Present while the Blender integration is turned off in Locus."""
    return os.path.join(locus_data_dir(), "disabled")


def is_disabled():
    """The add-on stays inert while Locus has Blender turned off. Processes
    Locus launched itself (`LOCUS_BLENDER_LAUNCHED_BY`) always start."""
    if os.environ.get("LOCUS_BLENDER_LAUNCHED_BY"):
        return False
    return os.path.exists(disabled_marker())


class _Filetime(ctypes.Structure):
    _fields_ = [("low", ctypes.c_uint32), ("high", ctypes.c_uint32)]


def process_start_ms():
    """Creation time of this process (unix ms), 0 when unknown."""
    try:
        kernel32 = ctypes.windll.kernel32
        creation, exit_, kernel, user = _Filetime(), _Filetime(), _Filetime(), _Filetime()
        handle = kernel32.GetCurrentProcess()
        if not kernel32.GetProcessTimes(
            ctypes.c_void_p(handle),
            ctypes.byref(creation),
            ctypes.byref(exit_),
            ctypes.byref(kernel),
            ctypes.byref(user),
        ):
            return 0
        ticks = (creation.high << 32) | creation.low  # 100 ns since 1601
        return max(0, ticks // 10_000 - 11_644_473_600_000)
    except (AttributeError, OSError):
        return 0


def registration_path(pid, start_ms):
    return os.path.join(registry_dir(), f"{pid}-{start_ms}.json")


def _replace(source, target, attempts=20):
    """`os.replace` that rides out the moments another process has the old
    file open (Locus reading it after a directory event, virus scanners):
    Windows refuses to replace an open file with ERROR_ACCESS_DENIED."""
    for attempt in range(attempts):
        try:
            os.replace(source, target)
            return
        except PermissionError:
            if attempt == attempts - 1:
                raise
            time.sleep(0.005 * (attempt + 1))


def write(path, record):
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".tmp-", suffix=".json", dir=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(record, handle, ensure_ascii=False, indent=2)
        _replace(temp, path)
    except BaseException:
        try:
            os.unlink(temp)
        except OSError:
            pass
        raise


def remove(path):
    try:
        os.unlink(path)
    except OSError:
        pass
