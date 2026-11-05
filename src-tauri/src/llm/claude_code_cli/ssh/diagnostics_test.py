"""Offline SSH host regression tests (diagnostics, automatic Claude Code
setup); no servers, downloads or credentials required.

Run: python src-tauri/src/llm/claude_code_cli/ssh/diagnostics_test.py
"""
from contextlib import nullcontext
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import uuid

if os.name == "nt":
    sys.modules.setdefault("pty", types.ModuleType("pty"))
host = types.ModuleType("locus_ssh_host_test")
source = Path(__file__).resolve().parents[5] / "crates/locus-remote-core/src/resources/ssh_host.py"
exec(compile(source.read_text(encoding="utf-8"), str(source), "exec"), host.__dict__)


class DiagnosticsTests(unittest.TestCase):
    def test_history_progress_precedes_lock_read_and_transfer(self):
        events = []

        def lease(root):
            self.assertEqual(events[-1]["stage"], "history_lock")
            return nullcontext()

        def history(config):
            self.assertEqual(events[-1]["stage"], "history_read")
            return {"version": 1}

        with patch.object(host, "frame", side_effect=lambda value, **kwargs: events.append(value)) as frame, \
                patch.object(host, "acquire_session_lease", side_effect=lease), \
                patch.object(host, "read_native_history", side_effect=history), \
                patch.object(host, "unlock_for_connection") as unlock, patch.object(host, "prepare") as prepare:
            host.run({"protocol": 1, "mode": "history", "sessionKey": str(uuid.uuid4()), "requireIdle": True})
        self.assertEqual(events[0]["source"], "helper")
        self.assertEqual([event.get("stage") for event in events[:-1]],
                         ["history", "history_lock", "history_read", "history_serialize"])
        self.assertEqual(events[-1]["type"], "hello")
        self.assertTrue(frame.call_args.kwargs["announce_history_size"])
        unlock.assert_not_called()
        prepare.assert_not_called()

    def test_announced_size_matches_unicode_json_and_final_newline(self):
        output = io.StringIO()
        value = {"type": "hello", "protocol": 1, "info": {"history": "中文\n"}}
        with patch.object(sys, "stdout", output):
            host.frame(value, announce_history_size=True)
        lines = output.getvalue().splitlines(keepends=True)
        progress, hello = map(json.loads, lines)
        self.assertEqual(progress["stage"], "history_transfer")
        self.assertEqual(progress["bytes"], len(lines[1].encode("utf-8")))
        self.assertEqual(hello, value)

    def test_normal_frames_add_no_progress_or_extra_serialization(self):
        output = io.StringIO()
        with patch.object(sys, "stdout", output), patch.object(json, "dumps", wraps=json.dumps) as dumps:
            host.frame({"type": "pong", "id": 1})
        self.assertEqual(dumps.call_count, 1)
        self.assertEqual(output.getvalue(), '{"type":"pong","id":1}\n')

    def test_transfer_marker_and_payload_hold_one_stdout_lock(self):
        class Lock:
            held = False
            entries = 0

            def __enter__(lock):
                self.assertFalse(lock.held)
                lock.held = True
                lock.entries += 1

            def __exit__(lock, *_):
                lock.held = False

        lock = Lock()

        class Output(io.StringIO):
            def write(output, text):
                self.assertTrue(lock.held)
                return super().write(text)

        for value in [{"type": "hello", "protocol": 1, "info": {"history": "中文"}},
                      {"type": "archive_chunk", "id": "request-1", "name": "transcript.jsonl", "data": "YWJj"},
                      {"type": "archive_result", "id": "request-1", "value": {"manifest": {}}}]:
            lock.entries = 0
            output = Output()
            with patch.object(host, "output_lock", lock), patch.object(sys, "stdout", output), \
                    patch.object(json, "dumps", wraps=json.dumps) as dumps:
                host.frame(value, announce_history_size=value["type"] == "hello")
            lines = output.getvalue().splitlines(keepends=True)
            self.assertEqual(len(lines), 2)
            progress, payload = map(json.loads, lines)
            self.assertEqual(progress["protocol"], 1)
            self.assertEqual(progress["bytes"], len(lines[1].encode("utf-8")))
            self.assertEqual(payload, value)
            self.assertEqual(lock.entries, 1)
            self.assertEqual(dumps.call_count, 2)  # payload once, small marker once
            if value["type"] != "hello":
                self.assertEqual(progress["stage"], "archive_transfer")
                self.assertEqual(progress["id"], value["id"])

    def test_failed_lease_does_not_report_history_read_or_ready(self):
        events = []
        with patch.object(host, "frame", side_effect=events.append), \
                patch.object(host, "acquire_session_lease", side_effect=TimeoutError("lease busy")), \
                patch.object(host, "read_native_history") as history:
            with self.assertRaisesRegex(TimeoutError, "lease busy"):
                host.run({"protocol": 1, "mode": "history", "sessionKey": str(uuid.uuid4()), "requireIdle": True})
        history.assert_not_called()
        self.assertEqual(events[-1]["stage"], "history_lock")


