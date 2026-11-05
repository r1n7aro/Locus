"""Image objects returned by `blender.look()` and attached to tool results.

On the Locus Python side the same tagged value decodes into `locus.Image`.
"""

import os
import struct


def png_size(data):
    """(width, height) of PNG bytes, or (None, None)."""
    if len(data) >= 24 and data[:8] == b"\x89PNG\r\n\x1a\n":
        width, height = struct.unpack(">II", data[16:24])
        return width, height
    return None, None


def _sniff_mime(data):
    if data[:8] == b"\x89PNG\r\n\x1a\n":
        return "image/png"
    if data[:3] == b"\xff\xd8\xff":
        return "image/jpeg"
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "image/webp"
    return None


class Image:
    """Encoded image bytes plus size and capture metadata (`meta`)."""

    __slots__ = ("data", "mime_type", "width", "height", "meta", "label")

    def __init__(self, data, mime_type=None, width=None, height=None, meta=None, label=None):
        data = bytes(data)
        mime_type = mime_type or _sniff_mime(data)
        if mime_type not in ("image/png", "image/jpeg", "image/webp"):
            raise ValueError("only PNG, JPEG and WebP images are supported")
        if mime_type == "image/png" and (width is None or height is None):
            width, height = png_size(data)
        self.data = data
        self.mime_type = mime_type
        self.width = width
        self.height = height
        self.meta = dict(meta or {})
        self.label = label

    @classmethod
    def from_file(cls, path, meta=None, label=None):
        with open(path, "rb") as handle:
            return cls(handle.read(), meta=meta, label=label)

    def save(self, path):
        os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        with open(path, "wb") as handle:
            handle.write(self.data)
        return path

    def __repr__(self):
        kind = self.mime_type.split("/")[-1]
        size = f"{self.width}x{self.height}" if self.width else "?"
        mode = self.meta.get("mode")
        return f"<Image {size} {kind}{' ' + mode if mode else ''}>"
