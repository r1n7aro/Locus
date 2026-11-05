"""Tagged JSON value codec shared by `blender_execute` and `locus.blender`.

Rules (both directions unless noted):

* None, bool, int, float, str: plain JSON. Non-finite floats: {"$f": "nan"}.
* list: JSON array. tuple: {"$t": [...]}. set/frozenset: {"$s": [...]}.
* dict: JSON object when every key is a str not starting with "$";
  otherwise {"$m": [[key, value], ...]} (non-string keys survive).
* bytes/bytearray/memoryview: {"$b": attachment}.
* numpy arrays: {"$nd": {"dtype", "shape", "data": attachment}}; object
  arrays are rejected. numpy scalars become Python numbers.
* pathlib paths: str (outbound only).
* images: {"$img": {"mime", "width", "height", "meta", "label", "data"}}.
* Blender data (outbound): ID blocks become {"$ref": {"type", "collection",
  "name", "library"}}; nested structs add "path" (`path_from_id`).
  Collections become lists of refs (at most 10000 items). Vectors, Eulers,
  Quaternions and Colors become tuples; matrices tuples of row tuples.
* Inbound refs resolve to the live data block; a deleted or renamed target
  is an error.
* Anything else: an error naming the location (`args["mesh"][2]`), or in
  lenient mode (`blender_execute` results) its repr plus a warning.

Attachments are {"b64": ...} up to the inline limit, else a named shared
memory reference {"shm": name, "size": n} (see `shm`).
"""

import base64
import math
import pathlib

from .images import Image

INLINE_LIMIT = 1 << 20
COLLECTION_LIMIT = 10_000

_ID_COLLECTIONS = {
    "ACTION": "actions", "ARMATURE": "armatures", "BRUSH": "brushes", "CACHEFILE": "cache_files",
    "CAMERA": "cameras", "COLLECTION": "collections", "CURVE": "curves", "CURVES": "hair_curves",
    "FONT": "fonts", "GREASEPENCIL": "grease_pencils", "GREASEPENCIL_V3": "grease_pencils_v3",
    "IMAGE": "images", "KEY": "shape_keys", "LATTICE": "lattices", "LIBRARY": "libraries",
    "LIGHT": "lights", "LIGHT_PROBE": "lightprobes", "LINESTYLE": "linestyles", "MASK": "masks",
    "MATERIAL": "materials", "MESH": "meshes", "META": "metaballs", "MOVIECLIP": "movieclips",
    "NODETREE": "node_groups", "OBJECT": "objects", "PAINTCURVE": "paint_curves",
    "PALETTE": "palettes", "PARTICLE": "particles", "POINTCLOUD": "pointclouds", "SCENE": "scenes",
    "SCREEN": "screens", "SOUND": "sounds", "SPEAKER": "speakers", "TEXT": "texts",
    "TEXTURE": "textures", "VOLUME": "volumes", "WINDOWMANAGER": "window_managers",
    "WORKSPACE": "workspaces", "WORLD": "worlds",
}


class CodecError(TypeError):
    """A value that cannot cross the Locus/Blender boundary."""


def _numpy():
    try:
        import numpy
        return numpy
    except Exception:
        return None


def _bpy():
    try:
        import bpy
        return bpy
    except Exception:
        return None


def _mathutils():
    try:
        import mathutils
        return mathutils
    except Exception:
        return None


def _path_child(path, key):
    return f"{path}[{key!r}]"