class SetupTests(unittest.TestCase):
    """When `prepare` installs Claude Code. The installer itself is mocked."""

    def prepare(self, config, versions, install_error=None):
        # `versions`: the CLI version each inspection reports, in order.
        reported = iter(versions)

        def inspect(_):
            version = next(reported)
            return "/usr/bin/claude", Path("/tmp"), {"PATH": "/usr/bin"}, {
                "cliVersion": version, "cliSupported": host.version_at_least(version, host.MIN_VERSION)}

        events = []
        with tempfile.TemporaryDirectory() as home, \
                patch.object(host, "inspect", side_effect=inspect), \
                patch.object(host, "install_claude", side_effect=install_error) as install, \
                patch.object(host, "frame", side_effect=events.append), \
                patch.object(host, "private_write"), \
                patch.object(host, "executable_identity", return_value={}), \
                patch.object(host.Path, "home", return_value=Path(home)):
            info = host.prepare({"authorizedSetup": True, **config}, "prepare")[3]
        return info, install, events

    def test_setup_with_native_agents_upgrades_an_older_cli(self):
        info, install, _ = self.prepare({"nativeMultiAgent": True}, ["2.1.300 (Claude Code)", "2.1.312 (Claude Code)"])
        install.assert_called_once()
        self.assertEqual(info["cliVersion"], "2.1.312 (Claude Code)")
        self.assertEqual(host.INSTALL_VERSION, "2.1.312")

    def test_setup_keeps_a_mods_capable_cli_when_the_upgrade_fails(self):
        info, install, events = self.prepare({"nativeMultiAgent": True}, ["2.1.300 (Claude Code)"], RuntimeError("offline"))
        install.assert_called_once()
        self.assertEqual(info["cliVersion"], "2.1.300 (Claude Code)")
        self.assertIn("native agents stay off", events[-1]["message"])

    def test_a_native_agent_launch_requires_the_upgrade(self):
        with self.assertRaisesRegex(RuntimeError, "offline"):
            self.prepare({"nativeAgents": True}, ["2.1.300 (Claude Code)"], RuntimeError("offline"))
        with self.assertRaisesRegex(RuntimeError, "does not support native agents"):
            self.prepare({"nativeAgents": True}, ["2.1.300 (Claude Code)", "2.1.311 (Claude Code)"])

    def test_other_setups_and_current_clis_install_nothing(self):
        for config, version in (({}, "2.1.300 (Claude Code)"), ({"nativeMultiAgent": True}, "2.1.320 (Claude Code)"),
                                ({"nativeAgents": True}, "2.1.312 (Claude Code)")):
            _, install, _ = self.prepare(config, [version])
            install.assert_not_called()

    def test_a_missing_cli_is_installed_and_must_install(self):
        info, install, _ = self.prepare({}, [None, "2.1.312 (Claude Code)"])
        install.assert_called_once()
        self.assertEqual(info["cliVersion"], "2.1.312 (Claude Code)")
        with self.assertRaisesRegex(RuntimeError, "offline"):
            self.prepare({"nativeMultiAgent": True}, [None], RuntimeError("offline"))

    def test_a_custom_executable_is_never_replaced(self):
        _, install, _ = self.prepare({"cliPath": "/opt/claude", "nativeMultiAgent": True}, ["2.1.300 (Claude Code)"])
        install.assert_not_called()
        with self.assertRaisesRegex(RuntimeError, "too old for native agents"):
            self.prepare({"cliPath": "/opt/claude", "nativeAgents": True}, ["2.1.300 (Claude Code)"])


if __name__ == "__main__":
    unittest.main()
