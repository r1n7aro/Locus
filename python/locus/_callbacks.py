from __future__ import annotations

import asyncio
import concurrent.futures
import hmac
import json
import secrets
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from ._tools import Tool

_MAX_BODY_BYTES = 2 * 1024 * 1024


@dataclass(slots=True)
class _Registration:
    tool: Tool
    loop: asyncio.AbstractEventLoop


class CallbackRegistry:
    def __init__(self) -> None:
        self._token = secrets.token_hex(32)
        self._entries: dict[str, _Registration] = {}
        self._lock = threading.RLock()
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    @property
    def token(self) -> str:
        return self._token

    @property
    def url(self) -> str:
        self._ensure_started()
        assert self._server is not None
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}/tool"

    def register(
        self,
        callback_key: str,
        tool: Tool,
        loop: asyncio.AbstractEventLoop,
    ) -> None:
        self._ensure_started()
        with self._lock:
            self._entries[callback_key] = _Registration(tool=tool, loop=loop)

    def unregister(self, callback_keys: list[str] | tuple[str, ...]) -> None:
        with self._lock:
            for callback_key in callback_keys:
                self._entries.pop(callback_key, None)

    def _registration(self, callback_key: str) -> _Registration | None:
        with self._lock:
            return self._entries.get(callback_key)

    def _invoke(self, callback_key: str, arguments: dict[str, Any]) -> Any:
        registration = self._registration(callback_key)
        if registration is None:
            raise KeyError("Python tool callback is no longer registered")
        future = asyncio.run_coroutine_threadsafe(
            registration.tool.invoke(arguments),
            registration.loop,
        )
        try:
            return future.result(timeout=registration.tool.timeout)
        except concurrent.futures.TimeoutError as error:
            future.cancel()
            raise TimeoutError(
                f"Python tool '{registration.tool.name}' timed out after "
                f"{registration.tool.timeout:g}s"
            ) from error

    def _ensure_started(self) -> None:
        with self._lock:
            if self._server is not None:
                return
            registry = self

            class Handler(BaseHTTPRequestHandler):
                server_version = "LocusPythonTool/1"

                def do_POST(self) -> None:
                    if self.path != "/tool":
                        self._write(404, {"ok": False, "error": "not found"})
                        return
                    if self.headers.get("Origin") is not None:
                        self._write(403, {"ok": False, "error": "browser origins are not allowed"})
                        return
                    expected = f"Bearer {registry.token}"
                    if not hmac.compare_digest(self.headers.get("Authorization", ""), expected):
                        self._write(401, {"ok": False, "error": "unauthorized"})
                        return
                    try:
                        length = int(self.headers.get("Content-Length", "0"))
                    except ValueError:
                        length = 0
                    if length <= 0 or length > _MAX_BODY_BYTES:
                        self._write(413, {"ok": False, "error": "invalid request size"})
                        return
                    try:
                        payload = json.loads(self.rfile.read(length).decode("utf-8"))
                        callback_key = str(payload["toolKey"])
                        arguments = payload.get("arguments") or {}
                        result = registry._invoke(callback_key, arguments)
                        json.dumps(result, ensure_ascii=False)
                    except Exception as error:
                        self._write(
                            200,
                            {
                                "ok": False,
                                "error": f"{type(error).__name__}: {error}",
                            },
                        )
                        return
                    self._write(200, {"ok": True, "result": result})

                def _write(self, status: int, payload: dict[str, Any]) -> None:
                    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
                    self.send_response(status)
                    self.send_header("Content-Type", "application/json; charset=utf-8")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

                def log_message(self, format: str, *args: Any) -> None:
                    return

            self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            self._thread = threading.Thread(
                target=self._server.serve_forever,
                name="locus-python-tools",
                daemon=True,
            )
            self._thread.start()


callbacks = CallbackRegistry()
