import contextlib
import io
import json
import linecache
import os
import unittest
from unittest.mock import patch

import locus
from locus._client import _print_reported_error
from locus._merges import MergeJob

STALE = "stale: a.txt changed after the snapshot; replan before writing"
HINT = "Nothing was written. Run `job = await job.replan()`."
SCRIPT = "async def main(job):\n    await job.plan().apply(expected_plan_hash='h')\n"


def bridge(error):
    """urlopen stand-in answering every request with one JSON-RPC error."""
    class Response:
        def __init__(self, request):
            self.id = json.loads(request.data)["id"]

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            pass

        def read(self):
            return json.dumps({"jsonrpc": "2.0", "id": self.id, "error": error}).encode()

    return patch("urllib.request.urlopen", side_effect=lambda request, **_: Response(request))


class ReportedErrorTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        client = locus.Client(base_url="http://127.0.0.1/sdk", token="test")
        self.job = MergeJob(client, {"checkoutId": "A"},
            {"id": "job-1", "snapshot": {}, "project_root": "C:/project", "state": "prepared"})
        # The Python tool compiles the agent's code under this file name.
        linecache.cache["<locus-python>"] = (len(SCRIPT), None, SCRIPT.splitlines(True), "<locus-python>")
        self.script = {}
        exec(compile(SCRIPT, "<locus-python>", "exec"), self.script)

    async def raised(self, error):
        # assertRaises would strip the traceback this test inspects.
        try:
            with bridge(error):
                await self.script["main"](self.job)
        except locus.LocusRpcError as raised:
            return raised
        self.fail("LocusRpcError was not raised")

    async def test_reported_condition_carries_kind_hint_and_method(self):
        error = await self.raised({"code": -32602, "message": STALE,
            "data": {"reported": True, "kind": "stale", "hint": HINT}})
        self.assertTrue(error.reported)
        self.assertEqual((error.kind, error.hint, error.method), ("stale", HINT, "merges.apply"))
        self.assertEqual(str(error), STALE)
        self.assertEqual(error.code, -32602)

    async def test_reported_condition_prints_script_frames_message_and_next_step(self):
        error = await self.raised({"code": -32602, "message": STALE,
            "data": {"reported": True, "kind": "stale", "hint": HINT}})
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            self.assertTrue(_print_reported_error(error))
        text = output.getvalue()
        self.assertIn("Locus SDK frames omitted", text)
        self.assertIn('File "<locus-python>", line 2, in main', text)
        self.assertIn("await job.plan().apply(expected_plan_hash='h')", text)
        self.assertIn(f"locus.LocusRpcError [merges.apply/stale]: {STALE}", text)
        self.assertIn(f"next: {HINT}", text)
        for internal in ("_client.py", "_merges.py", "asyncio", "concurrent", "threading"):
            self.assertNotIn(internal, text)

    async def test_unexpected_failures_keep_their_traceback(self):
        error = await self.raised({"code": -32602, "message": "Could not read LFS object: denied"})
        self.assertFalse(error.reported)
        self.assertIsNone(error.kind)
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            self.assertFalse(_print_reported_error(error))
            self.assertFalse(_print_reported_error(ValueError("not an SDK error")))
        self.assertEqual(output.getvalue(), "")

    async def test_full_traceback_can_be_requested_for_reported_conditions(self):
        error = await self.raised({"code": -32602, "message": STALE,
            "data": {"reported": True, "kind": "stale"}})
        with patch.dict(os.environ, {"LOCUS_SDK_FULL_TRACEBACK": "1"}):
            self.assertFalse(_print_reported_error(error))

    async def test_malformed_error_data_is_ignored(self):
        error = await self.raised({"code": -32602, "message": STALE, "data": ["reported"]})
        self.assertFalse(error.reported)
        self.assertEqual(error.data, {})


if __name__ == "__main__":
    unittest.main()
