"""Versions the Locus host add-on reports to Locus."""

# Wire protocol between Locus and this host (envelope payload shapes).
HOST_PROTOCOL = 1
# Broker C ABI the host expects (`locus_protocol_version`).
NATIVE_PROTOCOL = 1
# Oldest supported Blender. Earlier versions skip startup entirely.
MIN_BLENDER = (4, 2, 0)
# Capabilities this host code implements, merged with the broker's.
HOST_CAPABILITIES = (
    "blender_host_v1",
    "blender_exec_v1",
    "blender_exec_function_v1",
    "blender_exec_file_v1",
    "blender_session_namespace_v1",
    "blender_undo_batch_v1",
    "blender_look_v1",
    "blender_shm_v1",
    "blender_reload_host_v1",
)
