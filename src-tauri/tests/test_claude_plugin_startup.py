"""Claude plugin startup diagnostics; no credentials, SSH or model requests."""
from contextlib import nullcontext
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
import uuid
from unittest.mock import Mock, patch

if os.name == "nt":
    sys.modules.setdefault("pty", types.ModuleType("pty"))
host = types.ModuleType("locus_ssh_host")
source = Path(__file__).resolve().parents[2] / "crates/locus-remote-core/src/resources/ssh_host.py"
exec(compile(source.read_text(encoding="utf-8"), str(source), "exec"), host.__dict__)


class PluginStartupTests(unittest.TestCase):
    def test_invalid_plugin_reports_the_cli_cause(self):
        result = types.SimpleNamespace(returncode=1, stdout=b"register.js:116: invalid helper\nValidation failed")
        with patch.object(subprocess, "run", return_value=result) as run:
            with self.assertRaisesRegex(RuntimeError, "(?s)Claude plugin validation failed.*register.js:116"):
                host.validate_plugin("claude", Path("plugin"), {"PATH": "bin"})
        self.assertEqual(run.call_args.args[0], ["claude", "plugin", "validate", "plugin"])
        self.assertEqual(run.call_args.kwargs["stdin"], subprocess.DEVNULL)

    def test_validation_accepts_warnings_and_bounds_failures_and_timeouts(self):
        with patch.object(subprocess, "run", return_value=types.SimpleNamespace(returncode=0, stdout=b"warning")):
            host.validate_plugin("claude", Path("plugin"), {})
        with patch.object(subprocess, "run", return_value=types.SimpleNamespace(returncode=1, stdout=b"x" * 20000 + b"cause")):
            with self.assertRaises(RuntimeError) as raised:
                host.validate_plugin("claude", Path("plugin"), {})
            self.assertLess(len(str(raised.exception)), 8100)
            self.assertTrue(str(raised.exception).endswith("cause"))
        with patch.object(subprocess, "run", side_effect=subprocess.TimeoutExpired("claude", 20)):
            with self.assertRaisesRegex(RuntimeError, "validation timed out"):
                host.validate_plugin("claude", Path("plugin"), {})

    def test_session_propagates_plugin_failure_before_forking_a_tui(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            profile = home / ".claude"
            profile.mkdir()
            result = types.SimpleNamespace(returncode=1, stdout=b"register.js:116: invalid helper")
            with patch.object(sys, "platform", "darwin"), patch.object(Path, "home", return_value=home), \
                    patch.object(host, "prepare", return_value=("claude", profile, {}, {"authenticated": True})), \
                    patch.object(host, "acquire_session_lease", side_effect=lambda _: nullcontext()), \
                    patch.object(subprocess, "run", return_value=result), \
                    patch.object(host.pty, "fork", create=True) as fork:
                with self.assertRaisesRegex(RuntimeError, "(?s)Claude plugin validation failed.*register.js:116"):
                    host.run({"protocol": 1, "sessionKey": str(uuid.uuid4()), "modSource": "broken plugin", "token": "test", "options": {}})
                fork.assert_not_called()


class BridgeLifecycleTests(unittest.TestCase):
    def test_worker_ready_and_fault_bypass_the_ordered_tool_lane(self):
        # An unfinished tool deliberately has no reply in the ordered lane.
        # Lifecycle/control traffic must still reach the Rust watchdog.
        for path, payload in [
            ("/ready", {"sessionId": str(uuid.uuid4()), "model": "fixture"}),
            ("/event", {"type": "bridge_fault", "runId": "run", "error": "worker failed"}),
        ]:
            with self.subTest(path=path, payload=payload):
                body = json.dumps({"protocol": 1, **payload}).encode()
                lane = types.SimpleNamespace(submit=Mock(side_effect=AssertionError("blocked tool lane")))
                server = types.SimpleNamespace(token="token", authority="127.0.0.1:1", events=lane,
                                               session_root=Path("unused-fixture"))
                request = types.SimpleNamespace(path=path, server=server, rfile=io.BytesIO(body),
                    headers={"Authorization": "Bearer token", "Host": server.authority, "Content-Length": str(len(body))},
                    respond=Mock())
                request.finish_receipt = types.MethodType(host.Bridge.finish_receipt, request)
                def accept(frame):
                    receipt = host.pending[frame["id"]]
                    receipt["value"] = {"ok": True}
                    receipt["event"].set()
                with patch.object(host, "frame", side_effect=accept) as sent, patch.object(host, "private_write"):
                    host.Bridge.do_POST(request)
                request.respond.assert_called_once_with(200, {"ok": True})
                lane.submit.assert_not_called()
                self.assertEqual(sent.call_count, 1)
                self.assertFalse(host.pending)

    def test_health_does_not_wait_for_a_remote_receipt_or_use_tool_slots(self):
        # Even when no SSH reply comes back and all tool receipt slots are
        # occupied, each new sample must be forwarded, not stuck polling the
        # first sample forever with the Mod's reportingHealth flag held.
        lane = types.SimpleNamespace(submit=Mock(side_effect=AssertionError("blocked tool lane")))
        server = types.SimpleNamespace(token="token", authority="127.0.0.1:1", events=lane)
        for occupied in (0, 64):
            with self.subTest(occupied_tool_slots=occupied):
                pending = {str(index): object() for index in range(occupied)}
                with patch.object(host, "options", {"active": True, "inputId": "run"}), \
                        patch.object(host, "pending", pending), patch.object(host, "frame") as sent:
                    for state in ("running", "running", "complete"):
                        payload = {"protocol": 1, "type": "bridge_health", "runId": "run", "state": state}
                        body = json.dumps(payload).encode()
                        request = types.SimpleNamespace(path="/event", server=server, rfile=io.BytesIO(body),
                            headers={"Authorization": "Bearer token", "Host": server.authority, "Content-Length": str(len(body))},
                            respond=Mock(), finish_receipt=Mock(side_effect=AssertionError("health must not poll")))
                        host.Bridge.do_POST(request)
                        request.respond.assert_called_once_with(200, {"ok": True})
                        sent.assert_called_with({"type": "bridge_health", "payload": payload})
                    self.assertEqual(sent.call_count, 3)
                    self.assertEqual(len(pending), occupied)
                    lane.submit.assert_not_called()

    def test_health_rejects_stale_idle_and_invalid_samples_before_forwarding(self):
        for active, run_id, state in [(True, "old", "running"), (False, "run", "running"), (True, "run", "unknown")]:
            with self.subTest(active=active, run_id=run_id, state=state):
                body = json.dumps({"protocol": 1, "type": "bridge_health", "runId": run_id, "state": state}).encode()
                server = types.SimpleNamespace(token="token", authority="127.0.0.1:1")
                request = types.SimpleNamespace(path="/event", server=server, rfile=io.BytesIO(body),
                    headers={"Authorization": "Bearer token", "Host": server.authority, "Content-Length": str(len(body))},
                    respond=Mock())
                with patch.object(host, "options", {"active": active, "inputId": "run"}), patch.object(host, "frame") as sent:
                    host.Bridge.do_POST(request)
                self.assertGreaterEqual(request.respond.call_args.args[0], 400)
                sent.assert_not_called()

    def test_health_is_not_an_asynchronous_display_event(self):
        self.assertIn("bridge_health", host.CONTROL_EVENTS)
        self.assertIn("bridge_fault", host.CONTROL_EVENTS)
        self.assertNotIn("bridge_health", host.ASYNC_EVENTS)
        self.assertNotIn("bridge_fault", host.ASYNC_EVENTS)


if __name__ == "__main__":
    unittest.main()
