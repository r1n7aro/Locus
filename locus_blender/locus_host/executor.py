"""Runs `exec` requests on Blender's main thread.

Code runs inside `async def` (so it may `return` and `await`) compiled from
the parsed source, which keeps the user's line numbers; the source is
registered with `linecache` so tracebacks show it. Each execution uses a new
namespace unless the request asks for the per-session one. Output is teed to
the console and streamed to Locus; a monitor thread publishes progress to
the broker (readable while the code holds the main thread) and turns a
cancel signal into an exception on the main thread.
"""

import ast
import asyncio
import builtins
import ctypes
import importlib
import inspect
import io
import json
import linecache
import os
import sys
import threading
import time
import traceback

from . import codec, persist
from .helpers import Helpers
from .images import Image

OUTPUT_FLUSH_SECONDS = 0.05
OUTPUT_FLUSH_BYTES = 4096
OUTPUT_KEEP_BYTES = 512 * 1024
PROGRESS_SECONDS = 0.25
CANCEL_POLL_SECONDS = 0.1
MAX_ATTACHED_IMAGES = 4
BATCH_TTL_SECONDS = 600


class ExecutionCancelled(BaseException):
    """Raised on the main thread when Locus cancels the execution."""


class FunctionNotCached(Exception):
    pass


class _Tee(io.TextIOBase):
    """Writes through to the original stream and keeps/streams a copy."""

    def __init__(self, name, original, sink):
        super().__init__()
        self.name = name
        self.original = original
        self.sink = sink

    def writable(self):
        return True

    def write(self, text):
        if not isinstance(text, str):
            text = str(text)
        try:
            if self.original is not None:
                self.original.write(text)
        except Exception:
            pass
        self.sink.add(self.name, text)
        return len(text)

    def flush(self):
        try:
            if self.original is not None:
                self.original.flush()
        except Exception:
            pass


class _OutputSink:
    def __init__(self, emit):
        self._emit = emit
        self._lock = threading.Lock()
        self._pending = []
        self._pending_bytes = 0
        self._kept = {"stdout": [], "stderr": []}
        self._kept_bytes = {"stdout": 0, "stderr": 0}
        self.truncated = False
        self.total_bytes = 0

    def add(self, stream, text):
        size = len(text.encode("utf-8", "replace"))
        flush_now = False
        with self._lock:
            self.total_bytes += size
            if self._kept_bytes[stream] + size <= OUTPUT_KEEP_BYTES:
                self._kept[stream].append(text)
                self._kept_bytes[stream] += size
            else:
                self.truncated = True
            self._pending.append((stream, text))
            self._pending_bytes += size
            flush_now = self._pending_bytes >= OUTPUT_FLUSH_BYTES
        if flush_now:
            self.flush()

    def flush(self):
        with self._lock:
            pending, self._pending, self._pending_bytes = self._pending, [], 0
        if not pending or self._emit is None:
            return
        merged = []
        for stream, text in pending:
            if merged and merged[-1][0] == stream:
                merged[-1] = (stream, merged[-1][1] + text)
            else:
                merged.append((stream, text))
        for stream, text in merged:
            self._emit(stream, text)

    def text(self, stream):
        with self._lock:
            return "".join(self._kept[stream])


class _Monitor(threading.Thread):
    """Progress, output flushing and cancellation for one execution."""

    def __init__(self, native, execution_id, label, sink, main_thread_id):
        super().__init__(name="locus-blender-exec-monitor", daemon=True)
        self.native = native
        self.execution_id = execution_id
        self.label = label
        self.sink = sink
        self.main_thread_id = main_thread_id
        self.done = threading.Event()
        self.started_at = time.monotonic()
        self.cancelled = False
        self.since = native.execution_cancel_seq() if native else 0

    def run(self):
        last_progress = 0.0
        last_cancel = 0.0
        while not self.done.wait(OUTPUT_FLUSH_SECONDS):
            self.sink.flush()
            now = time.monotonic()
            if self.native is None:
                continue
            if now - last_cancel >= CANCEL_POLL_SECONDS:
                last_cancel = now
                if not self.cancelled and self.native.execution_cancel_requested(self.execution_id, self.since):
                    self.cancelled = True
                    ctypes.pythonapi.PyThreadState_SetAsyncExc(
                        ctypes.c_ulong(self.main_thread_id), ctypes.py_object(ExecutionCancelled)
                    )
            if now - last_progress >= PROGRESS_SECONDS:
                last_progress = now
                try:
                    self.native.execution_publish(self.execution_id, {
                        "active": True,
                        "title": self.label,
                        "elapsedMs": int((now - self.started_at) * 1000),
                        "outputBytes": self.sink.total_bytes,
                        "stopRequested": self.cancelled,
                    })
                except Exception:
                    pass

    def stop(self):
        self.done.set()
        self.join(timeout=2)
        self.sink.flush()
        if self.native is not None:
            try:
                self.native.execution_end(self.execution_id)
            except Exception:
                pass


