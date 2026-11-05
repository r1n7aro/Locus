"""Host lifecycle: broker DLL, registration file, request pump, reload.

Interactive Blender: one persistent timer runs every 50 ms on the main
thread; it pulses the broker heartbeat and runs queued requests. That is the
only main-thread cost while idle. Background Blender (`-b`): the main thread
blocks in `locus_wait_request` (GIL released) and runs requests the moment
they arrive.
"""

import atexit
import json
import os
import secrets
import sys
import time

from . import codec, executor, native as native_mod, persist, registry, shm, version
from .helpers import MANIFEST

INITIALIZING, READY, RELOADING, QUITTING = 0, 1, 2, 3
TICK_SECONDS = 0.05
TICK_BUDGET_SECONDS = 0.5
WAIT_TIMEOUT_MS = 1000

_runtime = None  # Runtime for the code currently loaded (recreated on reload)


def _bpy():
    import bpy
    return bpy


def _text(value):
    return value.decode("utf-8", "replace") if isinstance(value, bytes) else str(value)


def _host_dir():
    return os.path.dirname(os.path.abspath(__file__))


def host_hash():
    try:
        with open(os.path.join(_host_dir(), ".locus_host_hash"), "r", encoding="utf-8") as handle:
            return handle.read().strip() or "source"
    except OSError:
        return "source"


def _launched_by():
    raw = os.environ.get("LOCUS_BLENDER_LAUNCHED_BY")
    if not raw:
        return None
    try:
        value = json.loads(raw)
        return value if isinstance(value, dict) else None
    except ValueError:
        return None


class Runtime:
    """Objects owned by the currently loaded code."""

    def __init__(self, native):
        self.native = native
        self.shm_pool = shm.ShmPool()
        self.executor = executor.Executor(native, self.shm_pool, instance_info)


# ── identity and status ────────────────────────────────────────────────────


def blender_info():
    bpy = _bpy()
    return {
        "version": ".".join(str(v) for v in bpy.app.version),
        "version_string": bpy.app.version_string,
        "build_hash": _text(bpy.app.build_hash),
        "build_branch": _text(bpy.app.build_branch),
        "binary": bpy.app.binary_path,
    }


def file_state():
    # Add-ons in `startup/` register while `bpy.data` is still the restricted
    # `_RestrictData` placeholder; the file is known after `load_post`.
    data = _bpy().data
    try:
        path = data.filepath
        dirty = bool(data.is_dirty)
        saved = ".".join(str(v) for v in data.version) if path else None
    except AttributeError:
        return {"file": None, "dirty": False, "saved_version": None}
    return {"file": path or None, "dirty": dirty, "saved_version": saved}


def instance_info():
    state = persist.state()
    info = {
        "instance": (state.instance or {}).get("key"),
        "pid": os.getpid(),
        "mode": state.mode,
        "blender": _bpy().app.version_string,
    }
    info.update(file_state())
    return info


def _status_string():
    status = {"mode": persist.state().mode}
    status.update(file_state())
    return json.dumps(status, ensure_ascii=False)


def _registration_record(state, extra=None):
    instance = state.instance or {}
    record = {
        "pid": os.getpid(),
        "process_start_ms": instance.get("start_ms", 0),
        "instance_key": instance.get("key"),
        "pipe": instance.get("pipe"),
        "token": instance.get("token"),
        "protocol": version.HOST_PROTOCOL,
        "native_protocol": version.NATIVE_PROTOCOL,
        "native_dll": os.path.basename(state.native.path) if state.native else None,
        "host_hash": host_hash(),
        "host_dir": _host_dir(),
        "mode": state.mode,
        "launched_by": _launched_by(),
        "blender": blender_info(),
        "python": sys.version.split()[0],
        "started_at_ms": instance.get("started_at_ms"),
        "updated_at_ms": int(time.time() * 1000),
        "state": instance.get("state", "ready"),
        "capabilities": list(version.HOST_CAPABILITIES),
    }
    record.update(file_state())
    if extra:
        record.update(extra)
    return record


