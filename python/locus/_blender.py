"""`locus.blender`: drive running Blender instances from Locus Python.

Code runs inside Blender on its main thread (bpy is only available there):

* ``await locus.blender.execute(code, args=...)`` runs a code string,
* ``@locus.blender.function`` ships a plain ``def`` and calls it remotely,
* ``await locus.blender.execute_file(path)`` runs a workspace script,
* helper proxies (``look``, ``scene_summary``, ...) call ``blender.<helper>``.

See the SDK topic ``blender.md`` for targets, value rules and errors.
"""
from __future__ import annotations

import functools
import hashlib
import inspect
import os
import sys
import textwrap
import types
from dataclasses import dataclass, field
from typing import Any, Callable

from ._blender_codec import Array, CodecError, Decoder, Encoder, Ref
from ._client import Client, LocusSdkError
from ._image import Image
from ._scope import workspace_payload

__all__ = [
    "Blender", "BlenderError", "BlenderFunction", "Instance", "NotExecuted", "Result", "TargetUnavailable",
    "TimedOut", "Ref", "Array", "CodecError",
]

_DEFAULT_TIMEOUT = 120.0
_MAX_TIMEOUT = 1800.0
#: Extra RPC budget for launching a background instance on first use.
_LAUNCH_ALLOWANCE = 120.0
_PREINJECTED = frozenset({"bpy", "mathutils", "bmesh", "blender", "locus", "project"})
#: Helpers proxied from the Blender host manifest (name, readonly).
_HELPERS: dict[str, bool] = {
    "look": True,
    "scene_summary": True,
    "object_detail": True,
    "file_report": True,
    "api": True,
    "focus": False,
    "render_thumbnail": False,
}


class TargetUnavailable(LocusSdkError):
    """No Blender instance matches; ``candidates`` lists the running ones."""

    def __init__(self, message: str, candidates: list["Instance"]) -> None:
        super().__init__(message)
        self.candidates = candidates


class NotExecuted(LocusSdkError):
    """Blender never started the code; retrying is safe."""


class TimedOut(LocusSdkError):
    """The code started but did not finish; the scene state is uncertain."""


class BlenderError(LocusSdkError):
    """An exception raised by the code inside Blender."""

    def __init__(self, type_name: str, message: str, traceback: str, instance: "Instance | None") -> None:
        super().__init__(f"{type_name}: {message}" + (f"\n{traceback}" if traceback else ""))
        self.type = type_name
        self.message = message
        self.traceback = traceback
        self.instance = instance


@dataclass(frozen=True)
class Instance:
    """A running Blender with the Locus host."""

    key: str
    pid: int
    mode: str
    version: str
    version_string: str
    file: str | None
    dirty: bool
    connection: str
    in_workspace: bool = False
    is_target: bool = False
    owned_background: bool = False
    shared_file: bool = False
    busy: bool = False
    host_hash: str = ""
    raw: dict[str, Any] = field(default_factory=dict, repr=False, compare=False)

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "Instance":
        return cls(
            key=str(payload.get("key") or ""),
            pid=int(payload.get("pid") or 0),
            mode=str(payload.get("mode") or ""),
            version=str(payload.get("version") or ""),
            version_string=str(payload.get("versionString") or payload.get("version") or ""),
            file=payload.get("file"),
            dirty=bool(payload.get("dirty")),
            connection=str(payload.get("connection") or ""),
            in_workspace=bool(payload.get("inWorkspace")),
            is_target=bool(payload.get("isTarget")),
            owned_background=bool(payload.get("ownedBackground")),
            shared_file=bool(payload.get("sharedFile")),
            busy=bool(payload.get("busy")),
            host_hash=str(payload.get("hostHash") or ""),
            raw=dict(payload),
        )

    def __str__(self) -> str:
        return self.key


@dataclass
class Result:
    """Outcome of ``execute``/``execute_file``."""

    instance: Instance
    version: str
    stdout: str
    stderr: str
    value: Any
    warnings: list[str]
    waited_ms: int
    duration_ms: int
    undo: str | None = None
    images: list[Image] = field(default_factory=list)


def _target_text(target: Any) -> str | None:
    if target is None:
        return None
    if isinstance(target, Instance):
        return target.key
    if isinstance(target, int) and not isinstance(target, bool):
        return str(target)
    if isinstance(target, os.PathLike):
        return os.fspath(target)
    text = str(target).strip()
    return text or None


