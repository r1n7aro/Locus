"""ctypes binding to `locus_blender_native_<hash>.dll`.

ctypes releases the GIL around every call, so blocking calls
(`wait_request`) never hold up other Python threads, and the broker thread
inside the DLL never needs the GIL. Mirrors `LocusBridge.Native.cs`.
"""

import ctypes
import json
import os

from . import version

_c = ctypes


class NativeLoadError(RuntimeError):
    pass


def bin_dir():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin")


def resolve_dll(directory=None):
    """Path of the DLL named by `bin/native.json`."""
    directory = directory or bin_dir()
    meta_path = os.path.join(directory, "native.json")
    try:
        with open(meta_path, "r", encoding="utf-8") as handle:
            meta = json.load(handle)
    except FileNotFoundError:
        raise NativeLoadError(f"native layer missing: {meta_path} (run `bun run blender:bundle-native`)")
    except (OSError, ValueError) as error:
        raise NativeLoadError(f"native layer manifest unreadable: {meta_path}: {error}")
    name = str(meta.get("dll") or "")
    path = os.path.join(directory, name)
    if not name or not os.path.isfile(path):
        raise NativeLoadError(f"native layer DLL missing: {path}")
    return path, meta


def _bytes(value):
    if value is None:
        return None, 0
    data = value.encode("utf-8") if isinstance(value, str) else bytes(value)
    return data, len(data)


class Native:
    """One loaded broker DLL. Kept in `persist.state().native` across reloads."""

    def __init__(self, path):
        self.path = path
        try:
            lib = _c.CDLL(path)
        except OSError as error:
            raise NativeLoadError(f"failed to load {path}: {error}")
        self._lib = lib
        sig = self._sig
        sig("locus_protocol_version", [], _c.c_int)
        sig("locus_init_host", [_c.c_char_p, _c.c_int, _c.c_char_p, _c.c_int, _c.c_char_p, _c.c_int, _c.c_int], _c.c_int)
        sig("locus_set_auth_token", [_c.c_char_p, _c.c_int], None)
        sig("locus_set_managed_state", [_c.c_int, _c.c_int64, _c.c_char_p, _c.c_int], None)
        sig("locus_managed_heartbeat", [_c.c_int64], None)
        sig("locus_managed_execution", [_c.c_int], None)
        sig("locus_set_capabilities", [_c.c_char_p, _c.c_int], None)
        sig("locus_poll_request", [_c.c_void_p, _c.c_int, _c.POINTER(_c.c_int)], _c.c_int)
        sig("locus_wait_request", [_c.c_int], _c.c_int)
        sig("locus_complete_request", [_c.c_char_p, _c.c_int, _c.c_char_p, _c.c_int], None)
        sig("locus_emit_event", [_c.c_char_p, _c.c_int, _c.c_char_p, _c.c_int], None)
        sig("locus_set_host_info", [_c.c_char_p, _c.c_int], None)
        sig("locus_execution_publish", [_c.c_char_p, _c.c_int, _c.c_char_p, _c.c_int], None)
        sig("locus_execution_end", [_c.c_char_p, _c.c_int], None)
        sig("locus_execution_cancel_seq", [], _c.c_uint64)
        sig("locus_execution_cancel_requested", [_c.c_char_p, _c.c_int, _c.c_uint64], _c.c_int)
        sig("locus_main_thread_stall_ms", [], _c.c_int64)
        sig("locus_has_connected_client", [], _c.c_int)
        sig("locus_shutdown", [], None)
        protocol = lib.locus_protocol_version()
        if protocol != version.NATIVE_PROTOCOL:
            raise NativeLoadError(f"native layer protocol {protocol}, host expects {version.NATIVE_PROTOCOL}")
        self._buffer = _c.create_string_buffer(64 * 1024)

    def _sig(self, name, argtypes, restype):
        try:
            function = getattr(self._lib, name)
        except AttributeError:
            raise NativeLoadError(f"native layer lacks {name}")
        function.argtypes = argtypes
        function.restype = restype

    # ── lifecycle ──────────────────────────────────────────────────────────
    def init_host(self, instance_key, pipe_name, state_plane, protocol):
        key, key_len = _bytes(instance_key)
        pipe, pipe_len = _bytes(pipe_name)
        plane, plane_len = _bytes(state_plane)
        return self._lib.locus_init_host(key, key_len, pipe, pipe_len, plane, plane_len, protocol)

    def set_auth_token(self, token):
        data, length = _bytes(token)
        self._lib.locus_set_auth_token(data, length)

    def set_managed_state(self, state, generation, status=None):
        data, length = _bytes(status)
        self._lib.locus_set_managed_state(state, generation, data, length)

    def heartbeat(self, generation):
        self._lib.locus_managed_heartbeat(generation)

    def managed_execution(self, active):
        self._lib.locus_managed_execution(1 if active else 0)

    def set_capabilities(self, capabilities):
        data, length = _bytes(",".join(capabilities))
        self._lib.locus_set_capabilities(data, length)

    def set_host_info(self, info):
        data, length = _bytes(json.dumps(info, ensure_ascii=False))
        self._lib.locus_set_host_info(data, length)

    def shutdown(self):
        self._lib.locus_shutdown()

    # ── requests ───────────────────────────────────────────────────────────
    def poll_request(self):
        """Next queued request line (bytes) or None."""
        required = _c.c_int(0)
        while True:
            size = len(self._buffer)
            written = self._lib.locus_poll_request(_c.addressof(self._buffer), size, _c.byref(required))
            if written > 0:
                return self._buffer.raw[:written]
            if written == 0:
                return None
            self._buffer = _c.create_string_buffer(max(required.value, size * 2))

    def wait_request(self, timeout_ms):
        return self._lib.locus_wait_request(int(timeout_ms))

    def complete(self, request_id, response):
        rid, rid_len = _bytes(request_id)
        data, length = _bytes(response)
        self._lib.locus_complete_request(rid, rid_len, data, length)

    def emit_event(self, event_type, payload):
        kind, kind_len = _bytes(event_type)
        data, length = _bytes(payload if isinstance(payload, (str, bytes)) else json.dumps(payload, ensure_ascii=False))
        self._lib.locus_emit_event(kind, kind_len, data, length)

    # ── executions ─────────────────────────────────────────────────────────
    def execution_publish(self, execution_id, snapshot):
        eid, eid_len = _bytes(execution_id)
        data, length = _bytes(json.dumps(snapshot, ensure_ascii=False))
        self._lib.locus_execution_publish(eid, eid_len, data, length)

    def execution_end(self, execution_id):
        eid, eid_len = _bytes(execution_id)
        self._lib.locus_execution_end(eid, eid_len)

    def execution_cancel_seq(self):
        return self._lib.locus_execution_cancel_seq()

    def execution_cancel_requested(self, execution_id, since_seq):
        eid, eid_len = _bytes(execution_id)
        return bool(self._lib.locus_execution_cancel_requested(eid, eid_len, since_seq))

    def main_thread_stall_ms(self):
        return self._lib.locus_main_thread_stall_ms()

    def has_connected_client(self):
        return bool(self._lib.locus_has_connected_client())