def _write_registration(state, extra=None):
    instance = state.instance or {}
    path = instance.get("registration")
    if path:
        try:
            registry.write(path, _registration_record(state, extra))
        except OSError as error:
            print(f"[Locus] failed to write Blender registration: {error}", file=sys.stderr)


def _publish(state):
    if state.native is None:
        return
    info = {"blender": blender_info(), "python": sys.version.split()[0], "host_hash": host_hash(),
            "mode": state.mode, "pid": os.getpid()}
    info.update(file_state())
    state.native.set_host_info(info)
    state.native.set_managed_state(READY, state.generation, _status_string())


# ── start / stop ───────────────────────────────────────────────────────────


def start():
    """Start the host once per process. Returns True when it is running."""
    global _runtime
    state = persist.state()
    if state.started:
        return True
    bpy = _bpy()
    if tuple(bpy.app.version) < version.MIN_BLENDER:
        print(f"[Locus] Blender {bpy.app.version_string} is older than 4.2; Locus host disabled")
        return False
    if registry.is_disabled():
        return False
    state.mode = "background" if bpy.app.background else "interactive"
    if state.native is not None and getattr(state, "broker_live", False):
        # The broker of an earlier, interrupted start in this process is
        # still listening; `locus_init_host` would keep its pipe, so keep the
        # identity (key, pipe, token) that the registration must describe.
        native = state.native
    else:
        key = f"{os.getpid()}_{secrets.token_hex(4)}"
        start_ms = registry.process_start_ms()
        state.instance = {
            "key": key,
            "pipe": "\\\\.\\pipe\\locus_blender_" + key,
            "token": secrets.token_hex(32),
            "start_ms": start_ms,
            "started_at_ms": int(time.time() * 1000),
            "registration": registry.registration_path(os.getpid(), start_ms),
            "state": "ready",
        }
        try:
            dll_path, _ = native_mod.resolve_dll()
            native = native_mod.Native(dll_path)
            if native.init_host(key, state.instance["pipe"], "", version.NATIVE_PROTOCOL) != 0:
                raise native_mod.NativeLoadError("locus_init_host failed")
        except native_mod.NativeLoadError as error:
            # No pure-Python fallback: record why and stay inert.
            state.instance["state"] = "error"
            _write_registration(state, {"error": str(error), "pipe": None, "token": None})
            print(f"[Locus] Blender host not started: {error}", file=sys.stderr)
            return False
        native.set_auth_token(state.instance["token"])
        state.native = native
        state.broker_live = True
    native.set_capabilities(version.HOST_CAPABILITIES)
    state.generation += 1
    _runtime = Runtime(native)
    _publish(state)
    _write_registration(state)
    _install_callbacks(state)
    if not state.atexit_registered:
        atexit.register(_atexit_stop)
        state.atexit_registered = True
    state.started = True
    state.stop_requested = False
    print(f"[Locus] Blender host ready ({state.mode}) on {state.instance['pipe']}")
    return True


def stop(reason="stop"):
    state = persist.state()
    if not state.started:
        return
    state.stop_requested = True
    _remove_callbacks(state)
    if state.native is not None:
        try:
            state.native.set_managed_state(QUITTING, 0, None)
            state.native.shutdown()
        except Exception:
            pass
        state.broker_live = False
    if _runtime is not None:
        _runtime.shm_pool.clear()
    registry.remove((state.instance or {}).get("registration", ""))
    state.started = False
    print(f"[Locus] Blender host stopped ({reason})")


def _atexit_stop():
    stop("exit")


def _install_callbacks(state):
    bpy = _bpy()
    _remove_callbacks(state)
    handlers = bpy.app.handlers
    for list_name, function in (("load_post", _on_load_post), ("save_pre", _on_save_pre), ("save_post", _on_save_post)):
        getattr(handlers, list_name).append(function)
        state.handlers.append((list_name, function))
    exit_pre = getattr(handlers, "exit_pre", None)
    if exit_pre is not None:
        exit_pre.append(_on_exit)
        state.handlers.append(("exit_pre", _on_exit))
    if state.mode == "interactive":
        bpy.app.timers.register(_tick, first_interval=TICK_SECONDS, persistent=True)
        state.timer = _tick


