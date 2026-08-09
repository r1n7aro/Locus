"""Python-first RenderDoc replay helpers for Locus.

Skill activation makes this module importable from Locus's Python runtime. It
uses the bundled qrenderdoc Python host for replay when necessary. Public methods
return ordinary Python values; buffer payloads remain ``bytes`` so callers can
use ``struct``, ``memoryview``, or ``numpy.frombuffer`` directly.
"""

import os
import pickle
import subprocess
import struct
import tempfile
import time

try:
    import renderdoc as rd
except ImportError:
    rd = None


def _enum(value):
    try:
        return str(value)
    except Exception:
        return repr(value)


def _rid(value):
    if value is None or value == rd.ResourceId.Null():
        return None
    return str(value)


def _descriptor_resource(descriptor):
    if descriptor is None:
        return rd.ResourceId.Null()
    if hasattr(descriptor, "resource"):
        return descriptor.resource
    return getattr(descriptor, "resourceId", rd.ResourceId.Null())


def _int(value, default=0):
    try:
        return int(value)
    except Exception:
        return default


def _format_name(value):
    try:
        return value.Name()
    except Exception:
        return _enum(value)


class Capture(object):
    """An opened RDC and its replay controller."""

    def __init__(self, capture_path, replay_options=None):
        self.capture_path = os.path.abspath(capture_path)
        self._remote = rd is None
        if self._remote:
            self.capture_file = None
            self.controller = None
            self.open_result = None
            self.replay_result = None
            self._closed = False
            return
        self.capture_file = rd.OpenCaptureFile()
        self.controller = None
        self.open_result = None
        self.replay_result = None
        self._closed = False

        self.open_result = self.capture_file.OpenFile(self.capture_path, "", None)
        if self.open_result != rd.ResultCode.Succeeded:
            self.capture_file.Shutdown()
            self._closed = True
            raise RuntimeError("OpenFile failed: " + _enum(self.open_result))

        options = replay_options if replay_options is not None else rd.ReplayOptions()
        self.replay_result, self.controller = self.capture_file.OpenCapture(options, None)
        if self.replay_result != rd.ResultCode.Succeeded:
            self.capture_file.Shutdown()
            self._closed = True
            raise RuntimeError("OpenCapture failed: " + _enum(self.replay_result))

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self.close()
        return False

    def close(self):
        if self._closed:
            return
        if self._remote:
            self._closed = True
            return
        if self.controller is not None:
            self.controller.Shutdown()
            self.controller = None
        self.capture_file.Shutdown()
        self._closed = True

    def info(self):
        if self._remote:
            return _invoke_worker("info", {"capture_path": self.capture_path})
        api = self.controller.GetAPIProperties()
        return {
            "capturePath": self.capture_path,
            "openResult": _enum(self.open_result),
            "replayResult": _enum(self.replay_result),
            "pipelineType": _enum(api.pipelineType),
            "localReplaySupport": bool(self.capture_file.LocalReplaySupport()),
        }

    def set_event(self, event_id, force=True):
        if self._remote:
            raise RuntimeError(
                "PipelineState objects only exist inside qrenderdoc; use the data-returning facade methods"
            )
        event_id = int(event_id)
        self.controller.SetFrameEvent(event_id, bool(force))
        return self.controller.GetPipelineState()

    def actions(self):
        if self._remote:
            return _invoke_worker("actions", {"capture_path": self.capture_path})
        structured_file = self.controller.GetStructuredFile()
        rows = []

        def visit(action, depth, parent_event):
            try:
                name = action.GetName(structured_file)
            except Exception:
                name = "event {}".format(int(action.eventId))
            dimensions = getattr(action, "dispatchDimension", [0, 0, 0])
            row = {
                "eventId": int(action.eventId),
                "actionId": int(action.actionId),
                "parentEventId": parent_event,
                "depth": depth,
                "name": name,
                "flags": _enum(action.flags),
                "numIndices": _int(getattr(action, "numIndices", 0)),
                "numInstances": _int(getattr(action, "numInstances", 0)),
                "dispatchDimension": [_int(dimensions[i]) for i in range(3)],
                "outputs": [
                    rid for rid in (_rid(value) for value in getattr(action, "outputs", []))
                    if rid is not None
                ],
                "depthOutput": _rid(getattr(action, "depthOut", None)),
                "copySource": _rid(getattr(action, "copySource", None)),
                "copyDestination": _rid(getattr(action, "copyDestination", None)),
                "childCount": len(action.children),
            }
            rows.append(row)
            for child in action.children:
                visit(child, depth + 1, int(action.eventId))

        for root in self.controller.GetRootActions():
            visit(root, 0, None)
        return rows

    def resources(self):
        if self._remote:
            return _invoke_worker("resources", {"capture_path": self.capture_path})
        return [
            {
                "resourceId": _rid(item.resourceId),
                "name": item.name,
                "type": _enum(item.type),
                "autogeneratedName": bool(item.autogeneratedName),
            }
            for item in self.controller.GetResources()
        ]

    def textures(self):
        if self._remote:
            return _invoke_worker("textures", {"capture_path": self.capture_path})
        names = self._resource_names()
        rows = []
        for texture in self.controller.GetTextures():
            resource_id = _rid(texture.resourceId)
            rows.append({
                "resourceId": resource_id,
                "name": names.get(resource_id, ""),
                "dimension": _enum(getattr(texture, "dimension", "")),
                "width": _int(texture.width),
                "height": _int(texture.height),
                "depth": _int(texture.depth),
                "arraySize": _int(texture.arraysize),
                "mips": _int(texture.mips),
                "samples": _int(texture.msSamp),
                "format": _format_name(texture.format),
                "creationFlags": _enum(getattr(texture, "creationFlags", "")),
            })
        return rows

    def buffers(self):
        if self._remote:
            return _invoke_worker("buffers", {"capture_path": self.capture_path})
        names = self._resource_names()
        rows = []
        for buffer in self.controller.GetBuffers():
            resource_id = _rid(buffer.resourceId)
            rows.append({
                "resourceId": resource_id,
                "name": names.get(resource_id, ""),
                "length": _int(getattr(buffer, "length", 0)),
                "gpuAddress": _int(getattr(buffer, "gpuAddress", 0)),
                "creationFlags": _enum(getattr(buffer, "creationFlags", "")),
            })
        return rows

    def compute_bindings(self, event_id):
        if self._remote:
            return _invoke_worker(
                "compute_bindings",
                {"capture_path": self.capture_path, "event_id": int(event_id)},
            )
        pipeline = self.set_event(event_id)
        return {
            "eventId": int(event_id),
            "readOnly": self._binding_rows(
                pipeline.GetReadOnlyResources(rd.ShaderStage.Compute)
            ),
            "readWrite": self._binding_rows(
                pipeline.GetReadWriteResources(rd.ShaderStage.Compute)
            ),
        }

    def buffer_data(self, event_id, resource, offset=0, length=None):
        """Return exact buffer bytes at ``event_id`` plus ordinary metadata.

        ``resource`` accepts an rd.ResourceId, ``ResourceId::N`` text, exact
        resource name, a resource dictionary, or ``compute-readonly:N`` /
        ``compute-readwrite:N`` for flattened Compute-stage bindings.
        """
        if self._remote:
            return _invoke_worker(
                "buffer_data",
                {
                    "capture_path": self.capture_path,
                    "event_id": int(event_id),
                    "resource": resource,
                    "offset": int(offset),
                    "length": length,
                },
            )
        event_id = int(event_id)
        offset = int(offset)
        if offset < 0:
            raise ValueError("offset must be non-negative")
        self.set_event(event_id)
        resource_id = self.resolve_resource(resource, event_id)
        description = self._buffer_description(resource_id)
        available = max(0, description["length"] - offset)
        requested = available if length is None or int(length) == 0 else int(length)
        if requested < 0:
            raise ValueError("length must be non-negative")
        if description["length"] > 0:
            requested = min(requested, available)
        raw = self.controller.GetBufferData(resource_id, offset, requested)
        data = bytes(raw)
        return {
            "capturePath": self.capture_path,
            "eventId": event_id,
            "resourceId": _rid(resource_id),
            "name": description["name"],
            "buffer": description,
            "offset": offset,
            "requestedLength": requested,
            "byteLength": len(data),
            "data": data,
        }

    def save_texture(self, event_id, resource, output_path, mip=0, slice_index=0):
        if self._remote:
            return _invoke_worker(
                "save_texture",
                {
                    "capture_path": self.capture_path,
                    "event_id": int(event_id),
                    "resource": resource,
                    "output_path": os.path.abspath(output_path),
                    "mip": int(mip),
                    "slice_index": int(slice_index),
                },
            )
        event_id = int(event_id)
        output_path = os.path.abspath(output_path)
        self.set_event(event_id)
        resource_id = self.resolve_resource(resource, event_id)
        extension = os.path.splitext(output_path)[1].lower()
        file_types = {
            ".bmp": rd.FileType.BMP,
            ".dds": rd.FileType.DDS,
            ".exr": rd.FileType.EXR,
            ".hdr": rd.FileType.HDR,
            ".jpg": rd.FileType.JPG,
            ".jpeg": rd.FileType.JPG,
            ".png": rd.FileType.PNG,
            ".tga": rd.FileType.TGA,
        }
        if extension not in file_types:
            raise ValueError("Unsupported texture output extension: " + extension)
        save = rd.TextureSave()
        save.resourceId = resource_id
        save.destType = file_types[extension]
        save.mip = int(mip)
        save.slice.sliceIndex = int(slice_index)
        save.alpha = rd.AlphaMapping.Preserve
        directory = os.path.dirname(output_path)
        if directory:
            os.makedirs(directory, exist_ok=True)
        save_result = self.controller.SaveTexture(save, output_path)
        return {
            "eventId": event_id,
            "resourceId": _rid(resource_id),
            "outputPath": output_path,
            "saveResult": _enum(save_result),
            "bytes": os.path.getsize(output_path) if os.path.isfile(output_path) else 0,
        }

    def resolve_resource(self, selector, event_id=None):
        if isinstance(selector, dict):
            selector = selector.get("resourceId", selector.get("resource"))
        if selector is None:
            raise ValueError("resource selector is required")
        if not isinstance(selector, (str, int)):
            for resource in self.controller.GetResources():
                if selector == resource.resourceId:
                    return resource.resourceId
        text = str(selector).strip()
        normalized = text.lower()
        if normalized == "depth":
            if event_id is None:
                raise ValueError("event_id is required for depth selectors")
            pipeline = self.set_event(event_id)
            resource_id = _descriptor_resource(pipeline.GetDepthTarget())
            if resource_id == rd.ResourceId.Null():
                raise KeyError("The selected event has no depth output")
            return resource_id
        if normalized.startswith("color:"):
            if event_id is None:
                raise ValueError("event_id is required for color selectors")
            index = int(normalized.split(":", 1)[1])
            pipeline = self.set_event(event_id)
            outputs = pipeline.GetOutputTargets()
            if index < 0 or index >= len(outputs):
                raise IndexError("Color output index is out of range: {}".format(index))
            resource_id = _descriptor_resource(outputs[index])
            if resource_id == rd.ResourceId.Null():
                raise KeyError("The selected color output is null")
            return resource_id
        if normalized.startswith("compute-readonly:") or normalized.startswith("compute-readwrite:"):
            if event_id is None:
                raise ValueError("event_id is required for Compute binding selectors")
            kind, index_text = normalized.split(":", 1)
            index = int(index_text)
            bindings = self.compute_bindings(event_id)
            rows = bindings["readOnly" if kind == "compute-readonly" else "readWrite"]
            if index < 0 or index >= len(rows):
                raise IndexError("Compute binding index is out of range: {}".format(index))
            text = rows[index]["resourceId"]
            normalized = text.lower()
        for resource in self.controller.GetResources():
            if _rid(resource.resourceId).lower() == normalized or resource.name == text:
                return resource.resourceId
        raise KeyError("Resource was not found: " + text)

    def _resource_names(self):
        return {_rid(item.resourceId): item.name for item in self.controller.GetResources()}

    def _buffer_description(self, resource_id):
        resource_text = _rid(resource_id)
        names = self._resource_names()
        for buffer in self.controller.GetBuffers():
            if buffer.resourceId == resource_id:
                return {
                    "resourceId": resource_text,
                    "name": names.get(resource_text, ""),
                    "length": _int(getattr(buffer, "length", 0)),
                    "gpuAddress": _int(getattr(buffer, "gpuAddress", 0)),
                    "creationFlags": _enum(getattr(buffer, "creationFlags", "")),
                }
        raise TypeError("Resource is not a buffer: " + resource_text)

    def _binding_rows(self, groups):
        names = self._resource_names()
        rows = []
        for bind_index, group in enumerate(groups):
            resources = getattr(group, "resources", None)
            if resources is None:
                resources = [group]
            for array_index, bound in enumerate(resources):
                resource_id = _descriptor_resource(bound)
                resource_text = _rid(resource_id)
                if resource_text is None:
                    continue
                rows.append({
                    "bindingIndex": bind_index,
                    "arrayIndex": array_index,
                    "resourceId": resource_text,
                    "name": names.get(resource_text, ""),
                    "byteOffset": _int(getattr(bound, "byteOffset", 0)),
                    "byteSize": _int(getattr(bound, "byteSize", 0)),
                })
        return rows


