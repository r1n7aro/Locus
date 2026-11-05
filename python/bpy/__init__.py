"""Placeholder: `bpy` only exists inside a Blender process.

Locus puts this module on the Python SDK path so `import bpy` in the
`python` tool fails with directions instead of a bare ModuleNotFoundError.
Inside Blender the real built-in `bpy` always wins (the SDK path is appended
after Blender's own).
"""

raise ImportError(
    "bpy is only available inside a Blender process: use the blender_execute tool, "
    "or from Python `await locus.blender.execute(code)` / `@locus.blender.function`"
)