def _empty_arguments():
    return ast.arguments(posonlyargs=[], args=[], vararg=None, kwonlyargs=[], kw_defaults=[], kwarg=None, defaults=[])


class _TopLevelNames(ast.NodeVisitor):
    """Names the code binds at its top level (not inside nested scopes)."""

    def __init__(self):
        self.names = set()

    def _bind(self, name):
        if name:
            self.names.add(name)

    def visit_Name(self, node):
        if isinstance(node.ctx, (ast.Store, ast.Del)):
            self._bind(node.id)

    def visit_Import(self, node):
        for alias in node.names:
            self._bind(alias.asname or alias.name.split(".")[0])

    def visit_ImportFrom(self, node):
        for alias in node.names:
            if alias.name != "*":
                self._bind(alias.asname or alias.name)

    def _scope(self, node):
        self._bind(node.name)
        for decorator in node.decorator_list:
            self.visit(decorator)

    visit_FunctionDef = visit_AsyncFunctionDef = _scope

    def visit_ClassDef(self, node):
        self._scope(node)
        for base in node.bases:
            self.visit(base)

    def visit_Lambda(self, node):
        pass

    def _comprehension(self, node):
        pass

    visit_ListComp = visit_SetComp = visit_DictComp = visit_GeneratorExp = _comprehension

    def visit_ExceptHandler(self, node):
        self._bind(node.name)
        self.generic_visit(node)

    def visit_MatchAs(self, node):
        self._bind(node.name)
        self.generic_visit(node)

    def visit_MatchStar(self, node):
        self._bind(node.name)

    def visit_MatchMapping(self, node):
        self._bind(node.rest)
        self.generic_visit(node)

    def visit_Global(self, node):
        for name in node.names:
            self._bind(name)

    def visit_Nonlocal(self, node):
        raise SyntaxError("nonlocal is not allowed at the top level of executed code")


def compile_code(source, filename):
    """Compile `source` as the body of `async def __locus_blender_main__`.

    Every name the code binds at its top level is declared `global`, so the
    code reads and writes the namespace exactly like module-level code (a
    persistent namespace keeps its variables), while `return` and `await`
    work at the top level. Line numbers stay those of the source.
    """
    tree = ast.parse(source, filename=filename, mode="exec")
    collector = _TopLevelNames()
    for statement in tree.body:
        collector.visit(statement)
    body = tree.body or [ast.Pass(lineno=1, col_offset=0)]
    if collector.names:
        body = [ast.Global(names=sorted(collector.names), lineno=1, col_offset=0)] + body
    extra = {"type_params": []} if sys.version_info >= (3, 12) else {}
    function = ast.AsyncFunctionDef(
        name="__locus_blender_main__",
        args=_empty_arguments(),
        body=body,
        decorator_list=[],
        returns=None,
        type_comment=None,
        lineno=1,
        col_offset=0,
        **extra,
    )
    module = ast.Module(body=[function], type_ignores=[])
    ast.fix_missing_locations(module)
    return compile(module, filename, "exec")


def register_source(filename, source):
    linecache.cache[filename] = (len(source), None, source.splitlines(True), filename)


def run_coroutine(coroutine):
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(coroutine)
    finally:
        try:
            loop.run_until_complete(loop.shutdown_asyncgens())
        except Exception:
            pass
        loop.close()
        asyncio.set_event_loop(None)


