"""Download-only pooled SSH helper regressions. Offline; no CLI or model calls.

Run: python -B src-tauri/tests/test_claude_archive_pool.py
"""
import base64
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
import uuid
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "crates/locus-remote-core/src/resources/ssh_host.py"


def load_host():
    if os.name == "nt":
        sys.modules.setdefault("pty", types.ModuleType("pty"))
    host = types.ModuleType("archive_pool_fixture")
    exec(compile(SOURCE.read_text(encoding="utf-8"), str(SOURCE), "exec"), host.__dict__)
    return host


class ArchivePoolTests(unittest.TestCase):
    def setUp(self):
        self.host = load_host()

    def test_pool_handshake_never_installs_authenticates_or_starts_claude(self):
        host = self.host
        with patch.object(host, "read_frame", return_value={"type": "close"}), \
                patch.object(host, "frame") as frame, \
                patch.object(host, "unlock_for_connection") as unlock, \
                patch.object(host, "prepare") as prepare, \
                patch.object(host, "run_session") as session:
            host.run({"protocol": 1, "mode": "archive_read"})
        unlock.assert_not_called()
        prepare.assert_not_called()
        session.assert_not_called()
        hello = [c.args[0] for c in frame.call_args_list if c.args[0]["type"] == "hello"]
        self.assertEqual(len(hello), 1)
        self.assertFalse(hello[0]["info"]["cliSupported"])
        self.assertEqual(host.shutdown_status, (0, "client_close"))

    def test_one_connection_routes_each_download_to_its_own_session(self):
        host = self.host
        a, b, native = (str(uuid.uuid4()) for _ in range(3))
        requests = [{"type": "archive", "id": "a", "sessionKey": a, "nativeId": native},
                    {"type": "ping", "id": 7},
                    {"type": "archive", "id": "b", "sessionKey": b, "nativeId": native}, None]
        with patch.object(host, "read_frame", side_effect=requests), patch.object(host, "frame") as frame, \
                patch.object(host, "archive_download") as download:
            host.archive_read_listener()
        self.assertEqual([c.args[0].name for c in download.call_args_list], [a, b])
        self.assertEqual([c.args[1]["id"] for c in download.call_args_list], ["a", "b"])
        frame.assert_any_call({"type": "pong", "id": 7})
        self.assertEqual(host.shutdown_status, (0, "stdin_eof"))

    def test_writes_missing_identities_and_path_traversal_are_rejected_without_changing_scope(self):
        host = self.host
        session, native = str(uuid.uuid4()), str(uuid.uuid4())
        invalid = [{"type": "restore_begin"}, {"type": "restore_chunk"}, {"type": "restore_commit"},
                   {"type": "input"}, {"type": "archive"},
                   {"type": "archive", "sessionKey": "../outside", "nativeId": native},
                   {"type": "archive", "sessionKey": session, "nativeId": "../outside"},
                   {"type": "archive", "sessionKey": None, "nativeId": native}]
        valid = {"type": "archive", "id": "valid", "sessionKey": session, "nativeId": native}
        requests = [dict(value, id=str(i)) for i, value in enumerate(invalid)] + [valid, None]
        with patch.object(host, "read_frame", side_effect=requests), patch.object(host, "frame") as frame, \
                patch.object(host, "archive_download") as download, \
                patch.object(host, "restore_prepare") as restore:
            host.archive_read_listener()
        restore.assert_not_called()
        download.assert_called_once()
        self.assertEqual(download.call_args.args[0].name, session)
        errors = [c.args[0] for c in frame.call_args_list if c.args[0]["type"] == "archive_error"]
        self.assertEqual(len(errors), len(invalid))
        self.assertEqual({r["id"] for r in errors}, {str(i) for i in range(len(invalid))})

    def test_real_archive_protocol_reuses_connection_and_incremental_cursor_without_mixing_files(self):
        host = self.host
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            sessions = home / ".local/share/locus/claude-ssh/v1/sessions"
            native = str(uuid.uuid4())
            keys = [str(uuid.uuid4()), str(uuid.uuid4())]
            expected = {}
            for i, key in enumerate(keys):
                root = sessions / key
                project = root / "profile/projects/test"
                project.mkdir(parents=True)
                (root / "native-storage.json").write_text(json.dumps({"version": 1,
                    "profile": str(root / "profile"), "project": str(project)}))
                text = json.dumps({"uuid": str(uuid.uuid4()), "type": "user", "message": {"content": "session-%d" % i}}) + "\n"
                (project / (native + ".jsonl")).write_bytes(text.encode())
                expected[key] = text.encode()
            frames = []
            requests = iter([{"type": "archive", "id": key, "sessionKey": key, "nativeId": native} for key in keys])
            cursor_sent = False
            def next_frame():
                nonlocal cursor_sent
                value = next(requests, None)
                if value is not None:
                    return value
                if cursor_sent:
                    return None
                cursor_sent = True
                result = next(f for f in frames if f.get("id") == keys[0] and f["type"] == "archive_result")
                return {"type": "archive", "id": "incremental", "sessionKey": keys[0], "nativeId": native,
                        "cursor": result["value"]["manifest"]}
            with patch.object(host.Path, "home", return_value=home), \
                    patch.object(host, "read_frame", side_effect=next_frame), \
                    patch.object(host, "frame", side_effect=lambda value, **_: frames.append(value)):
                host.archive_read_listener()
            for key in keys:
                chunks = [f for f in frames if f["type"] == "archive_chunk" and f["id"] == key]
                self.assertEqual(b"".join(base64.b64decode(f["data"]) for f in chunks), expected[key])
            delta = next(f for f in frames if f.get("id") == "incremental" and f["type"] == "archive_result")
            self.assertEqual(delta["value"]["transferredBytes"], 0)
            self.assertFalse(any(f["type"] == "archive_error" for f in frames))


if __name__ == "__main__":
    unittest.main()
