"""Emit a JSON structure report for an RDC using qrenderdoc's embedded Python."""

import json
import os
import sys
import traceback

import renderdoc as rd


def required_env(name):
    value = os.environ.get(name, "").strip()
    if not value:
        raise RuntimeError("Missing required environment variable: " + name)
    return os.path.abspath(value)


def resource_id(value):
    return str(value) if value != rd.ResourceId.Null() else None


def enum_text(value):
    try:
        return str(value)
    except Exception:
        return repr(value)


def action_name(action, structured_file):
    try:
        return action.GetName(structured_file)
    except Exception:
        return "event {}".format(int(action.eventId))


def action_resource(value):
    return resource_id(value) if value is not None else None


def descriptor_resource(descriptor):
    if hasattr(descriptor, "resource"):
        return descriptor.resource
    return getattr(descriptor, "resourceId", rd.ResourceId.Null())


def flatten_actions(roots, structured_file):
    rows = []

    def visit(action, depth, parent_event):
        row = {
            "eventId": int(action.eventId),
            "actionId": int(action.actionId),
            "parentEventId": parent_event,
            "depth": depth,
            "name": action_name(action, structured_file),
            "flags": enum_text(action.flags),
            "numIndices": int(getattr(action, "numIndices", 0)),
            "numInstances": int(getattr(action, "numInstances", 0)),
            "dispatchDimension": [
                int(getattr(action, "dispatchDimension", [0, 0, 0])[i])
                for i in range(3)
            ],
            "outputs": [
                rid
                for rid in (action_resource(item) for item in getattr(action, "outputs", []))
                if rid is not None
            ],
            "depthOutput": action_resource(getattr(action, "depthOut", None)),
            "copySource": action_resource(getattr(action, "copySource", None)),
            "copyDestination": action_resource(getattr(action, "copyDestination", None)),
            "childCount": len(action.children),
        }
        rows.append(row)
        for child in action.children:
            visit(child, depth + 1, int(action.eventId))

    for root in roots:
        visit(root, 0, None)
    return rows


def describe_format(fmt):
    try:
        return fmt.Name()
    except Exception:
        return enum_text(fmt)


def describe_texture(texture, names):
    rid = resource_id(texture.resourceId)
    return {
        "resourceId": rid,
        "name": names.get(rid, ""),
        "dimension": enum_text(getattr(texture, "dimension", "")),
        "width": int(texture.width),
        "height": int(texture.height),
        "depth": int(texture.depth),
        "arraySize": int(texture.arraysize),
        "mips": int(texture.mips),
        "samples": int(texture.msSamp),
        "format": describe_format(texture.format),
        "creationFlags": enum_text(getattr(texture, "creationFlags", "")),
    }


def pipeline_snapshot(controller, event_id):
    controller.SetFrameEvent(event_id, True)
    pipeline = controller.GetPipelineState()
    colors = [
        resource_id(descriptor_resource(target))
        for target in pipeline.GetOutputTargets()
        if descriptor_resource(target) != rd.ResourceId.Null()
    ]
    depth = pipeline.GetDepthTarget()
    return {
        "eventId": event_id,
        "colorOutputs": colors,
        "depthOutput": resource_id(descriptor_resource(depth)),
    }


def main():
    capture_path = required_env("LOCUS_RENDERDOC_CAPTURE")
    report_path = os.environ.get("LOCUS_RENDERDOC_REPORT", "").strip()
    report_path = os.path.abspath(report_path) if report_path else capture_path + ".json"
    include_pipeline = os.environ.get("LOCUS_RENDERDOC_PIPELINE_SNAPSHOTS", "0") == "1"
    report = {
        "ok": False,
        "capturePath": capture_path,
        "reportPath": report_path,
        "python": sys.version,
    }

    capture = rd.OpenCaptureFile()
    controller = None
    try:
        open_result = capture.OpenFile(capture_path, "", None)
        report["openResult"] = enum_text(open_result)
        if open_result != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenFile failed: " + enum_text(open_result))

        report["localReplaySupport"] = bool(capture.LocalReplaySupport())
        replay_result, controller = capture.OpenCapture(rd.ReplayOptions(), None)
        report["replayResult"] = enum_text(replay_result)
        if replay_result != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenCapture failed: " + enum_text(replay_result))

        structured_file = controller.GetStructuredFile()
        actions = flatten_actions(controller.GetRootActions(), structured_file)
        resources = controller.GetResources()
        names = {resource_id(item.resourceId): item.name for item in resources}
        api = controller.GetAPIProperties()

        report["pipelineType"] = enum_text(api.pipelineType)
        report["actionCount"] = len(actions)
        report["resourceCount"] = len(resources)
        report["textureCount"] = len(controller.GetTextures())
        report["actions"] = actions
        report["resources"] = [
            {
                "resourceId": resource_id(item.resourceId),
                "name": item.name,
                "type": enum_text(item.type),
                "autogeneratedName": bool(item.autogeneratedName),
            }
            for item in resources
        ]
        report["textures"] = [
            describe_texture(texture, names) for texture in controller.GetTextures()
        ]

        if include_pipeline:
            report["pipelineSnapshots"] = [
                pipeline_snapshot(controller, row["eventId"])
                for row in actions
                if row["eventId"] > 0 and row["childCount"] == 0
            ]

        report["ok"] = True
    except Exception as error:
        report["error"] = str(error)
        report["traceback"] = traceback.format_exc()
    finally:
        if controller is not None:
            controller.Shutdown()
        capture.Shutdown()
        os.makedirs(os.path.dirname(report_path), exist_ok=True)
        with open(report_path, "w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2)

    print(json.dumps({"ok": report["ok"], "reportPath": report_path}, ensure_ascii=False))
    return 0 if report["ok"] else 1


raise SystemExit(main())
