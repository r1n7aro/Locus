"""One snapshot-bound Unity asset API for the YAML and live Editor backends."""
from __future__ import annotations

import math
import os
import re
from typing import Any, Literal, TYPE_CHECKING

from ._scope import workspace_payload

if TYPE_CHECKING:
    from ._client import Client

AssetBackend = Literal["yaml", "live"]
_DECIMAL = re.compile(r"-?(?:0|[1-9][0-9]*)\Z")
_SAFE_INTEGER = (1 << 53) - 1


def asset_integer(value: int | str) -> dict[str, str]:
    """Encode a signed 64-bit integer exactly in both SDKs and backends."""
    text = str(value)
    if isinstance(value, bool) or not isinstance(value, (int, str)) or not _DECIMAL.fullmatch(text) or text == "-0":
        raise ValueError("An asset integer must be a canonical decimal integer")
    if not -(1 << 63) <= int(text) < (1 << 63):
        raise ValueError("An asset integer must fit signed 64-bit range")
    return {"kind": "int64", "value": text}


def asset_unsigned_integer(value: int | str) -> dict[str, str]:
    """Encode a scalar unsigned 64-bit value; object and reference IDs stay signed."""
    text = str(value)
    if isinstance(value, bool) or not isinstance(value, (int, str)) or not re.fullmatch(r"0|[1-9][0-9]*", text):
        raise ValueError("An unsigned asset integer must be a canonical non-negative decimal integer")
    if not 0 <= int(text) < (1 << 64):
        raise ValueError("An unsigned asset integer must fit unsigned 64-bit range")
    return {"kind": "uint64", "value": text}


def _value(value: Any) -> Any:
    if value is None or isinstance(value, (str, bool)):
        return value
    if isinstance(value, int):
        return asset_integer(value) if abs(value) > _SAFE_INTEGER else value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError("Asset values cannot contain NaN or infinity")
        if value.is_integer() and abs(value) > _SAFE_INTEGER:
            raise ValueError("Unsafe floating-point integer; use an int or asset_integer()")
        # JavaScript JSON emits integral numbers without a fractional suffix.
        # Match that representation before either backend validates a field.
        return int(value) if value.is_integer() else value
    if isinstance(value, (list, tuple)):
        return [_value(item) for item in value]
    if isinstance(value, dict):
        if value.get("kind") in ("int64", "uint64"):
            if set(value) != {"kind", "value"} or not isinstance(value["value"], str):
                raise ValueError("An integer value must contain only kind and a decimal string value")
            encode = asset_unsigned_integer if value["kind"] == "uint64" else asset_integer
            return encode(value["value"])
        if not all(isinstance(key, str) for key in value):
            raise ValueError("Asset value object keys must be strings")
        pointer_key = "fileID" if "fileID" in value and value.keys() <= {"fileID", "guid", "type"} else "rid" if set(value) == {"rid"} else None
        if pointer_key is not None:
            reference_id = value[pointer_key]
            if isinstance(reference_id, float):
                reference_id = _value(reference_id)
            if isinstance(reference_id, dict) and reference_id.get("kind") == "int64":
                reference_id = _value(reference_id)["value"]
            encoded = asset_integer(reference_id)["value"]
            return {key: encoded if key == pointer_key else _value(item) for key, item in value.items()}
        return {key: _value(item) for key, item in value.items()}
    raise ValueError(f"Unsupported asset value: {type(value).__name__}")


