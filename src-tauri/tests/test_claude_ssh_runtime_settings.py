"""Remote Claude runtime regression tests; no SSH, credentials or model requests.

Run: python src-tauri/tests/test_claude_ssh_runtime_settings.py
Windows uses a stub for the POSIX-only pty module; exec is captured, not run.
"""
from contextlib import nullcontext
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
source = Path(__file__).resolve().parents[2] / "crates/locus-remote-core/src/resources/ssh_host.py"
exec(compile(source.read_text(encoding="utf-8"), str(source), "exec"), host.__dict__)


class RuntimeSettingsTests(unittest.TestCase):
    def test_native_compact_window_overrides_only_when_configured(self):
        key = "CLAUDE_CODE_AUTO_COMPACT_WINDOW"
        env = {}
        host.apply_runtime_env(env, {})
        self.assertNotIn(key, env)
        env[key] = "500000"
        host.apply_runtime_env(env, {"runtimeSettings": {"autoCompactWindow": None}})
        self.assertEqual(env[key], "500000")
        for tokens in (100_000, 350_000, 1_000_000):
            host.apply_runtime_env(env, {"runtimeSettings": {"autoCompactWindow": tokens}})
            self.assertEqual(env[key], str(tokens))

    def test_invalid_compact_window_is_rejected_before_launch(self):
        for tokens in (0, 99_999, 1_000_001, 200_000.5, "200000", True):
            with self.subTest(tokens=tokens), self.assertRaisesRegex(ValueError, "auto-compact window"):
                host.apply_runtime_env({}, {"runtimeSettings": {"autoCompactWindow": tokens}})

    def test_native_window_reaches_remote_claude_and_reset_uses_host_defaults(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            profile = home / ".claude"
            profile.mkdir()
            settings = profile / "settings.json"
            settings.write_text('{"autoCompactWindow":500000}')
            cli = home / "claude"
            cli.write_text("test executable")
            captured = {}

            class Launched(Exception):
                pass

            def launch(executable, args, environment):
                captured.clear()
                captured.update(environment)
                self.assertEqual(executable, str(cli))
                allowed = args[args.index("--allowedTools") + 1:args.index("--disallowedTools")]
                self.assertIn("WebFetch", allowed)
                self.assertEqual(args[args.index("--disallowedTools") + 1], "mcp__locus__web_fetch")
                raise Launched()

            # Exercise run -> prepare -> inspect -> apply_runtime_env -> execvpe.
            # Only platform/process boundaries are mocked.
            with patch.object(sys, "platform", "darwin"), patch.object(Path, "home", return_value=home), \
                    patch.object(host, "settings_and_env", side_effect=lambda: (profile, {}, {"HOME": str(home)})), \
                    patch.object(host, "version_of", return_value=("2.1.299", True)), \
                    patch.object(host, "cli_auth_status", return_value=True), patch.object(host, "mac_keychain", return_value=True), \
                    patch.object(host, "validate_plugin"), \
                    patch.object(host, "acquire_session_lease", side_effect=lambda _: nullcontext()), \
                    patch.object(host, "Server"), patch.object(host.pty, "fork", return_value=(0, 1), create=True), \
                    patch.object(os, "chdir"), patch.object(os, "execvpe", side_effect=launch), \
                    patch.object(host.signal, "signal"), patch.object(os, "_exit", side_effect=Launched):
                config = {"protocol": 1, "sessionKey": str(uuid.uuid4()), "cliPath": str(cli), "authorizedSetup": True,
                          "modSource": "test", "token": "test", "options": {},
                          "nativeTools": "TaskCreate,TaskGet,TaskList,TaskUpdate,ToolSearch"}
                for window in (350_000, None):
                    with self.assertRaises(Launched):
                        host.run({**config, "runtimeSettings": {"autoCompactWindow": window}})
                    if window is None:
                        self.assertNotIn("CLAUDE_CODE_AUTO_COMPACT_WINDOW", captured)
                    else:
                        self.assertEqual(captured["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "350000")
            self.assertEqual(settings.read_text(), '{"autoCompactWindow":500000}')


if __name__ == "__main__":
    unittest.main()