def _timeout_ms(timeout: float | None) -> int:
    value = _DEFAULT_TIMEOUT if timeout is None else float(timeout)
    if value <= 0:
        raise ValueError("timeout must be positive")
    return int(min(value, _MAX_TIMEOUT) * 1000)


class BlenderFunction:
    """A plain ``def`` that runs inside Blender when called (``await f(...)``).

    Pre-injected names (bpy, mathutils, bmesh, blender) need no import.
    Modules the function uses are imported inside Blender under the same
    name; other ``@function`` functions it calls are sent along. Any other
    outer variable is rejected: pass it as an argument.
    """

    def __init__(self, fn: Callable[..., Any], blender: "Blender", *, target: Any = None,
                 readonly: bool = False) -> None:
        if not isinstance(fn, types.FunctionType) or fn.__name__ == "<lambda>":
            raise TypeError("@locus.blender.function needs a plain def function")
        if inspect.iscoroutinefunction(fn) or inspect.isasyncgenfunction(fn) or inspect.isgeneratorfunction(fn):
            raise TypeError("@locus.blender.function only supports plain def functions "
                            "(no async def, generators or lambdas)")
        self._fn = fn
        self._blender = blender
        self._target = target
        self._readonly = readonly
        self._spec_cache: dict[str, Any] | None = None
        functools.update_wrapper(self, fn)

    def on(self, target: Any) -> "BlenderFunction":
        """The same function bound to another instance: ``await f.on(inst)(...)``."""
        bound = BlenderFunction(self._fn, self._blender, target=target, readonly=self._readonly)
        bound._spec_cache = self._spec_cache
        return bound

    def __repr__(self) -> str:
        return f"<blender function {self._fn.__qualname__}>"

    # ── source and references ──────────────────────────────────────────
    def _own_source(self) -> str:
        try:
            lines = inspect.getsource(self._fn)
        except (OSError, TypeError) as error:
            raise TypeError(f"cannot read the source of {self._fn.__name__}: {error}") from error
        source = textwrap.dedent(lines)
        kept = source.splitlines(True)
        while kept and kept[0].lstrip().startswith("@"):
            # Decorator lines (possibly spanning several lines) end where the
            # `def` begins.
            kept.pop(0)
            while kept and not kept[0].lstrip().startswith(("def ", "@")):
                kept.pop(0)
        return "".join(kept)

    def _spec(self) -> dict[str, Any]:
        if self._spec_cache is not None:
            return self._spec_cache
        source = self._own_source()
        closure = inspect.getclosurevars(self._fn)
        refs = {**closure.globals, **closure.nonlocals}
        modules: dict[str, str] = {}
        dependencies: dict[str, dict[str, Any]] = {}
        rejected = []
        for name, value in refs.items():
            if name in _PREINJECTED:
                continue
            if isinstance(value, types.ModuleType):
                modules[name] = value.__name__
            elif isinstance(value, BlenderFunction):
                spec = value._spec()
                modules.update(spec["modules"])
                for dep in spec["dependencies"]:
                    dependencies.setdefault(dep["name"], dep)
                dependencies.setdefault(value._fn.__name__, {
                    "name": value._fn.__name__, "hash": spec["hash"], "source": spec["source"],
                })
            elif value is self:
                continue
            else:
                rejected.append(name)
        if rejected:
            raise TypeError(
                f"@locus.blender.function {self._fn.__name__} uses outer variables {sorted(rejected)}; "
                "pass them as parameters (only modules and other @function functions are sent along)"
            )
        digest = hashlib.sha256(source.encode("utf-8"))
        for dep in sorted(dependencies.values(), key=lambda d: d["name"]):
            digest.update(dep["hash"].encode("ascii"))
        self._spec_cache = {
            "name": self._fn.__name__,
            "source": source,
            "hash": digest.hexdigest()[:32],
            "modules": modules,
            "dependencies": list(dependencies.values()),
        }
        return self._spec_cache

    # ── call ───────────────────────────────────────────────────────────────
    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        return self._call(args, kwargs)

    async def _call(self, args: tuple[Any, ...], kwargs: dict[str, Any]) -> Any:
        spec = self._spec()
        signature = inspect.signature(self._fn)
        bound = signature.bind(*args, **kwargs)
        bound.apply_defaults()
        positional: list[Any] = []
        keywords: dict[str, Any] = {}
        for name, parameter in signature.parameters.items():
            value = bound.arguments[name]
            if parameter.kind in (parameter.POSITIONAL_ONLY, parameter.POSITIONAL_OR_KEYWORD):
                positional.append(value)
            elif parameter.kind is parameter.VAR_POSITIONAL:
                positional.extend(value)
            elif parameter.kind is parameter.KEYWORD_ONLY:
                keywords[name] = value
            else:
                keywords.update(value)
        blender = self._blender
        target = _target_text(self._target)
        known = blender._instance_for_target.get(target)
        include_source = known is None or spec["hash"] not in blender._sent.get(known, set())
        for attempt in range(2):
            result = await blender._run(
                {"mode": "function", "readonly": self._readonly,
                 "function": {
                     "name": spec["name"], "hash": spec["hash"],
                     "source": spec["source"] if include_source else None,
                     "modules": spec["modules"],
                     "dependencies": [
                         {"name": d["name"], "hash": d["hash"], "source": d["source"] if include_source else None}
                         for d in spec["dependencies"]
                     ],
                     "args": positional, "kwargs": keywords,
                 }},
                target=self._target, timeout=None, encode_keys=("function.args", "function.kwargs"),
                label=f"Locus: {spec['name']}", raise_errors=False,
            )
            error = result.get("error") or {}
            if not result.get("ok") and error.get("type") == "FunctionNotCached" and attempt == 0:
                include_source = True
                continue
            break
        instance_key = result["instance"].key
        blender._instance_for_target[target] = instance_key
        if result.get("ok") or error.get("type") != "FunctionNotCached":
            blender._sent.setdefault(instance_key, set()).update(
                [spec["hash"], *(d["hash"] for d in spec["dependencies"])]
            )
        return blender._finish(result).value


