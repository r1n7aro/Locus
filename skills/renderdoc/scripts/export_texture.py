"""Export one texture at one event using qrenderdoc's embedded Python."""

import json
import os
import traceback

import renderdoc as rd


FILE_TYPES = {
    ".bmp": rd.FileType.BMP,
    ".dds": rd.FileType.DDS,
    ".exr": rd.FileType.EXR,
    ".hdr": rd.FileType.HDR,
    ".jpg": rd.FileType.JPG,
    ".jpeg": rd.FileType.JPG,
    ".png": rd.FileType.PNG,
    ".tga": rd.FileType.TGA,
}


def required_env(name):
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError("Missing required environment variable: " + name)
    return value


def rid_text(value):
    return str(value)


def descriptor_resource(descriptor):
    if hasattr(descriptor, "resource"):
        return descriptor.resource
    return getattr(descriptor, "resourceId", rd.ResourceId.Null())


def choose_resource(controller, selector):
    pipeline = controller.GetPipelineState()
    normalized = selector.strip().lower()
    if normalized == "depth":
        resource = descriptor_resource(pipeline.GetDepthTarget())
        if resource == rd.ResourceId.Null():
            raise RuntimeError("The selected event has no depth output")
        return resource
    if normalized.startswith("color:"):
        index = int(normalized.split(":", 1)[1])
        outputs = pipeline.GetOutputTargets()
        if index < 0 or index >= len(outputs):
            raise RuntimeError("Color output index is out of range: {}".format(index))
        resource = descriptor_resource(outputs[index])
        if resource == rd.ResourceId.Null():
            raise RuntimeError("The selected color output is null")
        return resource

    for item in controller.GetResources():
        if rid_text(item.resourceId).lower() == normalized or item.name == selector:
            return item.resourceId
    raise RuntimeError("Resource was not found: " + selector)


def main():
    capture_path = os.path.abspath(required_env("LOCUS_RENDERDOC_CAPTURE"))
    output_path = os.path.abspath(required_env("LOCUS_RENDERDOC_TEXTURE_OUTPUT"))
    event_id = int(required_env("LOCUS_RENDERDOC_EVENT"))
    selector = os.environ.get("LOCUS_RENDERDOC_RESOURCE", "color:0").strip() or "color:0"
    report_path = os.environ.get("LOCUS_RENDERDOC_EXPORT_REPORT", "").strip()
    report_path = os.path.abspath(report_path) if report_path else output_path + ".json"
    extension = os.path.splitext(output_path)[1].lower()
    if extension not in FILE_TYPES:
        raise RuntimeError("Unsupported texture output extension: " + extension)

    result = {
        "ok": False,
        "capturePath": capture_path,
        "eventId": event_id,
        "selector": selector,
        "outputPath": output_path,
    }
    capture = rd.OpenCaptureFile()
    controller = None
    try:
        open_result = capture.OpenFile(capture_path, "", None)
        if open_result != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenFile failed: " + str(open_result))
        replay_result, controller = capture.OpenCapture(rd.ReplayOptions(), None)
        if replay_result != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenCapture failed: " + str(replay_result))

        controller.SetFrameEvent(event_id, True)
        resource = choose_resource(controller, selector)
        save = rd.TextureSave()
        save.resourceId = resource
        save.destType = FILE_TYPES[extension]
        save.mip = int(os.environ.get("LOCUS_RENDERDOC_MIP", "0"))
        save.slice.sliceIndex = int(os.environ.get("LOCUS_RENDERDOC_SLICE", "0"))
        save.alpha = rd.AlphaMapping.Preserve

        os.makedirs(os.path.dirname(output_path), exist_ok=True)
        save_result = controller.SaveTexture(save, output_path)
        result["saveResult"] = str(save_result)
        result["resourceId"] = rid_text(resource)
        result["bytes"] = os.path.getsize(output_path) if os.path.isfile(output_path) else 0
        result["ok"] = result["bytes"] > 0
        if not result["ok"]:
            raise RuntimeError("SaveTexture did not produce an output file")
    except Exception as error:
        result["error"] = str(error)
        result["traceback"] = traceback.format_exc()
    finally:
        if controller is not None:
            controller.Shutdown()
        capture.Shutdown()

    os.makedirs(os.path.dirname(report_path), exist_ok=True)
    with open(report_path, "w", encoding="utf-8") as handle:
        json.dump(result, handle, ensure_ascii=False, indent=2)

    print(json.dumps({"ok": result["ok"], "reportPath": report_path}, ensure_ascii=False))
    return 0 if result["ok"] else 1


raise SystemExit(main())