class Encoder:
    """Encode one value. `lenient` turns unsupported objects into reprs."""

    def __init__(self, lenient=False, shm_pool=None, call_id="x", inline_limit=INLINE_LIMIT):
        self.lenient = lenient
        self.shm_pool = shm_pool
        self.call_id = call_id
        self.inline_limit = inline_limit
        self.warnings = []
        self.images = []
        self.shm_names = []

    def attachment(self, data):
        data = bytes(data)
        if len(data) <= self.inline_limit or self.shm_pool is None:
            return {"b64": base64.b64encode(data).decode("ascii")}
        ref = self.shm_pool.put(data, self.call_id)
        self.shm_names.append(ref["shm"])
        return ref

    def encode(self, value, path="value"):
        if value is None or isinstance(value, (bool, str)):
            return value
        if isinstance(value, int):
            return value
        if isinstance(value, float):
            if math.isnan(value):
                return {"$f": "nan"}
            if math.isinf(value):
                return {"$f": "inf" if value > 0 else "-inf"}
            return value
        if isinstance(value, Image):
            self.images.append(value)
            return {"$img": {
                "mime": value.mime_type,
                "width": value.width,
                "height": value.height,
                "meta": self.encode(value.meta, f"{path}.meta"),
                "label": value.label,
                "data": self.attachment(value.data),
            }}
        if isinstance(value, list):
            return [self.encode(item, _path_child(path, index)) for index, item in enumerate(value)]
        if isinstance(value, tuple):
            return {"$t": [self.encode(item, _path_child(path, index)) for index, item in enumerate(value)]}
        if isinstance(value, (set, frozenset)):
            return {"$s": [self.encode(item, f"{path}{{…}}") for item in value]}
        if isinstance(value, dict):
            if all(isinstance(key, str) and not key.startswith("$") for key in value):
                return {key: self.encode(item, _path_child(path, key)) for key, item in value.items()}
            return {"$m": [[self.encode(key, f"{path}.key"), self.encode(item, _path_child(path, key))] for key, item in value.items()]}
        if isinstance(value, (bytes, bytearray, memoryview)):
            return {"$b": self.attachment(value)}
        if isinstance(value, pathlib.PurePath):
            return str(value)

        numpy = _numpy()
        if numpy is not None:
            if isinstance(value, numpy.ndarray):
                if value.dtype.kind == "O":
                    return self._unsupported(value, path, "object arrays cannot be transferred")
                array = numpy.ascontiguousarray(value)
                return {"$nd": {
                    "dtype": array.dtype.str,
                    "shape": list(array.shape),
                    "data": self.attachment(array.tobytes()),
                }}
            if isinstance(value, numpy.generic):
                return self.encode(value.item(), path)

        mathutils = _mathutils()
        if mathutils is not None:
            if isinstance(value, mathutils.Matrix):
                return {"$t": [{"$t": [self.encode(float(x)) for x in row]} for row in value]}
            if isinstance(value, (mathutils.Vector, mathutils.Euler, mathutils.Quaternion, mathutils.Color)):
                return {"$t": [self.encode(float(x)) for x in value]}

        bpy = _bpy()
        if bpy is not None:
            encoded = self._encode_blender(bpy, value, path)
            if encoded is not NotImplemented:
                return encoded

        return self._unsupported(value, path, f"type {type(value).__name__} cannot be transferred")

    def _encode_blender(self, bpy, value, path):
        types = bpy.types
        if isinstance(value, types.ID):
            return {"$ref": id_ref(value)}
        if isinstance(value, types.bpy_prop_collection):
            items = list(value)
            if len(items) > COLLECTION_LIMIT:
                raise CodecError(
                    f"{path}: collection has {len(items)} items (limit {COLLECTION_LIMIT}); "
                    "use foreach_get into a numpy array instead"
                )
            return [self.encode(item, _path_child(path, index)) for index, item in enumerate(items)]
        if isinstance(value, types.bpy_struct):
            owner = getattr(value, "id_data", None)
            if owner is not None and isinstance(owner, types.ID):
                ref = id_ref(owner)
                try:
                    ref["path"] = value.path_from_id()
                except Exception:
                    return self._unsupported(value, path, "struct has no path from its data block")
                ref["struct"] = type(value).__name__
                return {"$ref": ref}
            return self._unsupported(value, path, f"{type(value).__name__} is not owned by a data block")
        array_type = getattr(bpy.types, "bpy_prop_array", None)
        if array_type is not None and isinstance(value, array_type):
            return {"$t": [self.encode(item, _path_child(path, index)) for index, item in enumerate(value)]}
        return NotImplemented

    def _unsupported(self, value, path, reason):
        if not self.lenient:
            raise CodecError(f"{path}: {reason}")
        text = _short_repr(value)
        self.warnings.append(f"{path}: {reason}; returned its repr")
        return text