def _operations(operations: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if not isinstance(operations, list) or not operations:
        raise ValueError("operations must be a non-empty list")
    result = []
    for operation in operations:
        if not isinstance(operation, dict):
            raise ValueError("Each asset operation must be a dictionary")
        op = operation.get("op")
        required = {
            "set": {"value"}, "array_insert": {"index", "value"},
            "array_remove": {"index"}, "array_move": {"index", "to_index"},
            "array_resize": {"size"},
        }.get(op)
        if required is None:
            raise ValueError(f"Unsupported asset operation: {op}")
        common = {"op", "object_id", "property_path"}
        allowed = common | required | ({"value"} if op == "array_resize" else set())
        if not (common | required) <= operation.keys() or not operation.keys() <= allowed:
            raise ValueError(f"Invalid fields for asset operation {op}")
        object_id = operation["object_id"]
        if not isinstance(object_id, str):
            raise ValueError("object_id must be an exact decimal string")
        asset_integer(object_id)
        path = operation["property_path"]
        if not isinstance(path, str) or not path.startswith("/") or re.search(r"~(?![01])", path):
            raise ValueError("property_path must be a root-inclusive RFC 6901 pointer")
        for key in required & {"index", "to_index", "size"}:
            value = operation[key]
            if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= _SAFE_INTEGER:
                raise ValueError(f"{key} must be a non-negative safe integer")
        try:
            result.append({key: _value(value) if key == "value" else value for key, value in operation.items()})
        except RecursionError as error:
            raise ValueError("Asset values cannot contain cycles or exceed the nesting limit") from error
    return result


def _filters(object_id: str | None, property_path: str | None) -> dict[str, Any]:
    if object_id is not None:
        if not isinstance(object_id, str):
            raise ValueError("object_id must be an exact decimal string")
        asset_integer(object_id)
    if property_path is not None and (not isinstance(property_path, str) or not property_path.startswith("/") or re.search(r"~(?![01])", property_path)):
        raise ValueError("property_path must be a root-inclusive RFC 6901 pointer")
    return {"object_id": object_id, "property_path": property_path}


class AssetResult(dict[str, Any]):
    """The common JSON result, with attribute access for top-level fields."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError as error:
            raise AttributeError(name) from error


class Assets:
    """Asset operations bound to one backend and checkout.

    YAML is the default and never starts an Editor. Select ``backend('live')``
    for the connected Editor. Backend selection does not mutate other contexts.
    """

    def __init__(self, client: Client, backend_name: AssetBackend = "yaml", *,
                 workspace_ref: dict[str, Any] | None = None) -> None:
        if backend_name not in {"yaml", "live"}:
            raise ValueError("backend must be yaml or live")
        self._client = client
        self._backend_name = backend_name
        self._workspace_ref = dict(workspace_ref) if workspace_ref is not None else None

    def backend(self, name: AssetBackend, *, workspace_ref: Any = None, worktree: Any = None) -> Assets:
        reference = workspace_payload(workspace_ref, worktree) if workspace_ref is not None or worktree is not None else self._workspace_ref or workspace_payload()
        return Assets(self._client, name, workspace_ref=reference)

    @staticmethod
    def integer(value: int | str) -> dict[str, str]:
        return asset_integer(value)

    @staticmethod
    def unsigned_integer(value: int | str) -> dict[str, str]:
        return asset_unsigned_integer(value)

    async def _call(self, action: str, path: str | None = None, params: dict[str, Any] | None = None, *,
                    workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        if path is not None and (not isinstance(path, str) or not path.strip()):
            raise ValueError("path must name a project-relative Unity asset")
        if self._workspace_ref is not None and (workspace_ref is not None or worktree is not None):
            raise ValueError("This asset context is bound; select a new context to change its checkout")
        reference = dict(self._workspace_ref) if self._workspace_ref is not None else workspace_payload(workspace_ref, worktree)
        payload = await self._client.rpc("assets." + action, {
            "workspaceRef": reference, "backend": self._backend_name,
            **({"path": path} if path is not None else {}),
            "execution_delegation": os.environ.get("LOCUS_SDK_EXECUTION_DELEGATION"),
            **(params or {}),
        }, timeout=300)
        return AssetResult(payload)

    async def capabilities(self, *, workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        return await self._call("capabilities", workspace_ref=workspace_ref, worktree=worktree)

    async def recover(self, transaction_id: str, *, workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        """Recover an interrupted YAML transaction without overwriting later edits."""
        if not isinstance(transaction_id, str) or not transaction_id.strip():
            raise ValueError("transaction_id must name a recorded asset transaction")
        return await self._call("recover", params={"transaction_id": transaction_id},
                                workspace_ref=workspace_ref, worktree=worktree)

    async def read(self, path: str, *, object_id: str | None = None, property_path: str | None = None,
                   workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        return await self._call("read", path, _filters(object_id, property_path),
                                workspace_ref=workspace_ref, worktree=worktree)

    async def discover(self, path: str, *, query: str | None = None, object_id: str | None = None,
                       property_path: str | None = None, offset: int = 0, limit: int = 100,
                       workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        if any(isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= _SAFE_INTEGER for value in [offset, limit]) or limit == 0:
            raise ValueError("offset must be a non-negative integer and limit a positive integer")
        return await self._call("discover", path, {"query": query, **_filters(object_id, property_path),
            "offset": offset, "limit": limit},
            workspace_ref=workspace_ref, worktree=worktree)

    async def preview(self, path: str, operations: list[dict[str, Any]], *,
                      expected_revision: str | None = None, workspace_ref: Any = None,
                      worktree: Any = None, persist: Literal["disk"] = "disk") -> AssetResult:
        """Validate all operations and return the proposed snapshot without saving."""
        if persist != "disk":
            raise ValueError("persist must be disk")
        return await self._call("preview", path, {"operations": _operations(operations), "persist": persist,
            "expected_revision": expected_revision}, workspace_ref=workspace_ref, worktree=worktree)

    async def apply(self, path: str, operations: list[dict[str, Any]], *, expected_revision: str,
                    workspace_ref: Any = None, worktree: Any = None,
                    persist: Literal["disk"] = "disk") -> AssetResult:
        """Apply and persist against the revision returned by read/discover.

        A stale revision fails before writing. Retry only after re-reading and
        reviewing the current state; array operations are not safe to replay.
        """
        if not isinstance(expected_revision, str) or not expected_revision.strip():
            raise ValueError("apply requires expected_revision from a current snapshot")
        if persist != "disk":
            raise ValueError("persist must be disk")
        return await self._call("apply", path, {"operations": _operations(operations), "persist": persist,
            "expected_revision": expected_revision}, workspace_ref=workspace_ref, worktree=worktree)

    async def _batch(self, action: str, entries: list[dict[str, Any]], *,
                     workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        if not isinstance(entries, list) or not entries:
            raise ValueError("entries must be a non-empty list")
        requests = []
        for entry in entries:
            if not isinstance(entry, dict) or not {"path", "operations"} <= entry.keys() or not entry.keys() <= {"path", "operations", "expected_revision"}:
                raise ValueError("Each batch entry requires path, operations and optionally expected_revision")
            if not isinstance(entry["path"], str) or not entry["path"].strip():
                raise ValueError("Each batch entry must name a project-relative Unity asset")
            revision = entry.get("expected_revision")
            if action == "apply_batch" and (not isinstance(revision, str) or not revision.strip()):
                raise ValueError("Every apply_batch entry requires expected_revision from a current snapshot")
            requests.append({**entry, "operations": _operations(entry["operations"])})
        return await self._call(action, params={"entries": requests, "persist": "disk"},
                                workspace_ref=workspace_ref, worktree=worktree)

    async def preview_batch(self, entries: list[dict[str, Any]], *,
                            workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        return await self._batch("preview_batch", entries, workspace_ref=workspace_ref, worktree=worktree)

    async def apply_batch(self, entries: list[dict[str, Any]], *,
                          workspace_ref: Any = None, worktree: Any = None) -> AssetResult:
        """Validate every asset first, then persist the batch as one transaction."""
        return await self._batch("apply_batch", entries, workspace_ref=workspace_ref, worktree=worktree)