def _format_exception(error):
    frames = traceback.extract_tb(error.__traceback__)
    # Drop host frames above the user's code.
    user = [frame for frame in frames if frame.filename.startswith("<locus-blender") or not frame.filename.startswith(os.path.dirname(__file__))]
    text = "".join(traceback.format_list(user or frames)) + "".join(traceback.format_exception_only(type(error), error))
    return {
        "type": type(error).__name__,
        "module": type(error).__module__,
        "message": str(error),
        "traceback": "Traceback (most recent call last):\n" + text,
    }


def push_undo(message):
    import bpy

    if bpy.app.background:
        return "skipped"
    window_manager = bpy.context.window_manager
    windows = list(window_manager.windows) if window_manager is not None else []
    try:
        if windows:
            with bpy.context.temp_override(window=windows[0]):
                bpy.ops.ed.undo_push(message=message)
        else:
            bpy.ops.ed.undo_push(message=message)
        return "pushed"
    except Exception as error:
        return f"failed: {error}"


class Executor:
    def __init__(self, native, shm_pool, instance_info):
        self.native = native
        self.shm_pool = shm_pool
        self.instance_info = instance_info
        self.sessions = {}
        self.functions = {}
        self.batches = {}  # batch id -> {"label", "dirty", "created"}

    # ── namespaces ─────────────────────────────────────────────────────────
    def reset_namespace(self, session=None):
        if session:
            self.sessions.pop(session, None)
        else:
            self.sessions.clear()

    def _base_names(self, helpers, context):
        import bmesh
        import bpy
        import mathutils

        names = {
            "__builtins__": builtins,
            "bpy": bpy,
            "mathutils": mathutils,
            "bmesh": bmesh,
            "blender": helpers,
            "project": (context or {}).get("workspace"),
        }
        locus_module = _import_locus(context)
        if locus_module is not None:
            names["locus"] = locus_module
        return names

    def _namespace(self, payload, helpers, context):
        names = self._base_names(helpers, context)
        if payload.get("namespace") == "session":
            key = str(payload.get("session") or (context or {}).get("session_id") or "default")
            namespace = self.sessions.setdefault(key, {"__name__": "__locus_blender__"})
            namespace.update(names)
            return namespace, True
        namespace = {"__name__": "__locus_blender__"}
        namespace.update(names)
        return namespace, False

    # ── batches ────────────────────────────────────────────────────────────
    def end_batch(self, batch_id):
        batch = self.batches.pop(batch_id, None)
        if batch and batch.get("dirty"):
            return push_undo(batch.get("label") or "Locus")
        return "skipped"

    def collect_batches(self):
        now = time.monotonic()
        for batch_id in [b for b, info in self.batches.items() if now - info["created"] > BATCH_TTL_SECONDS]:
            self.end_batch(batch_id)

    # ── execution ──────────────────────────────────────────────────────────
    def execute(self, request_id, payload):
        started = time.monotonic()
        context = payload.get("context") or {}
        readonly = bool(payload.get("readonly"))
        lenient = bool(payload.get("lenient"))
        label = str(payload.get("label") or "Locus")[:64]
        attached = []

        def attach(image):
            if len(attached) >= MAX_ATTACHED_IMAGES:
                raise RuntimeError(f"at most {MAX_ATTACHED_IMAGES} images per call")
            attached.append(image)

        helpers = Helpers(readonly=readonly, attach=attach)
        sink = _OutputSink(lambda stream, text: self._emit_output(request_id, stream, text))
        monitor = _Monitor(self.native, request_id, label, sink, threading.get_ident())
        decoder = codec.Decoder()
        state = persist.state()
        previous_attach = getattr(state, "attach_image", None)
        state.attach_image = helpers.attach_image
        saved_streams = (sys.stdout, sys.stderr)
        saved_env = _apply_sdk_env(context)
        sys.stdout = _Tee("stdout", saved_streams[0], sink)
        sys.stderr = _Tee("stderr", saved_streams[1], sink)
        if self.native is not None:
            self.native.managed_execution(True)
        monitor.start()
        result = {"ok": True}
        value = None
        try:
            namespace, persistent = self._namespace(payload, helpers, context)
            mode = payload.get("mode") or "code"
            if mode == "function":
                value = self._run_function(payload, namespace, decoder)
            else:
                if mode == "file":
                    source, filename = _read_script(payload, context)
                    namespace["__file__"] = filename
                else:
                    source = str(payload.get("code") or "")
                    filename = f"<locus-blender:{request_id}>"
                args = decoder.decode(payload.get("args") or {}, "args")
                if not isinstance(args, dict):
                    raise TypeError("args must be a dict of variable names to values")
                for name, item in args.items():
                    if not str(name).isidentifier():
                        raise ValueError(f"args key {name!r} is not a valid variable name")
                    namespace[str(name)] = item
                value = self._run_code(source, filename, namespace, persistent)
        except ExecutionCancelled:
            result = {"ok": False, "cancelled": True, "error": {
                "type": "Cancelled", "module": "locus", "message": "execution cancelled by Locus", "traceback": "",
            }}
        except FunctionNotCached as error:
            result = {"ok": False, "error": {"type": "FunctionNotCached", "module": "locus", "message": str(error), "traceback": ""}}
        except BaseException as error:  # noqa: BLE001 - user code must never end Blender
            result = {"ok": False, "error": _format_exception(error)}
        finally:
            sys.stdout, sys.stderr = saved_streams
            _restore_env(saved_env)
            state.attach_image = previous_attach
            if self.native is not None:
                self.native.managed_execution(False)
            monitor.stop()
            if decoder.released and self.native is not None:
                self.native.emit_event("blender-release-shm", {"names": decoder.released})

        encoder = codec.Encoder(
            lenient=lenient,
            shm_pool=self.shm_pool,
            call_id=request_id,
            inline_limit=int(payload.get("inline_limit") or codec.INLINE_LIMIT),
        )
        if result["ok"]:
            try:
                result["value"] = encoder.encode(value)
                result["value_repr"] = _value_repr(value)
            except codec.CodecError as error:
                result = {"ok": False, "error": {"type": "ReturnValueError", "module": "locus", "message": str(error), "traceback": ""}}
        try:
            result["attachments"] = [encoder.encode(image, "attachments") for image in attached]
        except codec.CodecError as error:
            result.setdefault("warnings", []).append(str(error))
        result["warnings"] = result.get("warnings", []) + encoder.warnings
        if sink.truncated:
            result["warnings"].append(f"output above {OUTPUT_KEEP_BYTES // 1024} KiB per stream was streamed but not kept")
        result["stdout"] = sink.text("stdout")
        result["stderr"] = sink.text("stderr")
        result["shm"] = encoder.shm_names
        result["duration_ms"] = int((time.monotonic() - started) * 1000)
        result["instance"] = self.instance_info()
        result["undo"] = self._after_execution(payload, readonly, label, result)
        return result

    def _after_execution(self, payload, readonly, label, result):
        if readonly:
            return "readonly"
        batch_id = payload.get("batch")
        if batch_id:
            batch = self.batches.setdefault(batch_id, {"label": label, "dirty": False, "created": time.monotonic()})
            batch["dirty"] = True
            return "batched"
        return push_undo(label)

    def _run_code(self, source, filename, namespace, persistent):
        register_source(filename, source)
        try:
            code = compile_code(source, filename)
        except SyntaxError as error:
            if "import *" not in str(error):
                raise
            # `from x import *` is only valid at module level: run it there
            # (no top-level `return` or `await` in that case).
            exec(compile(source, filename, "exec"), namespace)
            return None
        exec(code, namespace)
        coroutine = namespace.pop("__locus_blender_main__")()
        return run_coroutine(coroutine)

    def _run_function(self, payload, namespace, decoder):
        spec = payload.get("function") or {}
        name = str(spec.get("name") or "")
        digest = str(spec.get("hash") or "")
        source = spec.get("source")
        if source:
            self.functions[digest] = source
        else:
            source = self.functions.get(digest)
            if source is None:
                raise FunctionNotCached(f"function {name!r} ({digest}) is not cached on this instance")
        for alias, module_name in (spec.get("modules") or {}).items():
            try:
                namespace[alias] = importlib.import_module(module_name)
            except ImportError as error:
                raise ImportError(f"module {module_name!r} (used as {alias!r}) is not available in this Blender: {error}")
        for dependency in spec.get("dependencies") or ():
            dep_source = dependency.get("source") or self.functions.get(dependency.get("hash", ""))
            if dep_source is None:
                raise FunctionNotCached(f"function {dependency.get('name')!r} is not cached on this instance")
            self.functions[dependency.get("hash", "")] = dep_source
            dep_file = f"<locus-blender:fn:{dependency.get('name')}>"
            register_source(dep_file, dep_source)
            exec(compile(dep_source, dep_file, "exec"), namespace)
        filename = f"<locus-blender:fn:{name}>"
        register_source(filename, source)
        exec(compile(source, filename, "exec"), namespace)
        function = namespace.get(name)
        if not callable(function):
            raise NameError(f"function {name!r} was not defined by its source")
        args = decoder.decode(spec.get("args") or [], "args")
        kwargs = decoder.decode(spec.get("kwargs") or {}, "kwargs")
        value = function(*args, **kwargs)
        if inspect.isawaitable(value):
            raise TypeError("@bl.function only supports plain def functions")
        return value

    def _emit_output(self, request_id, stream, text):
        if self.native is None:
            return
        try:
            self.native.emit_event("blender-output", {"executionId": request_id, "stream": stream, "text": text})
        except Exception:
            pass


