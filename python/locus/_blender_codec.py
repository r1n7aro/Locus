"""Value codec between Locus Python and Blender (mirror of the Blender host's
`locus_host/codec.py`; both sides follow the same tags).

* None, bool, int, float, str: plain JSON; nan/inf as {"$f": "nan"}.
* tuple {"$t"}, set {"$s"}, dicts with non-str keys {"$m"}; lists stay lists.
* bytes {"$b": att}; numpy arrays {"$nd": {"dtype", "shape", "data": att}}
  (object arrays rejected); numpy scalars become numbers; paths become str.
* images {"$img": {...}} <-> `locus.Image`; data blocks {"$ref"} <-> `Ref`.

Attachments travel inline (base64) up to `INLINE_LIMIT`, larger ones through
named shared memory written by the sender (`Local\\locus_blender_...`).
"""
from __future__ import annotations

import base64
import math
import mmap
import os
import pathlib
import secrets
import sys
from dataclasses import dataclass, field
from typing import Any

from ._image import Image

# The SDK request body is capped at 2 MiB; keep inline data well below it.
INLINE_LIMIT = 256 * 1024


class CodecError(TypeError):
    """A value that cannot cross the Locus/Blender boundary."""


@dataclass(frozen=True)
class Ref:
    """Reference to Blender data (`bpy.data.<collection>[name]`, optionally a
    nested struct via `path`). Resolved to the live object inside Blender;
    a deleted or renamed target raises there."""

    type: str
    name: str
    collection: str = ""
    library: str | None = None
    path: str | None = None
    struct: str | None = None

    def to_payload(self) -> dict[str, Any]:
        payload: dict[str, Any] = {"type": self.type, "name": self.name, "collection": self.collection,
                                   "library": self.library}
        if self.path:
            payload["path"] = self.path
        if self.struct:
            payload["struct"] = self.struct
        return payload

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "Ref":
        return cls(type=str(payload.get("type") or ""), name=str(payload.get("name") or ""),
                   collection=str(payload.get("collection") or ""), library=payload.get("library"),
                   path=payload.get("path"), struct=payload.get("struct"))

    def __repr__(self) -> str:
        where = f".{self.path}" if self.path else ""
        lib = f" (library {self.library})" if self.library else ""
        return f"Ref({self.type} {self.name!r}{where}{lib})"


@dataclass
class Array:
    """An n-dimensional array received while numpy is not installed."""

    dtype: str
    shape: tuple[int, ...]
    data: bytes = field(repr=False)

    def tolist(self) -> list[Any]:
        import array as _array
        import struct as _struct

        code = _STRUCT_CODES.get(self.dtype.lstrip("<>|=")) if self.dtype else None
        if code is None:
            raise TypeError(f"cannot convert dtype {self.dtype!r} without numpy")
        order = ">" if self.dtype.startswith(">") else "<"
        count = len(self.data) // _struct.calcsize(code)
        flat = list(_struct.unpack(f"{order}{count}{code}", self.data))
        del _array
        return _reshape(flat, list(self.shape))


_STRUCT_CODES = {"f4": "f", "f8": "d", "i1": "b", "u1": "B", "i2": "h", "u2": "H", "i4": "i", "u4": "I",
                 "i8": "q", "u8": "Q", "b1": "?"}


def _reshape(flat: list[Any], shape: list[int]) -> list[Any]:
    if len(shape) <= 1:
        return flat
    step = len(flat) // shape[0] if shape[0] else 0
    return [_reshape(flat[i * step:(i + 1) * step], shape[1:]) for i in range(shape[0])]


def _numpy():
    try:
        import numpy
        return numpy
    except Exception:
        return None


def _child(path: str, key: Any) -> str:
    return f"{path}[{key!r}]"


