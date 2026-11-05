"""State that must survive a hot reload of the `locus_host` package.

`reload_host` drops every `locus_host*` module and imports the new code. The
broker DLL stays loaded (its pipe and queue keep running), so the handle to
it, the instance identity and the registered callbacks live in a separate
module object that is never reloaded.
"""

import sys
import types

_NAME = "_locus_host_persist"


def state():
    module = sys.modules.get(_NAME)
    if module is None:
        module = types.ModuleType(_NAME)
        module.native = None  # native.Native bound to the loaded DLL
        module.broker_live = False  # the DLL broker is listening (init_host ran, no shutdown)
        module.instance = None  # dict: key, pipe, token, registration path…
        module.started = False
        module.generation = 0
        module.timer = None  # interactive tick function registered with bpy
        module.handlers = []  # (handler list name, function)
        module.mode = None  # "interactive" | "background"
        module.stop_requested = False
        module.atexit_registered = False
        sys.modules[_NAME] = module
    return module
