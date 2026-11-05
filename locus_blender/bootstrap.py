"""Start the Locus host from `--python` without installing the add-on.

Locus launches background instances as
`blender -b --factory-startup [file] --python <resources>/locus_blender/bootstrap.py`.
The process serves requests until Locus sends `shutdown_host`, then exits.
"""

import os
import sys

_here = os.path.dirname(os.path.abspath(__file__))
if _here not in sys.path:
    sys.path.insert(0, _here)

import locus_host  # noqa: E402

locus_host.start_from_bootstrap()
