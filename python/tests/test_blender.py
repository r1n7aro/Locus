import ast
import base64
import io
import linecache
import math
import os
import pathlib
import contextlib
import unittest
from unittest import mock

import locus
from locus import _blender
from locus._blender_codec import Array, Decoder, Encoder, Ref

SCOPE = {"checkoutId": "blend-project", "expectedGeneration": 2}
PNG = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + (4).to_bytes(4, "big") + (3).to_bytes(4, "big") + b"rest"
INSTANCE = {"key": "100-5", "pid": 100, "mode": "interactive", "version": "4.2.1", "versionString": "4.2.1 LTS",
            "file": "F:/p/hero.blend", "dirty": False, "connection": "connected"}


class FakeClient(locus.Client):
    def __init__(self, responses=None):
        super().__init__(base_url="http://127.0.0.1/sdk", token="test", current_session_id="session-b")
        self.calls = []
        self.sync_calls = []
        self.responses = list(responses or [])

    async def rpc(self, method, params=None, **kwargs):
        self.calls.append((method, params, kwargs))
        if self.responses:
            response = self.responses.pop(0)
            return response(params) if callable(response) else response
        return {}

    def _rpc_sync(self, method, params, timeout):
        self.sync_calls.append((method, params))
        return {}


def ok(value=None, **extra):
    result = {"ok": True, "value": value, "stdout": "", "stderr": "", "duration_ms": 3, "shm": []}
    result.update(extra)
    return {"ok": True, "instance": INSTANCE, "result": result, "waitedMs": 1, "elapsedMs": 4, "warnings": []}


class CodecTests(unittest.TestCase):
    def test_round_trip_keeps_container_types_and_special_floats(self):
        value = {"t": (1, 2.5), "s": {3}, "m": {1: "a"}, "f": [math.inf, -math.inf], "b": b"xy",
                 "p": pathlib.PurePosixPath("a/b"), "r": Ref("Object", "Cube", "objects")}
        encoded = Encoder().encode(value)
        self.assertEqual(encoded["t"], {"$t": [1, 2.5]})
        self.assertEqual(encoded["m"], {"$m": [[1, "a"]]})
        self.assertEqual(encoded["p"], "a/b")
        decoded = Decoder().decode(encoded)
        self.assertEqual(decoded["t"], (1, 2.5))
        self.assertEqual(decoded["s"], {3})
        self.assertEqual(decoded["m"], {1: "a"})
        self.assertEqual(decoded["f"], [math.inf, -math.inf])
        self.assertEqual(decoded["b"], b"xy")
        self.assertEqual(decoded["r"], Ref("Object", "Cube", "objects"))
        nan = Decoder().decode(Encoder().encode(float("nan")))
        self.assertTrue(math.isnan(nan))

    def test_unsupported_values_name_their_location(self):
        with self.assertRaisesRegex(TypeError, r"args\['mesh'\]\[1\]"):
            Encoder().encode({"mesh": [1, object()]}, "args")

    def test_images_decode_to_locus_image(self):
        encoded = {"$img": {"mime": "image/png", "width": 4, "height": 3, "meta": {"mode": "viewport"},
                            "data": {"b64": base64.b64encode(PNG).decode()}}}
        image = Decoder().decode(encoded)
        self.assertIsInstance(image, locus.Image)
        self.assertEqual((image.width, image.height), (4, 3))
        self.assertEqual(repr(image), "<Image 4x3 png viewport>")

    def test_array_without_numpy_converts_to_nested_lists(self):
        import struct
        array = Array(dtype="<f4", shape=(2, 2), data=struct.pack("<4f", 1, 2, 3, 4))
        self.assertEqual(array.tolist(), [[1.0, 2.0], [3.0, 4.0]])


