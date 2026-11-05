"""Locus host for Blender.

Installed into `<user scripts>/startup/locus_host/`, Blender imports this
package at startup and calls `register()`. Background instances launched by
Locus run `bootstrap.py` from the Locus resources instead (no install
needed). Both paths start the same host once per process.
"""


def register():
    from . import host

    host.start()


def unregister():
    from . import host

    host.stop("unregister")


def start_from_bootstrap():
    """`blender [-b] --python bootstrap.py`: start; serve when in background."""
    import bpy

    from . import host

    if host.start() and bpy.app.background:
        host.serve_forever()