def _remove_callbacks(state):
    bpy = _bpy()
    for list_name, function in state.handlers:
        handler_list = getattr(bpy.app.handlers, list_name, None)
        if handler_list is not None and function in handler_list:
            handler_list.remove(function)
    state.handlers = []
    if state.timer is not None:
        try:
            if bpy.app.timers.is_registered(state.timer):
                bpy.app.timers.unregister(state.timer)
        except Exception:
            pass
        state.timer = None


# ── request pump ───────────────────────────────────────────────────────────


def _tick():
    state = persist.state()
    native = state.native
    if native is None or state.stop_requested:
        return None
    native.heartbeat(state.generation)
    deadline = time.monotonic() + TICK_BUDGET_SECONDS
    while time.monotonic() < deadline:
        line = native.poll_request()
        if line is None:
            break
        dispatch(line)
    return TICK_SECONDS


def serve_forever():
    """Background mode: block on the broker until `shutdown_host`."""
    state = persist.state()
    while state.started and not state.stop_requested:
        native = state.native
        native.heartbeat(state.generation)
        if native.wait_request(WAIT_TIMEOUT_MS) == 1:
            while True:
                line = native.poll_request()
                if line is None:
                    break
                dispatch(line)
                native.heartbeat(state.generation)
        if _runtime is not None:
            _runtime.executor.collect_batches()
            _runtime.shm_pool.collect()


def _complete(request_id, ok, message=None, error=None):
    envelope = {"reply_to": request_id, "type": "response", "ok": ok}
    if message is not None:
        envelope["message"] = message if isinstance(message, str) else json.dumps(message, ensure_ascii=False)
    if error is not None:
        envelope["error"] = error
    persist.state().native.complete(request_id, json.dumps(envelope, ensure_ascii=False))


def dispatch(line):
    state = persist.state()
    try:
        request = json.loads(line.decode("utf-8", "replace"))
    except ValueError:
        return
    request_id = str(request.get("id") or "")
    kind = request.get("type") or ""
    message = request.get("message") or ""
    if not request_id:
        return
    try:
        if kind == "exec":
            payload = json.loads(message) if message else {}
            result = _runtime.executor.execute(request_id, payload)
            _complete(request_id, True, result)
            _publish(state)
        elif kind == "end_batch":
            _complete(request_id, True, {"undo": _runtime.executor.end_batch(message)})
        elif kind == "reset_namespace":
            _runtime.executor.reset_namespace(message or None)
            _complete(request_id, True, {"reset": True})
        elif kind == "release_shm":
            names = json.loads(message) if message else []
            _runtime.shm_pool.release(names)
            _complete(request_id, True, {"released": len(names)})
        elif kind == "describe":
            _complete(request_id, True, {
                "instance": instance_info(),
                "blender": blender_info(),
                "python": sys.version.split()[0],
                "host_hash": host_hash(),
                "protocol": version.HOST_PROTOCOL,
                "capabilities": list(version.HOST_CAPABILITIES),
                "helpers": MANIFEST,
                "file": file_state(),
            })
        elif kind == "reload_host":
            _complete(request_id, True, {"reloading": True})
            _schedule(reload_host)
        elif kind == "shutdown_host":
            _complete(request_id, True, {"stopping": True})
            if state.mode == "background":
                state.stop_requested = True
            else:
                _schedule(lambda: stop("shutdown_host"))
        elif kind == "cancel":
            _complete(request_id, True, {"cancelled": True})
        elif kind == "exec_progress":
            _complete(request_id, True, {"active": False, "servedBy": "host"})
        else:
            _complete(request_id, False, error=f"unknown_request: {kind}")
    except Exception as error:  # noqa: BLE001 - every request gets an answer
        try:
            _complete(request_id, False, error=f"host_error: {type(error).__name__}: {error}")
        except Exception:
            pass