class ExecuteTests(unittest.IsolatedAsyncioTestCase):
    async def test_execute_sends_encoded_args_and_returns_decoded_value(self):
        client = FakeClient([ok({"$t": [1, 2]}, stdout="hi\n")])
        bl = client.blender.scoped(workspace_ref=SCOPE)
        with contextlib.redirect_stdout(io.StringIO()) as out:
            result = await bl.execute("return (1, 2)", args={"size": (1, 2)}, readonly=True)
        self.assertEqual(result.value, (1, 2))
        self.assertEqual(result.instance.pid, 100)
        self.assertEqual(out.getvalue(), "hi\n")
        method, params, kwargs = client.calls[0]
        self.assertEqual(method, "blender.execute")
        self.assertEqual(params["workspaceRef"], SCOPE)
        self.assertEqual(params["sessionId"], "session-b")
        self.assertEqual(params["payload"]["args"], {"size": {"$t": [1, 2]}})
        self.assertTrue(params["payload"]["readonly"])
        self.assertEqual(params["timeoutMs"], 120000)
        self.assertGreater(kwargs["timeout"], 120)

    async def test_failures_map_to_typed_errors(self):
        cases = [
            ({"ok": False, "failure": {"kind": "target_unavailable", "candidates": [INSTANCE]}, "message": "none"},
             locus.TargetUnavailable),
            ({"ok": False, "failure": {"kind": "not_executed"}, "message": "queued"}, locus.NotExecuted),
            ({"ok": False, "failure": {"kind": "timed_out"}, "message": "late"}, locus.TimedOut),
        ]
        for body, error in cases:
            client = FakeClient([body])
            with self.assertRaises(error) as caught:
                await client.blender.scoped(workspace_ref=SCOPE).execute("x")
            if error is locus.TargetUnavailable:
                self.assertEqual(caught.exception.candidates[0].key, "100-5")

    async def test_exceptions_inside_blender_raise_blender_error(self):
        response = ok()
        response["result"].update({"ok": False, "error": {"type": "KeyError", "message": "'Cube'",
                                                          "traceback": "Traceback ..."}})
        client = FakeClient([response])
        with self.assertRaises(locus.BlenderError) as caught:
            await client.blender.scoped(workspace_ref=SCOPE).execute("bpy.data.objects['Cube']")
        self.assertEqual(caught.exception.type, "KeyError")
        self.assertIs(locus.blender.BlenderError, locus.BlenderError)

    async def test_shared_memory_results_are_released(self):
        client = FakeClient([ok({"$b": {"b64": base64.b64encode(b"x").decode()}}, shm=["Local\\a"])])
        await client.blender.scoped(workspace_ref=SCOPE).execute("return b'x'")
        self.assertEqual(client.sync_calls[0][0], "blender.release_shm")
        self.assertEqual(client.sync_calls[0][1]["names"], ["Local\\a"])

    async def test_python_readonly_forces_readonly_payloads(self):
        client = FakeClient([ok(1)])
        with mock.patch.dict(os.environ, {"LOCUS_PYTHON_READONLY": "1", "LOCUS_PYTHON_CALL": "call-1"}):
            await client.blender.scoped(workspace_ref=SCOPE).execute("return 1")
        params = client.calls[0][1]
        self.assertTrue(params["payload"]["readonly"])
        self.assertEqual(params["pythonCall"], "call-1")

    async def test_helpers_match_the_blender_host_manifest(self):
        host = pathlib.Path(__file__).resolve().parents[2] / "locus_blender" / "locus_host" / "helpers.py"
        tree = ast.parse(host.read_text(encoding="utf-8"))
        manifest = next(node.value for node in tree.body
                        if isinstance(node, ast.Assign) and getattr(node.targets[0], "id", "") == "MANIFEST")
        entries = ast.literal_eval(manifest)
        remote = {entry["name"]: entry["readonly"] for entry in entries if entry["name"] != "view3d"}
        self.assertEqual(remote, _blender._HELPERS)
        for name in remote:
            self.assertTrue(callable(getattr(_blender.Blender, name)))


def make_function(source, namespace):
    """Define functions the way the `python` tool runs scripts."""
    filename = "<locus-python>"
    linecache.cache[filename] = (len(source), None, source.splitlines(True), filename)
    exec(compile(source, filename, "exec"), namespace)
    return namespace


