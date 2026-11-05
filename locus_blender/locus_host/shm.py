"""Named shared memory for large payloads (bytes, arrays, images).

Payloads above the inline limit are written by the sender into
`Local\\locus_blender_<call>_<seq>`; the message only carries the name and the
size. The receiver maps the same name, copies, and sends `release_shm`; the
sender then drops its mapping. Windows frees the memory when the last handle
closes, so a crash on either side leaks nothing.
"""

import mmap
import secrets
import threading
import time

DEFAULT_TTL_SECONDS = 120


class ShmPool:
    """Mappings this process created and still has to keep alive."""

    def __init__(self, ttl_seconds=DEFAULT_TTL_SECONDS):
        self._lock = threading.Lock()
        self._live = {}  # name -> (mmap, created_at)
        self._ttl = ttl_seconds
        self._seq = 0

    def put(self, data, call_id="x"):
        data = memoryview(data).cast("B")
        size = len(data)
        with self._lock:
            self._seq += 1
            name = f"Local\\locus_blender_{_safe(call_id)}_{self._seq}_{secrets.token_hex(4)}"
        mapping = mmap.mmap(-1, max(size, 1), tagname=name)
        mapping[:size] = data
        with self._lock:
            self._live[name] = (mapping, time.monotonic())
        self.collect()
        return {"shm": name, "size": size}

    def release(self, names):
        with self._lock:
            for name in names or ():
                entry = self._live.pop(name, None)
                if entry is not None:
                    try:
                        entry[0].close()
                    except Exception:
                        pass

    def collect(self):
        """Drop mappings nobody acknowledged within the TTL."""
        now = time.monotonic()
        with self._lock:
            stale = [name for name, (_, created) in self._live.items() if now - created > self._ttl]
        self.release(stale)

    def clear(self):
        with self._lock:
            names = list(self._live)
        self.release(names)


def read(ref):
    """Copy a payload another process published (`{"shm", "size"}`)."""
    name = str(ref["shm"])
    size = int(ref["size"])
    if size == 0:
        return b""
    mapping = mmap.mmap(-1, size, tagname=name)
    try:
        return bytes(mapping[:size])
    finally:
        mapping.close()


def _safe(value):
    return "".join(c for c in str(value) if c.isalnum() or c in "-_")[:40] or "x"
