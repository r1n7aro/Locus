"""Helper functions available as `blender` inside executed code.

Implemented once here. `MANIFEST` lists each helper (name, parameters,
read-only flag); the Locus SDK generates same-named proxies from it, so
`await locus.blender.look(...)` in Locus Python and `blender.look(...)` in
Blender take the same arguments.
"""

import contextlib
import math
import os
import tempfile

from .images import Image

MANIFEST = [
    {"name": "view3d", "params": [], "readonly": True,
     "doc": "Context manager overriding window/area/region with the first 3D view (interactive only)."},
    {"name": "look", "params": ["mode='viewport'", "views=None", "shading=None", "frames=None", "max_long_edge=None"],
     "readonly": True,
     "doc": "Image of the scene: mode viewport | camera | angles | frames; shading solid | material | rendered | wireframe."},
    {"name": "scene_summary", "params": ["fields=None", "query=None", "root=None", "limit=20"], "readonly": True,
     "doc": "One summary line, then one line per object."},
    {"name": "object_detail", "params": ["name"], "readonly": True,
     "doc": "Type, transform, parenting, modifiers, constraints and materials of one object."},
    {"name": "file_report", "params": [], "readonly": True,
     "doc": "Data block counts, missing external files, linked libraries, path and save state."},
    {"name": "api", "params": ["identifier"], "readonly": True,
     "doc": "Reflect the running Blender's API (bpy.types.X, X.prop, bpy.ops.x.y)."},
    {"name": "focus", "params": ["name"], "readonly": False,
     "doc": "Select an object and frame it in the 3D view (interactive only)."},
    {"name": "render_thumbnail", "params": ["path", "max_long_edge=256"], "readonly": False,
     "doc": "Render the scene camera to an image file and restore the render settings."},
]

LOOK_MODES = ("viewport", "camera", "angles", "frames")
SHADINGS = ("solid", "material", "rendered", "wireframe")
DEFAULT_LONG_EDGE = 1024
MAX_LONG_EDGE = 2048
ANGLES = {
    "front": (0.0, -1.0, 0.0),
    "back": (0.0, 1.0, 0.0),
    "left": (-1.0, 0.0, 0.0),
    "right": (1.0, 0.0, 0.0),
    "top": (0.0, 0.0, 1.0),
    "iso": (1.0, -1.0, 0.8),
}
DEFAULT_ANGLES = ("front", "right", "top", "iso")


class HelperError(RuntimeError):
    pass


def _bpy():
    import bpy
    return bpy


def _round(values, digits=4):
    return tuple(round(float(v), digits) for v in values)


def _view3d_target(bpy):
    window_manager = bpy.context.window_manager
    if window_manager is None:
        return None
    for window in window_manager.windows:
        screen = window.screen
        if screen is None:
            continue
        for area in screen.areas:
            if area.type != "VIEW_3D":
                continue
            region = next((r for r in area.regions if r.type == "WINDOW"), None)
            if region is not None:
                return window, area, region
    return None