def id_ref(block):
    library = getattr(block, "library", None)
    return {
        "type": type(block).__name__,
        "collection": _ID_COLLECTIONS.get(getattr(block, "id_type", ""), ""),
        "name": block.name,
        "library": library.filepath if library is not None else None,
    }


def _short_repr(value, limit=2000):
    try:
        text = repr(value)
    except Exception as error:
        text = f"<unrepresentable {type(value).__name__}: {error}>"
    return text if len(text) <= limit else text[: limit - 1] + "…"


def short_repr(value, limit=20_000):
    return _short_repr(value, limit)


# ── decoding ───────────────────────────────────────────────────────────────


def read_attachment(ref, released=None):
    if "b64" in ref:
        return base64.b64decode(ref["b64"])
    if "shm" in ref:
        from . import shm

        data = shm.read(ref)
        if released is not None:
            released.append(ref["shm"])
        return data
    raise CodecError("attachment has neither inline data nor a shared memory name")


class Decoder:
    """Decode a tagged value; `released` collects shm names to acknowledge."""

    def __init__(self):
        self.released = []

    def decode(self, value, path="args"):
        if isinstance(value, list):
            return [self.decode(item, _path_child(path, index)) for index, item in enumerate(value)]
        if not isinstance(value, dict):
            return value
        if len(value) == 1:
            (tag, inner), = value.items()
            if tag == "$f":
                return float(inner)
            if tag == "$t":
                return tuple(self.decode(item, _path_child(path, index)) for index, item in enumerate(inner))
            if tag == "$s":
                return set(self.decode(item, path) for item in inner)
            if tag == "$m":
                return {self.decode(key, path): self.decode(item, _path_child(path, key)) for key, item in inner}
            if tag == "$b":
                return read_attachment(inner, self.released)
            if tag == "$nd":
                numpy = _numpy()
                if numpy is None:
                    raise CodecError(f"{path}: numpy is not available in this Blender")
                data = read_attachment(inner["data"], self.released)
                return numpy.frombuffer(data, dtype=numpy.dtype(inner["dtype"])).reshape(inner["shape"]).copy()
            if tag == "$img":
                return Image(
                    read_attachment(inner["data"], self.released),
                    mime_type=inner.get("mime"),
                    width=inner.get("width"),
                    height=inner.get("height"),
                    meta=self.decode(inner.get("meta") or {}, path),
                    label=inner.get("label"),
                )
            if tag == "$ref":
                return resolve_ref(inner, path)
        return {key: self.decode(item, _path_child(path, key)) for key, item in value.items()}


def resolve_ref(ref, path="args"):
    bpy = _bpy()
    if bpy is None:
        raise CodecError(f"{path}: data block references need Blender")
    collection_name = ref.get("collection") or ""
    collection = getattr(bpy.data, collection_name, None) if collection_name else None
    if collection is None:
        raise CodecError(f"{path}: unknown data collection for {ref.get('type')!r}")
    name = ref.get("name")
    library = ref.get("library")
    block = collection.get((name, library)) if library else collection.get(name)
    if block is None:
        raise CodecError(
            f"{path}: {ref.get('type')} {name!r} no longer exists (deleted or renamed)"
        )
    struct_path = ref.get("path")
    if struct_path:
        try:
            return block.path_resolve(struct_path)
        except (ValueError, AttributeError) as error:
            raise CodecError(f"{path}: {ref.get('type')} {name!r} has no {struct_path!r} ({error})")
    return block