def open_capture(capture_path, replay_options=None):
    return Capture(capture_path, replay_options)


def inspect_capture(capture_path, include_pipeline=False):
    if rd is None:
        return _invoke_worker(
            "inspect_capture",
            {
                "capture_path": os.path.abspath(capture_path),
                "include_pipeline": bool(include_pipeline),
            },
        )
    with open_capture(capture_path) as capture:
        actions = capture.actions()
        result = capture.info()
        result.update({
            "actions": actions,
            "resources": capture.resources(),
            "textures": capture.textures(),
            "buffers": capture.buffers(),
        })
        if include_pipeline:
            snapshots = []
            for action in actions:
                if action["eventId"] <= 0 or action["childCount"] != 0:
                    continue
                pipeline = capture.set_event(action["eventId"])
                colors = [
                    _rid(_descriptor_resource(target))
                    for target in pipeline.GetOutputTargets()
                    if _descriptor_resource(target) != rd.ResourceId.Null()
                ]
                snapshots.append({
                    "eventId": action["eventId"],
                    "colorOutputs": colors,
                    "depthOutput": _rid(_descriptor_resource(pipeline.GetDepthTarget())),
                })
            result["pipelineSnapshots"] = snapshots
        return result


def unpack_buffer(buffer_or_bytes, format_string, stride=None, max_elements=None):
    """Decode buffer bytes into tuples with Python's standard ``struct`` module."""
    data = buffer_or_bytes.get("data") if isinstance(buffer_or_bytes, dict) else buffer_or_bytes
    if not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("buffer data must be bytes-like")
    decoder = struct.Struct(format_string)
    element_stride = decoder.size if stride is None else int(stride)
    if element_stride < decoder.size or element_stride <= 0:
        raise ValueError("stride must be at least the struct size")
    count = len(data) // element_stride
    if max_elements is not None:
        count = min(count, max(0, int(max_elements)))
    return [decoder.unpack_from(data, index * element_stride) for index in range(count)]


