"""Describe this Blender installation for Locus's installation table.

Run as `blender -b --factory-startup --python probe.py`; prints one line
`LOCUS_BLENDER_PROBE {json}`. Blender reports its own directories, so
custom builds with non-standard config folder names work too.
"""

import json
import os
import sys

import bpy


def _text(value):
    return value.decode("utf-8", "replace") if isinstance(value, bytes) else str(value)


def _is_portable(user, binary):
    """Portable builds keep the user resources next to the executable:
    `<exe dir>/portable` since 4.2, `<exe dir>/<version>/config` before."""
    if not user or not binary:
        return False
    user = os.path.normcase(os.path.abspath(user))
    root = os.path.normcase(os.path.abspath(os.path.dirname(binary)))
    try:
        return os.path.commonpath([user, root]) == root
    except ValueError:  # different drives
        return False


def main():
    scripts = bpy.utils.user_resource("SCRIPTS")
    config = bpy.utils.user_resource("CONFIG")
    resource_types = {}
    for name in ("LOCAL", "USER", "SYSTEM"):
        try:
            resource_types[name.lower()] = bpy.utils.resource_path(name)
        except Exception:
            resource_types[name.lower()] = None
    info = {
        "version": list(bpy.app.version),
        "version_string": bpy.app.version_string,
        "version_cycle": getattr(bpy.app, "version_cycle", None),
        "build_hash": _text(bpy.app.build_hash),
        "build_branch": _text(bpy.app.build_branch),
        "build_date": _text(bpy.app.build_date),
        "python": sys.version.split()[0],
        "user_scripts": scripts,
        "user_config": config,
        "portable": _is_portable(resource_types.get("user"), bpy.app.binary_path),
        "resources": resource_types,
        "binary": bpy.app.binary_path,
    }
    print("LOCUS_BLENDER_PROBE " + json.dumps(info, ensure_ascii=False), flush=True)


main()
