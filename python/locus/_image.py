"""Images returned by Blender helpers and `locus.attach_image`."""
from __future__ import annotations

import os
import struct
import tempfile
import uuid
from typing import Any

_MAX_ATTACH_BYTES = 20 * 1024 * 1024


def _sniff_mime(data: bytes) -> str | None:
    if data[:8] == b"\x89PNG\r\n\x1a\n":
        return "image/png"
    if data[:3] == b"\xff\xd8\xff":
        return "image/jpeg"
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    return None


def _png_size(data: bytes) -> tuple[int | None, int | None]:
    if len(data) >= 24 and data[:8] == b"\x89PNG\r\n\x1a\n":
        width, height = struct.unpack(">II", data[16:24])
        return width, height
    return None, None


class Image:
    """Encoded image bytes (PNG, JPEG or WebP) with size and capture metadata.

    ``blender.look()`` results arrive as ``Image``; ``meta`` holds the mode,
    view and camera matrices at capture time.
    """

    __slots__ = ("data", "mime_type", "width", "height", "meta", "label")

    def __init__(self, data: bytes, mime_type: str | None = None, width: int | None = None,
                 height: int | None = None, meta: dict[str, Any] | None = None, label: str | None = None) -> None:
        data = bytes(data)
        mime_type = mime_type or _sniff_mime(data)
        if mime_type not in ("image/png", "image/jpeg", "image/webp"):
            raise ValueError("only PNG, JPEG and WebP images are supported")
        if mime_type == "image/png" and (width is None or height is None):
            width, height = _png_size(data)
        self.data = data
        self.mime_type = mime_type
        self.width = width
        self.height = height
        self.meta = dict(meta or {})
        self.label = label

    @classmethod
    def from_file(cls, path: str | os.PathLike[str], *, label: str | None = None) -> "Image":
        with open(path, "rb") as handle:
            return cls(handle.read(), label=label)

    def save(self, path: str | os.PathLike[str]) -> str:
        path = os.fspath(path)
        directory = os.path.dirname(os.path.abspath(path))
        os.makedirs(directory, exist_ok=True)
        with open(path, "wb") as handle:
            handle.write(self.data)
        return path

    def __repr__(self) -> str:
        kind = self.mime_type.split("/")[-1]
        size = f"{self.width}x{self.height}" if self.width else "?"
        mode = self.meta.get("mode")
        return f"<Image {size} {kind}{' ' + str(mode) if mode else ''}>"


def _image_bytes(image: Any, label: str | None) -> tuple[bytes, str | None]:
    if isinstance(image, Image):
        return image.data, label or image.label
    if isinstance(image, (bytes, bytearray, memoryview)):
        return bytes(image), label
    if isinstance(image, (str, os.PathLike)):
        with open(image, "rb") as handle:
            return handle.read(), label
    raise TypeError("attach_image takes a locus.Image, a file path or bytes")


class _Done:
    """Already finished: `attach_image(...)` works with or without await."""

    __slots__ = ()

    def __await__(self):
        return iter(())

    def __repr__(self) -> str:
        return "<attached>"


def attach_image(image: Any, label: str | None = None) -> _Done:
    """Attach an image to the current tool result (at most 4 per call).

    Works in the ``python`` tool (the image is returned with the result) and
    inside Blender code (``blender_execute``, ``@locus.blender.function``),
    where it is the same as returning the image. PNG, JPEG and WebP up to
    20 MiB; downscale large captures (``look(max_long_edge=...)``) first.
    """
    data, label = _image_bytes(image, label)
    if _sniff_mime(data) is None:
        raise ValueError("only PNG, JPEG and WebP images can be attached")
    if len(data) > _MAX_ATTACH_BYTES:
        raise ValueError("images above 20 MiB cannot be attached")
    if os.environ.get("LOCUS_BLENDER_EXECUTION") == "1":
        hook = _blender_attach_hook()
        if hook is not None:
            hook(data, label)
            return _Done()
    call = os.environ.get("LOCUS_PYTHON_CALL", "").strip()
    if not call:
        raise RuntimeError("locus.attach_image works inside a `python` tool call or Blender execution")
    from ._client import Client

    path = os.path.join(tempfile.gettempdir(), f"locus-attach-{uuid.uuid4().hex}.img")
    with open(path, "wb") as handle:
        handle.write(data)
    try:
        Client()._rpc_sync("attachments.add", {"pythonCall": call, "path": path, "delete": True}, None)
    finally:
        if os.path.exists(path):
            try:
                os.remove(path)
            except OSError:
                pass
    return _Done()


def _blender_attach_hook():
    try:
        from locus_host import persist  # type: ignore[import-not-found]
    except Exception:
        return None
    return getattr(persist.state(), "attach_image", None)