def _value_repr(value):
    if isinstance(value, Image):
        return repr(value)
    if isinstance(value, (list, tuple)) and value and all(isinstance(item, Image) for item in value):
        return f"[{', '.join(repr(item) for item in value)}]"
    return codec.short_repr(value)


def _read_script(payload, context):
    path = str(payload.get("path") or "")
    if not path:
        raise ValueError("execute_file needs a path")
    if not os.path.isabs(path):
        base = (context or {}).get("workspace") or os.getcwd()
        path = os.path.join(base, path)
    path = os.path.abspath(path)
    with open(path, "r", encoding="utf-8") as handle:
        return handle.read(), path


_SDK_ENV = ("LOCUS_SDK_URL", "LOCUS_SDK_TOKEN", "LOCUS_SESSION_ID", "LOCUS_PROJECT", "LOCUS_CHECKOUT_ID",
            "LOCUS_BLENDER_INSTANCE")


def _apply_sdk_env(context):
    """Per-execution SDK context (executions are serialized on this thread)."""
    saved = {key: os.environ.get(key) for key in _SDK_ENV + ("LOCUS_BLENDER_EXECUTION",)}
    mapping = {
        "LOCUS_SDK_URL": context.get("sdk_url"),
        "LOCUS_SDK_TOKEN": context.get("sdk_token"),
        "LOCUS_SESSION_ID": context.get("session_id"),
        "LOCUS_PROJECT": context.get("workspace"),
        "LOCUS_CHECKOUT_ID": context.get("checkout_id"),
        # Lets `locus.blender` refuse calls into this same instance (they
        # would wait on the main thread they are running on).
        "LOCUS_BLENDER_INSTANCE": (persist.state().instance or {}).get("key"),
    }
    for key, value in mapping.items():
        if value:
            os.environ[key] = str(value)
    os.environ["LOCUS_BLENDER_EXECUTION"] = "1"
    return saved


def _restore_env(saved):
    for key, value in saved.items():
        if value is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = value


def _import_locus(context):
    sdk_dir = (context or {}).get("sdk_dir")
    if sdk_dir and os.path.isdir(sdk_dir) and sdk_dir not in sys.path:
        sys.path.append(sdk_dir)
    try:
        return importlib.import_module("locus")
    except Exception:
        return None


def encode_response(request_id, result):
    """Full envelope line for `locus_complete_request`."""
    return json.dumps(
        {"reply_to": request_id, "type": "response", "ok": True, "message": json.dumps(result, ensure_ascii=False)},
        ensure_ascii=False,
    )