class Encoder:
    """Encode arguments; keeps shared memory mappings alive until `close()`."""

    def __init__(self, call_id: str = "sdk", inline_limit: int = INLINE_LIMIT) -> None:
        self.call_id = "".join(c for c in call_id if c.isalnum() or c in "-_")[:32] or "sdk"
        self.inline_limit = inline_limit
        self._mappings: list[mmap.mmap] = []
        self._seq = 0

    def close(self) -> None:
        for mapping in self._mappings:
            try:
                mapping.close()
            except Exception:
                pass
        self._mappings.clear()

    def attachment(self, data: bytes) -> dict[str, Any]:
        if len(data) <= self.inline_limit or sys.platform != "win32":
            return {"b64": base64.b64encode(data).decode("ascii")}
        self._seq += 1
        name = f"Local\\locus_blender_{self.call_id}_{self._seq}_{secrets.token_hex(4)}"
        mapping = mmap.mmap(-1, max(len(data), 1), tagname=name)
        mapping[: len(data)] = data
        self._mappings.append(mapping)
        return {"shm": name, "size": len(data)}

    def encode(self, value: Any, path: str = "args") -> Any:
        if value is None or isinstance(value, (bool, str, int)):
            return value
        if isinstance(value, float):
            if math.isnan(value):
                return {"$f": "nan"}
            if math.isinf(value):
                return {"$f": "inf" if value > 0 else "-inf"}
            return value
        if isinstance(value, Image):
            return {"$img": {"mime": value.mime_type, "width": value.width, "height": value.height,
                             "meta": self.encode(value.meta, f"{path}.meta"), "label": value.label,
                             "data": self.attachment(value.data)}}
        if isinstance(value, Ref):
            return {"$ref": value.to_payload()}
        if isinstance(value, Array):
            return {"$nd": {"dtype": value.dtype, "shape": list(value.shape), "data": self.attachment(value.data)}}
        if isinstance(value, list):
            return [self.encode(item, _child(path, i)) for i, item in enumerate(value)]
        if isinstance(value, tuple):
            return {"$t": [self.encode(item, _child(path, i)) for i, item in enumerate(value)]}
        if isinstance(value, (set, frozenset)):
            return {"$s": [self.encode(item, f"{path}{{…}}") for item in value]}
        if isinstance(value, dict):
            if all(isinstance(key, str) and not key.startswith("$") for key in value):
                return {key: self.encode(item, _child(path, key)) for key, item in value.items()}
            return {"$m": [[self.encode(key, f"{path}.key"), self.encode(item, _child(path, key))]
                           for key, item in value.items()]}
        if isinstance(value, (bytes, bytearray, memoryview)):
            return {"$b": self.attachment(bytes(value))}
        if isinstance(value, pathlib.PurePath):
            return str(value)
        numpy = _numpy()
        if numpy is not None:
            if isinstance(value, numpy.ndarray):
                if value.dtype.kind == "O":
                    raise CodecError(f"{path}: object arrays cannot be transferred")
                array = numpy.ascontiguousarray(value)
                return {"$nd": {"dtype": array.dtype.str, "shape": list(array.shape),
                                "data": self.attachment(array.tobytes())}}
            if isinstance(value, numpy.generic):
                return self.encode(value.item(), path)
        raise CodecError(f"{path}: type {type(value).__name__} cannot be sent to Blender "
                         "(use numbers, strings, lists/tuples/dicts/sets, bytes, numpy arrays, "
                         "locus.Image or locus.blender.Ref)")


def read_attachment(ref: dict[str, Any], released: list[str]) -> bytes:
    if "b64" in ref:
        return base64.b64decode(ref["b64"])
    if "shm" in ref:
        name, size = str(ref["shm"]), int(ref["size"])
        released.append(name)
        if size == 0:
            return b""
        mapping = mmap.mmap(-1, size, tagname=name)
        try:
            return bytes(mapping[:size])
        finally:
            mapping.close()
    raise CodecError("attachment has neither inline data nor a shared memory name")


class Decoder:
    """Decode a tagged value; `released` lists shared memory to acknowledge."""

    def __init__(self) -> None:
        self.released: list[str] = []

    def decode(self, value: Any) -> Any:
        if isinstance(value, list):
            return [self.decode(item) for item in value]
        if not isinstance(value, dict):
            return value
        if len(value) == 1:
            (tag, inner), = value.items()
            if tag == "$f":
                return float(inner)
            if tag == "$t":
                return tuple(self.decode(item) for item in inner)
            if tag == "$s":
                return {self.decode(item) for item in inner}
            if tag == "$m":
                return {self.decode(key): self.decode(item) for key, item in inner}
            if tag == "$b":
                return read_attachment(inner, self.released)
            if tag == "$nd":
                data = read_attachment(inner["data"], self.released)
                numpy = _numpy()
                if numpy is None:
                    return Array(dtype=str(inner["dtype"]), shape=tuple(inner["shape"]), data=data)
                return numpy.frombuffer(data, dtype=numpy.dtype(inner["dtype"])).reshape(inner["shape"]).copy()
            if tag == "$img":
                return Image(read_attachment(inner["data"], self.released), mime_type=inner.get("mime"),
                             width=inner.get("width"), height=inner.get("height"),
                             meta=self.decode(inner.get("meta") or {}), label=inner.get("label"))
            if tag == "$ref":
                return Ref.from_payload(inner)
        return {key: self.decode(item) for key, item in value.items()}


def is_windows() -> bool:
    return os.name == "nt"