class Blender:
    """Entry point (``locus.blender``). Every method is a coroutine except
    ``function``; all accept ``workspace_ref=``/``worktree=`` like other
    workspace APIs and default to the current checkout and session."""

    def __init__(self, client: Client | None = None, *, workspace_ref: Any = None, worktree: Any = None) -> None:
        self._client = client
        self._workspace_ref = workspace_ref
        self._worktree = worktree
        self._sent: dict[str, set[str]] = {}
        self._instance_for_target: dict[str | None, str] = {}
        self._seen_instances: set[str] = set()

    # ── plumbing ───────────────────────────────────────────────────────────
    @property
    def client(self) -> Client:
        if self._client is None:
            self._client = Client()
        return self._client

    def scoped(self, *, workspace_ref: Any = None, worktree: Any = None) -> "Blender":
        """The same API for another checkout (e.g. a worktree handle)."""
        return Blender(self._client, workspace_ref=workspace_ref, worktree=worktree)

    def _base(self) -> dict[str, Any]:
        return {
            "workspaceRef": workspace_payload(self._workspace_ref, self._worktree),
            "sessionId": self.client.current_session_id,
            "pythonCall": os.environ.get("LOCUS_PYTHON_CALL") or None,
        }

    async def _rpc(self, action: str, params: dict[str, Any] | None = None, *, timeout: float = 60.0) -> Any:
        payload = self._base()
        payload.update({key: value for key, value in (params or {}).items() if value is not None})
        return await self.client.rpc(f"blender.{action}", payload, timeout=timeout)

    async def _run(self, payload: dict[str, Any], *, target: Any, timeout: float | None,
                   encode_keys: tuple[str, ...], label: str, raise_errors: bool = True,
                   namespace: str = "fresh") -> Any:
        timeout_ms = _timeout_ms(timeout)
        encoder = Encoder(call_id=os.urandom(4).hex())
        try:
            for dotted in encode_keys:
                container: dict[str, Any] = payload
                *parents, leaf = dotted.split(".")
                for key in parents:
                    container = container[key]
                container[leaf] = encoder.encode(container.get(leaf), leaf)
            payload.setdefault("label", label[:64])
            payload["namespace"] = namespace
            if os.environ.get("LOCUS_PYTHON_READONLY") == "1":
                payload["readonly"] = True
            body = await self._rpc("execute", {
                "target": _target_text(target),
                "payload": payload,
                "timeoutMs": timeout_ms,
                "callerInstance": os.environ.get("LOCUS_BLENDER_INSTANCE") or None,
            }, timeout=timeout_ms / 1000 + _LAUNCH_ALLOWANCE)
        finally:
            encoder.close()
        if not body.get("ok"):
            self._raise_failure(body)
        instance = Instance.from_payload(body["instance"])
        result = dict(body["result"])
        result["instance"] = instance
        result["warnings"] = list(body.get("warnings") or []) + list(result.get("warnings") or [])
        result["waited_ms"] = int(body.get("waitedMs") or 0)
        if raise_errors and not result.get("ok"):
            self._finish(result)
        return result

    def _raise_failure(self, body: dict[str, Any]) -> None:
        failure = body.get("failure") or {}
        kind = failure.get("kind") or body.get("code")
        message = str(body.get("message") or failure.get("message") or "Blender call failed")
        if kind == "target_unavailable":
            candidates = [Instance.from_payload(item) for item in failure.get("candidates") or []]
            raise TargetUnavailable(message, candidates)
        if kind == "not_executed":
            raise NotExecuted(message)
        if kind == "timed_out":
            raise TimedOut(message)
        raise LocusSdkError(message)

    def _finish(self, result: dict[str, Any]) -> Result:
        """Decode the value, release shared memory, print Blender's output and
        raise `BlenderError` for exceptions inside Blender."""
        instance: Instance = result["instance"]
        decoder = Decoder()
        value = decoder.decode(result.get("value")) if result.get("ok") else None
        images = [item for item in (decoder.decode(a) for a in result.get("attachments") or [])
                  if isinstance(item, Image)]
        names = list(decoder.released) + [n for n in result.get("shm") or [] if n not in decoder.released]
        if names:
            try:
                self.client._rpc_sync("blender.release_shm", {"instance": instance.key, "names": names}, 10.0)
            except Exception:
                pass  # the host drops unacknowledged memory after its TTL
        self._seen_instances.add(instance.key)
        prefix = f"[Blender {instance.pid}] " if len(self._seen_instances) > 1 else ""
        for stream, out in (("stdout", sys.stdout), ("stderr", sys.stderr)):
            text = result.get(stream) or ""
            if text:
                if prefix:
                    text = "".join(prefix + line for line in text.splitlines(True))
                out.write(text if text.endswith("\n") else text + "\n")
        for warning in result.get("warnings") or []:
            sys.stderr.write(f"[Blender] warning: {warning}\n")
        if images:
            # Images the code attached explicitly (`locus.attach_image` inside
            # Blender) join this tool call's result.
            from ._image import attach_image
            for image in images:
                try:
                    attach_image(image)
                except Exception as error:  # noqa: BLE001 - no tool call to attach to
                    sys.stderr.write(f"[Blender] image not attached: {error}\n")
        if not result.get("ok"):
            error = result.get("error") or {}
            raise BlenderError(str(error.get("type") or "Error"), str(error.get("message") or ""),
                               str(error.get("traceback") or ""), instance)
        return Result(
            instance=instance,
            version=instance.version_string,
            stdout=result.get("stdout") or "",
            stderr=result.get("stderr") or "",
            value=value,
            warnings=list(result.get("warnings") or []),
            waited_ms=int(result.get("waited_ms") or 0),
            duration_ms=int(result.get("duration_ms") or 0),
            undo=result.get("undo"),
            images=images,
        )

    # ── execution ──────────────────────────────────────────────────────────
    async def execute(self, code: str, target: Any = None, args: dict[str, Any] | None = None, *,
                      namespace: str = "fresh", readonly: bool = False, timeout: float | None = None) -> Result:
        """Run ``code`` as a function body inside Blender (``return`` gives
        ``Result.value``). ``args`` values become variables of the same name.
        Same semantics as the ``blender_execute`` tool; images in the value
        are not attached automatically (use ``locus.attach_image``)."""
        if namespace not in ("fresh", "session"):
            raise ValueError("namespace must be 'fresh' or 'session'")
        if args is not None and not isinstance(args, dict):
            raise TypeError("args must be a dict of variable names to values")
        payload = {"mode": "code", "code": str(code), "readonly": bool(readonly), "args": args or {}}
        result = await self._run(payload, target=target, timeout=timeout, encode_keys=("args",),
                                 label=f"Locus: {_first_line(code)}", namespace=namespace)
        return self._finish(result)

    async def execute_file(self, path: str | os.PathLike[str], target: Any = None,
                           args: dict[str, Any] | None = None, *, namespace: str = "fresh",
                           readonly: bool = False, timeout: float | None = None) -> Result:
        """Run a script file (relative paths resolve against the checkout)."""
        payload = {"mode": "file", "path": os.fspath(path), "readonly": bool(readonly), "args": args or {}}
        result = await self._run(payload, target=target, timeout=timeout, encode_keys=("args",),
                                 label=f"Locus: {os.path.basename(os.fspath(path))}", namespace=namespace)
        return self._finish(result)

    def function(self, fn: Callable[..., Any] | None = None, *, readonly: bool = False) -> Any:
        """Decorator: ``@locus.blender.function`` or ``@locus.blender.function(readonly=True)``."""
        if fn is None:
            return lambda inner: BlenderFunction(inner, self, readonly=readonly)
        return BlenderFunction(fn, self, readonly=readonly)

    async def import_module(self, path: str | os.PathLike[str], target: Any = None) -> "ModuleHandle":
        """Handle on a workspace module loaded inside Blender; each attribute
        call is one round trip: ``mod = await bl.import_module("tools/x.py");
        await mod.run(1)``."""
        handle = ModuleHandle(self, os.fspath(path), target)
        await handle._load()
        return handle

    async def reset_namespace(self, target: Any = None) -> None:
        """Clear this session's persistent namespace (``namespace="session"``)."""
        await self._rpc("reset_namespace", {"target": _target_text(target)})

    # ── instances and targets ──────────────────────────────────────────────
    async def instances(self, scope: str = "workspace") -> list[Instance]:
        """Running instances of this workspace (``scope="all"``: every one)."""
        if scope not in ("workspace", "all"):
            raise ValueError("scope must be 'workspace' or 'all'")
        body = await self._rpc("instances", {"scope": scope})
        return [Instance.from_payload(item) for item in body.get("instances") or []]

    async def target(self) -> str | None:
        return (await self._rpc("target.get")).get("target")

    async def set_target(self, instance: Any = None, *, pid: int | None = None, file: str | None = None,
                         background: bool = False, version: str | None = None) -> str:
        """Session target for calls without ``target=`` (memory only)."""
        body = await self._rpc("target.set", {
            "instance": _target_text(instance), "pid": pid, "file": file,
            "background": background or None, "version": version,
        })
        return body.get("target")

    async def clear_target(self) -> None:
        await self._rpc("target.clear")

    async def launch(self, file: str | None = None, *, installation: str | None = None,
                     version: str | None = None, allow_downgrade: bool = False) -> Instance:
        """Start an interactive Blender (it stays open after Locus exits)."""
        body = await self._rpc("launch", {"file": file, "installation": installation, "version": version,
                                          "allowDowngrade": allow_downgrade or None}, timeout=180.0)
        return Instance.from_payload(body)

    async def background(self, version: str | None = None, *, new: bool = False, file: str | None = None,
                         installation: str | None = None) -> Instance:
        """This checkout's Locus-managed background instance (``-b``)."""
        body = await self._rpc("background", {"version": version, "new": new or None, "file": file,
                                              "installation": installation}, timeout=180.0)
        return Instance.from_payload(body)

    async def close(self, instance: Any) -> None:
        """Close a background instance Locus launched (never interactive ones)."""
        await self._rpc("close", {"instance": _target_text(instance)})

    # ── installations ──────────────────────────────────────────────────────
    async def installations(self) -> list[dict[str, Any]]:
        return await self.client.rpc("blender.installations", {}, timeout=180.0)

    async def workspace_version(self) -> dict[str, Any]:
        """Pinned or inferred Blender version and the `.blend` version counts."""
        return await self._rpc("workspace_version", timeout=120.0)

    async def set_workspace_version(self, version: str | None) -> dict[str, Any]:
        return await self._rpc("set_workspace_version", {"version": version or ""})

    async def file_info(self, path: str) -> dict[str, Any]:
        """Saved version of a .blend file and which installations can open it."""
        return await self._rpc("file_info", {"path": path}, timeout=180.0)

    async def install_addon(self, installation: str | None = None, *, all: bool = False) -> list[dict[str, Any]]:
        return await self.client.rpc("blender.install_addon", {"installation": installation, "all": all},
                                     timeout=300.0)

    async def reload_hosts(self) -> dict[str, Any]:
        return await self.client.rpc("blender.reload_hosts", {}, timeout=120.0)

    # ── helper proxies ─────────────────────────────────────────────────────
    async def _helper(self, name: str, args: tuple[Any, ...], kwargs: dict[str, Any], target: Any) -> Any:
        payload = {"mode": "code", "code": f"return blender.{name}(*__args, **__kwargs)",
                   "readonly": _HELPERS[name], "args": {"__args": tuple(args), "__kwargs": dict(kwargs)}}
        result = await self._run(payload, target=target, timeout=None, encode_keys=("args",),
                                 label=f"Locus: {name}")
        return self._finish(result).value

    async def look(self, *args: Any, target: Any = None, **kwargs: Any) -> Image | list[Image]:
        """``blender.look(...)`` in the target; returns ``locus.Image``."""
        return await self._helper("look", args, kwargs, target)

    async def scene_summary(self, *args: Any, target: Any = None, **kwargs: Any) -> str:
        return await self._helper("scene_summary", args, kwargs, target)

    async def object_detail(self, *args: Any, target: Any = None, **kwargs: Any) -> str:
        return await self._helper("object_detail", args, kwargs, target)

    async def file_report(self, *args: Any, target: Any = None, path: str | None = None, **kwargs: Any) -> str:
        """Report on the open file; with ``path=``, a background instance opens
        that file first (e.g. check missing textures before importing)."""
        if path is not None:
            instance = await self.background(file=path, new=False)
            target = instance
        return await self._helper("file_report", args, kwargs, target)

    async def api(self, *args: Any, target: Any = None, **kwargs: Any) -> str:
        return await self._helper("api", args, kwargs, target)

    async def focus(self, *args: Any, target: Any = None, **kwargs: Any) -> Any:
        return await self._helper("focus", args, kwargs, target)

    async def render_thumbnail(self, *args: Any, target: Any = None, **kwargs: Any) -> Any:
        return await self._helper("render_thumbnail", args, kwargs, target)