def _schedule(function):
    bpy = _bpy()
    if bpy.app.background:
        function()
        return

    def run_once():
        function()
        return None

    bpy.app.timers.register(run_once, first_interval=0.0)


# ── file events ────────────────────────────────────────────────────────────


def _emit_file_event(name, extra=None):
    state = persist.state()
    if state.native is None:
        return
    payload = {"event": name}
    payload.update(file_state())
    if extra:
        payload.update(extra)
    state.native.emit_event("blender-file", payload)


def _on_load_post(*_args):
    state = persist.state()
    if _runtime is not None:
        _runtime.executor.reset_namespace()
    _publish(state)
    _write_registration(state)
    _emit_file_event("load_post")


def _on_save_pre(*_args):
    bpy = _bpy()
    _emit_file_event("save_pre", {"writing_version": ".".join(str(v) for v in bpy.app.version)})


def _on_save_post(*_args):
    state = persist.state()
    _publish(state)
    _write_registration(state)
    _emit_file_event("save_post")


def _on_exit(*_args):
    stop("exit")


for _function in (_on_load_post, _on_save_pre, _on_save_post, _on_exit):
    try:
        import bpy as _bpy_module

        _bpy_module.app.handlers.persistent(_function)
    except Exception:
        pass


# ── reload ─────────────────────────────────────────────────────────────────


def reload_host():
    """Re-import the host code; keep the broker (and its pipe) running.

    When the installed DLL changed, the old broker shuts down and the new DLL
    takes over the same pipe name; Locus reconnects after a short gap.
    """
    state = persist.state()
    if state.native is not None:
        state.native.set_managed_state(RELOADING, state.generation, _status_string())
    state.instance = dict(state.instance or {}, state="reloading")
    _write_registration(state)
    _remove_callbacks(state)
    # Re-import from the copy this host was loaded from (the installed add-on
    # or the bootstrap sources). `--python bootstrap.py` puts the Locus
    # resources first on sys.path, which would otherwise replace an updated
    # add-on in `startup/` with the bootstrap copy.
    parent = os.path.dirname(_host_dir())
    sys.path[:] = [entry for entry in sys.path if os.path.normcase(os.path.abspath(entry or ".")) != os.path.normcase(parent)]
    sys.path.insert(0, parent)
    for name in [m for m in sys.modules if m == "locus_host" or m.startswith("locus_host.")]:
        del sys.modules[name]
    import importlib

    module = importlib.import_module("locus_host.host")
    module.rebind()


def rebind():
    """Entry point of freshly imported code after `reload_host`."""
    global _runtime
    state = persist.state()
    try:
        dll_path, _ = native_mod.resolve_dll()
        if state.native is None or os.path.normcase(dll_path) != os.path.normcase(state.native.path):
            if state.native is not None:
                state.native.shutdown()
            native = native_mod.Native(dll_path)
            instance = state.instance or {}
            if native.init_host(instance["key"], instance["pipe"], "", version.NATIVE_PROTOCOL) != 0:
                raise native_mod.NativeLoadError("locus_init_host failed after DLL update")
            native.set_auth_token(instance["token"])
            native.set_capabilities(version.HOST_CAPABILITIES)
            state.native = native
            state.broker_live = True
    except (native_mod.NativeLoadError, KeyError) as error:
        state.instance = dict(state.instance or {}, state="error")
        _write_registration(state, {"error": f"reload failed: {error}"})
        return
    state.generation += 1
    state.instance = dict(state.instance or {}, state="ready")
    _runtime = Runtime(state.native)
    _install_callbacks(state)
    _publish(state)
    _write_registration(state)
    state.native.set_capabilities(version.HOST_CAPABILITIES)
    print("[Locus] Blender host reloaded")


def current_runtime():
    return _runtime


__all__ = ["start", "stop", "serve_forever", "dispatch", "reload_host", "rebind", "current_runtime", "codec"]
