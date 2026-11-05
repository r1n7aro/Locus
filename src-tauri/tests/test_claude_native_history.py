"""Native chat-display regression tests; no SSH, credentials or model requests.

Run: python src-tauri/tests/test_claude_native_history.py
The fixtures are also exercised by Rust's native archive projection tests.
"""
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
import uuid
from unittest.mock import patch

if os.name == "nt":
    sys.modules.setdefault("pty", types.ModuleType("pty"))
host = types.ModuleType("locus_ssh_host")
root = Path(__file__).resolve().parents[1]
source = root.parent / "crates/locus-remote-core/src/resources/ssh_host.py"
exec(compile(source.read_text(encoding="utf-8"), str(source), "exec"), host.__dict__)
CASES = json.loads((root / "tests/fixtures/claude_native_user_visibility.json").read_text(encoding="utf-8"))


def record(index, text, parent=None):
    return {"uuid": str(uuid.UUID(int=index)), "parentUuid": str(uuid.UUID(int=parent)) if parent else None,
            "type": "user", "message": {"content": text}}


class NativeHistoryDisplayTests(unittest.TestCase):
    def test_completed_answer_retains_stop_reason_for_lost_ack_recovery(self):
        user = record(1, "request")
        answer = {"uuid": str(uuid.UUID(int=2)), "parentUuid": user["uuid"], "type": "assistant",
                  "message": {"id": "answer", "content": [{"type": "text", "text": "done"}], "stop_reason": "end_turn"}}
        self.assertEqual(host.canonical_history([user, answer])[-1]["stopReason"], "end_turn")
        del answer["message"]["stop_reason"]
        self.assertNotIn("stopReason", host.canonical_history([user, answer])[-1])

    def test_compaction_fork_snapshot_keeps_command_echo_and_carry_forward(self):
        rows = [
            {"uuid": "command"},
            {"uuid": "boundary", "type": "system"},
            {"uuid": "summary", "type": "user", "isCompactSummary": True, "message": {"content": "Summary"}},
            {"uuid": "caveat", "type": "user", "isMeta": True, "message": {"content": "<local-command-caveat>x</local-command-caveat>"}},
            {"uuid": "echo", "type": "user", "message": {"content": "<command-name>/compact</command-name>"}},
            {"uuid": "stdout", "type": "user", "message": {"content": "<local-command-stdout>Compacted</local-command-stdout>"}},
            {"uuid": "carry", "type": "attachment", "attachment": {"type": "hook_additional_context", "content": ["CARRY"]}},
            {"uuid": "future", "type": "user", "message": {"content": "next request"}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "native.jsonl"
            path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
            copied = host.transcript_snapshot(path, "command", True)
            self.assertIn('"carry"', copied)
            self.assertIn('"stdout"', copied)
            self.assertNotIn('"future"', copied)
            self.assertNotIn('"carry"', host.transcript_snapshot(path, "caveat", False))

    def test_session_lease_waits_for_orphan_and_times_out_without_killing_it(self):
        fake = types.SimpleNamespace(LOCK_EX=2, LOCK_NB=4)
        with tempfile.TemporaryDirectory() as directory, patch.dict(sys.modules, {"fcntl": fake}):
            with patch.object(fake, "flock", create=True, side_effect=[BlockingIOError(), None]) as lock, \
                    patch.object(host.time, "sleep") as sleep:
                lease = host.acquire_session_lease(Path(directory))
                self.assertFalse(lease.closed)
                self.assertEqual(lock.call_count, 2)
                sleep.assert_called_once_with(0.1)
                lease.close()
            with patch.object(fake, "flock", create=True, side_effect=BlockingIOError()) as lock, \
                    patch.object(host.time, "monotonic", side_effect=[0, 40]):
                with self.assertRaisesRegex(TimeoutError, "Claude transport disconnected"):
                    host.acquire_session_lease(Path(directory))
                self.assertTrue(lock.call_args.args[0].closed)

    def test_automatic_history_recovery_takes_exclusive_lease_before_reading(self):
        from contextlib import contextmanager
        steps = []
        @contextmanager
        def lease(_):
            steps.append("locked")
            yield
            steps.append("released")
        def history(_):
            self.assertEqual(steps, ["locked"])
            return {"verified": True}
        config = {"protocol": 1, "mode": "history", "sessionKey": str(uuid.uuid4()), "requireIdle": True}
        with patch.object(host, "acquire_session_lease", side_effect=lease), \
                patch.object(host, "read_native_history", side_effect=history), patch.object(host, "frame") as frame:
            host.run(config)
        self.assertEqual(steps, ["locked", "released"])
        self.assertTrue(frame.call_args.args[0]["info"]["history"]["verified"])

    def test_shared_visibility_fixtures(self):
        for case in CASES:
            with self.subTest(case=case["name"]):
                self.assertEqual(host.hidden_native_user(case["row"]), case["hidden"])

    def test_hidden_parents_do_not_drop_real_messages_or_change_the_source(self):
        for case in CASES:
            with self.subTest(case=case["name"]):
                candidate = {**copy.deepcopy(case["row"]), "uuid": str(uuid.UUID(int=2)), "parentUuid": str(uuid.UUID(int=1))}
                rows = [record(1, "before"), candidate, record(3, "after", 2)]
                original = copy.deepcopy(rows)
                hidden = []
                messages = host.canonical_history(rows, hidden)
                self.assertEqual(any(candidate["uuid"] in m["uuids"] for m in messages), not case["hidden"])
                self.assertEqual(candidate["uuid"] in hidden, case["hidden"])
                self.assertEqual(messages[0]["content"][0]["text"], "before")
                self.assertEqual(messages[-1]["content"][0]["text"], "after")
                self.assertEqual(rows, original)

    def test_remote_history_reader_preserves_classification_flags_and_hidden_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            transcript = home / "native.jsonl"
            rows = [record(1, "before")]
            hidden = []
            visible = {rows[0]["uuid"]}
            for index, case in enumerate(CASES, 2):
                candidate = {**copy.deepcopy(case["row"]), "uuid": str(uuid.UUID(int=index)), "parentUuid": str(uuid.UUID(int=index - 1))}
                rows.append(candidate)
                if case["hidden"]:
                    hidden.append(candidate["uuid"])
                else:
                    visible.add(candidate["uuid"])
            data = "".join(json.dumps(row) + "\n" for row in rows)
            transcript.write_text(data, encoding="utf-8")
            with patch.object(Path, "home", return_value=home), \
                    patch.object(host, "history_transcripts", return_value=[transcript]), \
                    patch.object(host, "native_storage", return_value=(home, home)):
                snapshot = host.read_native_history({"sessionKey": str(uuid.uuid4()), "nativeId": str(uuid.uuid4())})
            self.assertEqual(snapshot["hiddenUserUuids"], hidden)
            self.assertEqual({u for m in snapshot["messages"] for u in m["uuids"]}, visible)
            self.assertEqual(snapshot["pendingToolIds"], [])
            self.assertEqual(transcript.read_text(encoding="utf-8"), data)

    def test_consumed_human_queue_can_quote_native_output_and_keep_images(self):
        queued = {"uuid": str(uuid.uuid4()), "type": "attachment", "isMeta": True,
                  "attachment": {"type": "queued_command", "humanTurn": True, "origin": {"kind": "human"},
                                 "prompt": "<local-command-stdout>quoted by a person</local-command-stdout>"},
                  "rendered": [{"content": [{"type": "text", "text": "native reminder"},
                                            {"type": "image", "source": {"type": "base64", "data": "aQ==", "media_type": "image/png"}}]}]}
        messages = host.canonical_history([queued])
        self.assertEqual(len(messages), 1)
        self.assertTrue(messages[0]["queuedInput"])
        self.assertEqual(messages[0]["content"][0]["text"], queued["attachment"]["prompt"])
        self.assertEqual(messages[0]["content"][1]["type"], "image")


if __name__ == "__main__":
    unittest.main()