class ModuleHandle:
    """Attribute calls run ``getattr(module, name)(*args, **kwargs)`` in Blender."""

    _LOADER = (
        "import importlib.util, os, sys\n"
        "path = os.path.abspath(os.path.join(project or os.getcwd(), __path))\n"
        "key = '__locus_module__' + str(abs(hash(path)))\n"
        "stamp = os.stat(path).st_mtime_ns\n"
        "module = sys.modules.get(key)\n"
        "if module is None or getattr(module, '__locus_stamp__', None) != stamp:\n"
        "    spec = importlib.util.spec_from_file_location(key, path)\n"
        "    module = importlib.util.module_from_spec(spec)\n"
        "    spec.loader.exec_module(module)\n"
        "    module.__locus_stamp__ = stamp\n"
        "    sys.modules[key] = module\n"
    )

    def __init__(self, blender: Blender, path: str, target: Any) -> None:
        self._blender = blender
        self._path = path
        self._target = target

    async def _load(self) -> None:
        await self._blender.execute(self._LOADER, self._target, {"__path": self._path}, readonly=True)

    def __getattr__(self, name: str) -> Callable[..., Any]:
        if name.startswith("_"):
            raise AttributeError(name)

        async def call(*args: Any, **kwargs: Any) -> Any:
            code = self._LOADER + "return getattr(module, __name)(*__args, **__kwargs)\n"
            result = await self._blender.execute(code, self._target, {
                "__path": self._path, "__name": name, "__args": tuple(args), "__kwargs": dict(kwargs),
            })
            return result.value

        call.__name__ = name
        return call

    def __repr__(self) -> str:
        return f"<blender module {self._path}>"


def _first_line(code: str) -> str:
    for line in str(code).splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            return line[:48]
    return "execute"


# `locus.blender.BlenderError`, `locus.blender.Ref`, ... through the default object.
for _name in ("BlenderError", "NotExecuted", "TimedOut", "TargetUnavailable", "Instance", "Result", "Ref", "Array",
              "CodecError"):
    setattr(Blender, _name, globals()[_name])
Blender.Image = Image
del _name