class FunctionTests(unittest.IsolatedAsyncioTestCase):
    async def test_function_ships_source_once_and_binds_arguments_locally(self):
        client = FakeClient([ok(6), ok(7)])
        bl = client.blender.scoped(workspace_ref=SCOPE)
        namespace = make_function(
            "async def main(bl):\n"
            "    import math\n"
            "    @bl.function\n"
            "    def area(w, h=3, *rest, scale=1, **extra):\n"
            "        return math.floor(w * h * scale) + len(rest) + len(extra)\n"
            "    return area\n",
            {},
        )
        area = await namespace["main"](bl)
        self.assertEqual(await area(2), 6)
        payload = client.calls[0][1]["payload"]
        function = payload["function"]
        self.assertTrue(function["source"].startswith("def area(w, h=3"))
        self.assertNotIn("@bl.function", function["source"])
        self.assertEqual(function["modules"], {"math": "math"})
        self.assertEqual(function["args"], [2, 3])
        self.assertEqual(function["kwargs"], {"scale": 1})
        self.assertEqual(await area(1, 2, 9, scale=2, tag="x"), 7)
        second = client.calls[1][1]["payload"]["function"]
        self.assertIsNone(second["source"])
        self.assertEqual(second["args"], [1, 2, 9])
        self.assertEqual(second["kwargs"], {"scale": 2, "tag": "x"})

    async def test_uncached_function_is_resent_with_source(self):
        missing = ok()
        missing["result"].update({"ok": False, "error": {"type": "FunctionNotCached", "message": "gone"}})
        client = FakeClient([ok(1), missing, ok(2)])
        bl = client.blender.scoped(workspace_ref=SCOPE)
        namespace = make_function("def setup(bl):\n    @bl.function\n    def one():\n        return 1\n    return one\n", {})
        one = namespace["setup"](bl)
        await one()
        self.assertEqual(await one(), 2)
        self.assertIsNone(client.calls[1][1]["payload"]["function"]["source"])
        self.assertIsNotNone(client.calls[2][1]["payload"]["function"]["source"])

    async def test_outer_variables_are_rejected_and_dependencies_travel_along(self):
        client = FakeClient([ok(1)])
        bl = client.blender.scoped(workspace_ref=SCOPE)
        namespace = make_function(
            "def setup(bl):\n"
            "    LIMIT = 3\n"
            "    @bl.function\n"
            "    def helper(x):\n"
            "        return x + 1\n"
            "    @bl.function\n"
            "    def main(x):\n"
            "        return helper(x)\n"
            "    @bl.function\n"
            "    def bad():\n"
            "        return LIMIT\n"
            "    return main, bad\n",
            {},
        )
        main, bad = namespace["setup"](bl)
        await main(0)
        dependencies = client.calls[0][1]["payload"]["function"]["dependencies"]
        self.assertEqual([d["name"] for d in dependencies], ["helper"])
        with self.assertRaisesRegex(TypeError, "LIMIT"):
            await bad()

    def test_async_and_lambda_functions_are_refused(self):
        bl = FakeClient().blender
        with self.assertRaises(TypeError):
            bl.function(lambda: 1)

        async def coro():
            return 1

        with self.assertRaises(TypeError):
            bl.function(coro)


class AttachImageTests(unittest.TestCase):
    def test_attach_image_posts_a_temporary_file_for_the_python_call(self):
        seen = {}

        def fake_rpc(self, method, params, timeout):
            seen["method"] = method
            seen["exists"] = os.path.exists(params["path"])
            seen["call"] = params["pythonCall"]
            return {"attached": 1}

        env = {"LOCUS_PYTHON_CALL": "call-9", "LOCUS_SDK_URL": "http://x", "LOCUS_SDK_TOKEN": "t"}
        with mock.patch.dict(os.environ, env), mock.patch.object(locus.Client, "_rpc_sync", fake_rpc):
            locus.attach_image(PNG)
        self.assertEqual(seen, {"method": "attachments.add", "exists": True, "call": "call-9"})

    def test_attach_image_rejects_other_formats(self):
        with self.assertRaises(ValueError):
            locus.attach_image(b"GIF89a")


class PlaceholderBpyTests(unittest.TestCase):
    def test_bpy_import_explains_where_bpy_lives(self):
        with self.assertRaisesRegex(ImportError, "blender_execute"):
            import bpy  # noqa: F401


if __name__ == "__main__":
    unittest.main()