def buffers(capture_path):
    with open_capture(capture_path) as capture:
        return capture.buffers()


def compute_bindings(capture_path, event_id):
    with open_capture(capture_path) as capture:
        return capture.compute_bindings(event_id)


def buffer_data(capture_path, event_id, resource, offset=0, length=None):
    with open_capture(capture_path) as capture:
        return capture.buffer_data(event_id, resource, offset, length)


def save_texture(capture_path, event_id, resource, output_path, mip=0, slice_index=0):
    with open_capture(capture_path) as capture:
        return capture.save_texture(event_id, resource, output_path, mip, slice_index)


def _runtime_root():
    return os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "runtime", "windows-x64"))


def _invoke_worker(method, arguments, timeout_seconds=120.0):
    if rd is not None:
        return _worker_dispatch(method, arguments)
    runtime_root = _runtime_root()
    qrenderdoc = os.path.join(runtime_root, "qrenderdoc.exe")
    worker = os.path.join(os.path.dirname(__file__), "renderdoc_worker.py")
    if not os.path.isfile(qrenderdoc):
        raise RuntimeError("Bundled qrenderdoc.exe is missing: " + qrenderdoc)
    if not os.path.isfile(worker):
        raise RuntimeError("RenderDoc Python worker is missing: " + worker)

    with tempfile.TemporaryDirectory(prefix="locus-renderdoc-") as temp_dir:
        request_path = os.path.join(temp_dir, "request.pkl")
        response_path = os.path.join(temp_dir, "response.pkl")
        with open(request_path, "wb") as handle:
            pickle.dump(
                {"method": method, "arguments": arguments},
                handle,
                protocol=4,
            )
        environment = os.environ.copy()
        environment["LOCUS_RENDERDOC_REQUEST"] = request_path
        environment["LOCUS_RENDERDOC_RESPONSE"] = response_path
        environment["LOCUS_RENDERDOC_MODULE_ROOT"] = os.path.dirname(__file__)
        environment.pop("PYTHONHOME", None)
        environment.pop("PYTHONPATH", None)
        creation_flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        process = subprocess.Popen(
            [qrenderdoc, "--python=" + worker],
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=creation_flags,
        )
        deadline = time.time() + float(timeout_seconds)
        while not os.path.isfile(response_path):
            if time.time() >= deadline:
                try:
                    process.terminate()
                except Exception:
                    pass
                raise TimeoutError("RenderDoc Python worker timed out after {}s".format(timeout_seconds))
            if process.poll() is not None:
                time.sleep(0.1)
                if not os.path.isfile(response_path):
                    raise RuntimeError(
                        "RenderDoc Python worker exited with code {} before returning data".format(
                            process.returncode
                        )
                    )
            time.sleep(0.02)
        with open(response_path, "rb") as handle:
            response = pickle.load(handle)
        if not response.get("ok"):
            message = response.get("error", "RenderDoc Python worker failed")
            traceback_text = response.get("traceback")
            if traceback_text:
                message += "\n" + traceback_text
            raise RuntimeError(message)
        return response.get("value")


def _worker_dispatch(method, arguments):
    capture_path = arguments["capture_path"]
    if method == "inspect_capture":
        return inspect_capture(capture_path, arguments.get("include_pipeline", False))
    with open_capture(capture_path) as capture:
        if method == "info":
            return capture.info()
        if method == "actions":
            return capture.actions()
        if method == "resources":
            return capture.resources()
        if method == "textures":
            return capture.textures()
        if method == "buffers":
            return capture.buffers()
        if method == "compute_bindings":
            return capture.compute_bindings(arguments["event_id"])
        if method == "buffer_data":
            return capture.buffer_data(
                arguments["event_id"],
                arguments["resource"],
                arguments.get("offset", 0),
                arguments.get("length"),
            )
        if method == "save_texture":
            return capture.save_texture(
                arguments["event_id"],
                arguments["resource"],
                arguments["output_path"],
                arguments.get("mip", 0),
                arguments.get("slice_index", 0),
            )
    raise ValueError("Unsupported RenderDoc worker method: " + method)