class Helpers:
    """Bound per execution; `readonly` guards mutating helpers."""

    def __init__(self, readonly=False, attach=None):
        self._readonly = readonly
        self._attach = attach

    # ── context ────────────────────────────────────────────────────────────
    @contextlib.contextmanager
    def view3d(self):
        bpy = _bpy()
        if bpy.app.background:
            raise HelperError("view3d() needs an interactive Blender; background instances have no window")
        target = _view3d_target(bpy)
        if target is None:
            raise HelperError("no 3D view is open in this Blender window")
        window, area, region = target
        with bpy.context.temp_override(window=window, area=area, region=region):
            yield area.spaces.active

    def attach_image(self, image, label=None):
        """Attach an image to this tool result (same as returning it)."""
        if not isinstance(image, Image):
            if isinstance(image, (bytes, bytearray)):
                image = Image(image, label=label)
            else:
                image = Image.from_file(os.fspath(image), label=label)
        if label:
            image.label = label
        if self._attach is None:
            raise HelperError("images can only be attached during an execution")
        self._attach(image)
        return image

    # ── look ───────────────────────────────────────────────────────────────
    def look(self, mode="viewport", views=None, shading=None, frames=None, max_long_edge=None):
        bpy = _bpy()
        mode = (mode or "viewport").lower()
        if mode not in LOOK_MODES:
            raise HelperError(f"mode must be one of {', '.join(LOOK_MODES)}")
        if shading is not None and shading not in SHADINGS:
            raise HelperError(f"shading must be one of {', '.join(SHADINGS)}")
        long_edge = int(max_long_edge or DEFAULT_LONG_EDGE)
        long_edge = max(64, min(long_edge, MAX_LONG_EDGE))
        if mode == "viewport":
            if bpy.app.background:
                raise HelperError(
                    "look(mode='viewport') needs an interactive Blender; "
                    "background instances have no window. Use mode='camera' or 'angles'."
                )
            return self._look_viewport(bpy, shading, long_edge)
        if mode == "camera":
            return self._render_camera(bpy, None, shading, long_edge, {"mode": "camera"})
        if mode == "angles":
            names = list(views or DEFAULT_ANGLES)
            unknown = [name for name in names if name not in ANGLES]
            if unknown:
                raise HelperError(f"unknown views {unknown}; choose from {', '.join(ANGLES)}")
            if len(names) > 6:
                raise HelperError("at most 6 views")
            return [self._render_angle(bpy, name, shading, long_edge) for name in names]
        scene = bpy.context.scene
        frame_list = list(frames or [scene.frame_start, (scene.frame_start + scene.frame_end) // 2, scene.frame_end])
        if len(frame_list) > 12:
            raise HelperError("at most 12 frames")
        original = scene.frame_current
        images = []
        try:
            for frame in frame_list:
                scene.frame_set(int(frame))
                images.append(self._render_camera(bpy, None, shading, long_edge, {"mode": "frames", "frame": int(frame)}))
        finally:
            scene.frame_set(original)
        return images

    def _look_viewport(self, bpy, shading, long_edge):
        target = _view3d_target(bpy)
        if target is None:
            raise HelperError("no 3D view is open in this Blender window")
        window, area, region = target
        space = area.spaces.active
        shading_state = space.shading
        previous_type = shading_state.type
        temp_dir = tempfile.mkdtemp(prefix="locus_look_")
        path = os.path.join(temp_dir, "viewport.png")
        try:
            if shading is not None:
                shading_state.type = {"solid": "SOLID", "material": "MATERIAL", "rendered": "RENDERED", "wireframe": "WIREFRAME"}[shading]
            with bpy.context.temp_override(window=window, area=area, region=region):
                bpy.ops.screen.screenshot_area(filepath=path)
            data = _scaled_png(bpy, path, long_edge)
        finally:
            shading_state.type = previous_type
            _remove_tree(temp_dir)
        r3d = space.region_3d
        meta = {
            "mode": "viewport",
            "shading": shading or previous_type.lower(),
            "view_matrix": [list(_round(row)) for row in r3d.view_matrix],
            "perspective": r3d.view_perspective,
            "lens": round(space.lens, 3),
        }
        return Image(data, meta=meta)

    def _render_angle(self, bpy, name, shading, long_edge):
        scene = bpy.context.scene
        center, radius = _scene_bounds(bpy)
        direction = ANGLES[name]
        length = math.sqrt(sum(c * c for c in direction)) or 1.0
        distance = radius * 2.6 + 0.5
        location = tuple(center[i] + direction[i] / length * distance for i in range(3))
        camera_data = bpy.data.cameras.new(f"__locus_look_{name}")
        camera_data.lens = 50
        camera_data.clip_end = max(1000.0, distance * 4)
        camera = bpy.data.objects.new(f"__locus_look_{name}", camera_data)
        scene.collection.objects.link(camera)
        try:
            import mathutils

            camera.location = location
            forward = mathutils.Vector(center) - mathutils.Vector(location)
            camera.rotation_euler = forward.to_track_quat("-Z", "Y").to_euler()
            return self._render_camera(bpy, camera, shading, long_edge, {"mode": "angles", "view": name})
        finally:
            bpy.data.objects.remove(camera, do_unlink=True)
            bpy.data.cameras.remove(camera_data, do_unlink=True)

    def _render_camera(self, bpy, camera, shading, long_edge, meta):
        scene = bpy.context.scene
        camera = camera or scene.camera
        if camera is None:
            raise HelperError("the scene has no camera; use look(mode='angles') or add a camera")
        render = scene.render
        saved = {
            "camera": scene.camera,
            "engine": render.engine,
            "resolution_x": render.resolution_x,
            "resolution_y": render.resolution_y,
            "resolution_percentage": render.resolution_percentage,
            "filepath": render.filepath,
            "file_format": render.image_settings.file_format,
            "color_mode": render.image_settings.color_mode,
            "use_file_extension": render.use_file_extension,
            "shading_type": scene.display.shading.type,
            "light": scene.display.shading.light,
            "color_type": scene.display.shading.color_type,
        }
        width, height = saved["resolution_x"], saved["resolution_y"]
        scale = long_edge / max(width, height, 1)
        temp_dir = tempfile.mkdtemp(prefix="locus_look_")
        path = os.path.join(temp_dir, "render.png")
        try:
            scene.camera = camera
            if shading != "rendered":
                render.engine = "BLENDER_WORKBENCH"
                display = scene.display.shading
                display.light = "STUDIO"
                display.color_type = "MATERIAL" if shading in (None, "material") else "OBJECT"
                display.type = "WIREFRAME" if shading == "wireframe" else "SOLID"
            render.resolution_x = max(16, int(round(width * scale)))
            render.resolution_y = max(16, int(round(height * scale)))
            render.resolution_percentage = 100
            render.filepath = path
            render.use_file_extension = False
            render.image_settings.file_format = "PNG"
            render.image_settings.color_mode = "RGBA"
            bpy.ops.render.render(write_still=True)
            with open(path, "rb") as handle:
                data = handle.read()
        finally:
            scene.camera = saved["camera"]
            render.engine = saved["engine"]
            render.resolution_x = saved["resolution_x"]
            render.resolution_y = saved["resolution_y"]
            render.resolution_percentage = saved["resolution_percentage"]
            render.filepath = saved["filepath"]
            render.use_file_extension = saved["use_file_extension"]
            render.image_settings.file_format = saved["file_format"]
            render.image_settings.color_mode = saved["color_mode"]
            display = scene.display.shading
            display.type = saved["shading_type"]
            display.light = saved["light"]
            display.color_type = saved["color_type"]
            _remove_tree(temp_dir)
        meta = dict(meta)
        meta.update({
            "camera": camera.name if not camera.name.startswith("__locus_look_") else None,
            "camera_matrix": [list(_round(row)) for row in camera.matrix_world],
            "lens": round(getattr(camera.data, "lens", 0.0), 3),
            "shading": shading or "material",
            "frame": meta.get("frame", scene.frame_current),
        })
        return Image(data, meta=meta)

    # ── inspection ─────────────────────────────────────────────────────────
    def scene_summary(self, fields=None, query=None, root=None, limit=20):
        bpy = _bpy()
        scene = bpy.context.scene
        objects = list(scene.objects)
        counts = {}
        for obj in objects:
            counts[obj.type] = counts.get(obj.type, 0) + 1
        type_text = ", ".join(f"{kind.lower()} {count}" for kind, count in sorted(counts.items()))
        lines = [
            f"Scene {scene.name!r}: {len(objects)} objects ({type_text or 'none'}); frame {scene.frame_current} "
            f"({scene.frame_start}-{scene.frame_end}); engine {scene.render.engine}; "
            f"unit {scene.unit_settings.system.lower()} x{scene.unit_settings.scale_length:g}; "
            f"file {bpy.data.filepath or '(unsaved)'}{' *' if bpy.data.is_dirty else ''}"
        ]
        selected = objects
        if root:
            base = bpy.data.objects.get(root)
            if base is None:
                raise HelperError(f"object {root!r} not found")
            selected = [base, *base.children_recursive]
        if query:
            needle = str(query).lower()
            selected = [obj for obj in selected if needle in obj.name.lower() or needle == obj.type.lower()]
        fields = list(fields or ("type", "location", "parent", "visible", "data", "materials", "modifiers"))
        for obj in selected[: max(0, int(limit))]:
            parts = [obj.name]
            for field in fields:
                if field == "type":
                    parts.append(obj.type.lower())
                elif field == "location":
                    parts.append("loc " + ",".join(f"{v:g}" for v in _round(obj.location, 3)))
                elif field == "rotation":
                    parts.append("rot " + ",".join(f"{math.degrees(v):.1f}" for v in obj.rotation_euler))
                elif field == "scale":
                    parts.append("scale " + ",".join(f"{v:g}" for v in _round(obj.scale, 3)))
                elif field == "dimensions":
                    parts.append("dim " + ",".join(f"{v:g}" for v in _round(obj.dimensions, 3)))
                elif field == "parent" and obj.parent is not None:
                    parts.append(f"parent {obj.parent.name}")
                elif field == "visible":
                    parts.append("visible" if obj.visible_get() else "hidden")
                elif field == "data" and obj.data is not None:
                    data = obj.data
                    if obj.type == "MESH":
                        parts.append(f"mesh {data.name} v{len(data.vertices)} f{len(data.polygons)}")
                    else:
                        parts.append(f"data {data.name}")
                elif field == "materials":
                    names = [slot.material.name for slot in obj.material_slots if slot.material]
                    if names:
                        parts.append("mat " + ",".join(names))
                elif field == "modifiers" and len(obj.modifiers):
                    parts.append("mod " + ",".join(f"{m.name}:{m.type.lower()}" for m in obj.modifiers))
                elif field == "collections":
                    parts.append("coll " + ",".join(c.name for c in obj.users_collection))
            lines.append(" | ".join(parts))
        if len(selected) > limit:
            lines.append(f"… {len(selected) - limit} more (raise limit or narrow query/root)")
        return "\n".join(lines)

    def object_detail(self, name):
        bpy = _bpy()
        obj = bpy.data.objects.get(name)
        if obj is None:
            raise HelperError(f"object {name!r} not found")
        detail = {
            "name": obj.name,
            "type": obj.type,
            "data": obj.data.name if obj.data is not None else None,
            "location": _round(obj.location),
            "rotation_euler_deg": tuple(round(math.degrees(v), 3) for v in obj.rotation_euler),
            "rotation_mode": obj.rotation_mode,
            "scale": _round(obj.scale),
            "dimensions": _round(obj.dimensions),
            "parent": obj.parent.name if obj.parent else None,
            "parent_type": obj.parent_type if obj.parent else None,
            "children": [child.name for child in obj.children],
            "collections": [c.name for c in obj.users_collection],
            "visible": obj.visible_get(),
            "hide_render": obj.hide_render,
            "modifiers": [{"name": m.name, "type": m.type, "show_viewport": m.show_viewport} for m in obj.modifiers],
            "constraints": [{"name": c.name, "type": c.type} for c in obj.constraints],
            "materials": [slot.material.name if slot.material else None for slot in obj.material_slots],
            "custom_properties": [key for key in obj.keys() if not key.startswith("_")],
        }
        if obj.type == "MESH":
            mesh = obj.data
            detail["mesh"] = {
                "vertices": len(mesh.vertices),
                "edges": len(mesh.edges),
                "faces": len(mesh.polygons),
                "uv_layers": [uv.name for uv in mesh.uv_layers],
                "shape_keys": [k.name for k in mesh.shape_keys.key_blocks] if mesh.shape_keys else [],
            }
        if obj.animation_data and obj.animation_data.action:
            detail["action"] = obj.animation_data.action.name
        return detail

    def file_report(self):
        bpy = _bpy()
        counts = {}
        for attr in ("objects", "meshes", "materials", "images", "textures", "node_groups", "actions",
                     "armatures", "cameras", "lights", "collections", "scenes", "libraries", "worlds"):
            collection = getattr(bpy.data, attr, None)
            if collection is not None and len(collection):
                counts[attr] = len(collection)
        missing = []
        for image in bpy.data.images:
            if image.source in ("FILE", "SEQUENCE", "TILED") and image.packed_file is None and image.filepath:
                path = bpy.path.abspath(image.filepath, library=image.library)
                if not os.path.exists(path):
                    missing.append({"type": "image", "name": image.name, "path": image.filepath})
        libraries = []
        for library in bpy.data.libraries:
            path = bpy.path.abspath(library.filepath)
            libraries.append({
                "name": library.name,
                "path": library.filepath,
                "exists": os.path.exists(path),
                "parent": library.parent.name if library.parent else None,
            })
            if not os.path.exists(path):
                missing.append({"type": "library", "name": library.name, "path": library.filepath})
        return {
            "filepath": bpy.data.filepath or None,
            "dirty": bpy.data.is_dirty,
            "saved_version": ".".join(str(v) for v in bpy.data.version),
            "blender_version": bpy.app.version_string,
            "counts": counts,
            "missing": missing,
            "libraries": libraries,
        }

    def api(self, identifier):
        bpy = _bpy()
        text = str(identifier).strip()
        if text.startswith("bpy.ops."):
            parts = text[len("bpy.ops."):].split(".")
            if len(parts) != 2:
                raise HelperError("operators look like bpy.ops.<module>.<name>")
            operator = getattr(getattr(bpy.ops, parts[0]), parts[1])
            rna = operator.get_rna_type()
            lines = [f"{text}: {rna.description}"]
            lines += [_describe_property(prop) for prop in rna.properties if prop.identifier != "rna_type"]
            return "\n".join(lines)
        if text.startswith("bpy.types."):
            text = text[len("bpy.types."):]
        type_name, _, prop_name = text.partition(".")
        cls = getattr(bpy.types, type_name, None)
        if cls is None:
            raise HelperError(f"bpy.types has no {type_name!r}")
        rna = cls.bl_rna
        if prop_name:
            prop = rna.properties.get(prop_name)
            if prop is not None:
                return _describe_property(prop)
            function = rna.functions.get(prop_name)
            if function is None:
                raise HelperError(f"{type_name} has no property or function {prop_name!r}")
            params = ", ".join(p.identifier for p in function.parameters if not p.is_output)
            outputs = ", ".join(p.identifier for p in function.parameters if p.is_output)
            return f"{type_name}.{function.identifier}({params}){' -> ' + outputs if outputs else ''}: {function.description}"
        base = rna.base.identifier if rna.base else None
        lines = [f"{type_name}{f'({base})' if base else ''}: {rna.description}"]
        lines += [_describe_property(prop) for prop in rna.properties if prop.identifier != "rna_type"]
        functions = [f.identifier for f in rna.functions]
        if functions:
            lines.append("functions: " + ", ".join(functions))
        return "\n".join(lines)

    # ── mutating ───────────────────────────────────────────────────────────
    def focus(self, name):
        self._require_write("focus")
        bpy = _bpy()
        obj = bpy.data.objects.get(name)
        if obj is None:
            raise HelperError(f"object {name!r} not found")
        for other in bpy.context.view_layer.objects:
            other.select_set(False)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        with self.view3d():
            bpy.ops.view3d.view_selected()
        return obj.name

    def render_thumbnail(self, path, max_long_edge=256):
        self._require_write("render_thumbnail")
        bpy = _bpy()
        image = self._render_camera(bpy, None, None, int(max_long_edge), {"mode": "thumbnail"})
        return image.save(os.path.abspath(path))

    def _require_write(self, name):
        if self._readonly:
            raise HelperError(f"{name}() changes the scene; run with readonly=False")


def _describe_property(prop):
    kind = prop.type.lower()
    extra = ""
    if prop.type == "ENUM":
        items = [item.identifier for item in prop.enum_items]
        extra = f" {{{', '.join(items[:24])}{', …' if len(items) > 24 else ''}}}"
    elif prop.type == "POINTER" and getattr(prop, "fixed_type", None) is not None:
        extra = f" -> {prop.fixed_type.identifier}"
    elif prop.type == "COLLECTION" and getattr(prop, "fixed_type", None) is not None:
        extra = f" of {prop.fixed_type.identifier}"
    elif getattr(prop, "array_length", 0):
        extra = f"[{prop.array_length}]"
    readonly = " (read-only)" if prop.is_readonly else ""
    return f"  {prop.identifier}: {kind}{extra}{readonly} — {prop.description}"


def _scene_bounds(bpy):
    import mathutils

    points = []
    for obj in bpy.context.scene.objects:
        if obj.type in ("CAMERA", "LIGHT") or obj.name.startswith("__locus_look_") or not obj.visible_get():
            continue
        points.extend(obj.matrix_world @ mathutils.Vector(corner) for corner in obj.bound_box)
    if not points:
        return (0.0, 0.0, 0.0), 1.0
    low = [min(p[i] for p in points) for i in range(3)]
    high = [max(p[i] for p in points) for i in range(3)]
    center = tuple((low[i] + high[i]) / 2 for i in range(3))
    radius = max(0.5, math.sqrt(sum((high[i] - low[i]) ** 2 for i in range(3))) / 2)
    return center, radius


def _scaled_png(bpy, path, long_edge):
    image = bpy.data.images.load(path, check_existing=False)
    try:
        width, height = image.size
        scale = long_edge / max(width, height, 1)
        if scale < 1:
            image.scale(max(1, int(width * scale)), max(1, int(height * scale)))
            out = path + ".scaled.png"
            image.filepath_raw = out
            image.file_format = "PNG"
            image.save()
            path = out
        with open(path, "rb") as handle:
            return handle.read()
    finally:
        bpy.data.images.remove(image)


def _remove_tree(directory):
    try:
        for name in os.listdir(directory):
            try:
                os.unlink(os.path.join(directory, name))
            except OSError:
                pass
        os.rmdir(directory)
    except OSError:
        pass
