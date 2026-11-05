"""SSH helper shutdown regressions; offline, no SSH or Claude processes.

Run: python -B src-tauri/tests/test_claude_ssh_lifecycle.py
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "crates/locus-remote-core/src/resources/ssh_host.py"


def load_host():
    if os.name == "nt":
        sys.modules.setdefault("pty", types.ModuleType("pty"))
    host = types.ModuleType("locus_ssh_lifecycle_test")
    host._locus_source = SOURCE.read_bytes()
    exec(compile(host._locus_source, str(SOURCE), "exec"), host.__dict__)
    return host


# Keep stdin OPEN while waiting for exit: communicate() would close it and hide
# the original BufferedReader/daemon finalization race. No real child is killed.
CHILD = r'''
import os, pathlib, signal, sys, threading, time, types
if os.name == "nt":
    sys.modules["pty"] = types.ModuleType("pty")
host = types.ModuleType("locus_ssh_lifecycle_child")
host._locus_source = pathlib.Path(sys.argv[1]).read_bytes()
exec(compile(host._locus_source, sys.argv[1], "exec"), host.__dict__)
host.Path.home = classmethod(lambda cls: pathlib.Path(sys.argv[2]))
case = sys.argv[3]
host.read_frame = lambda: {"protocol": 1, "password": "DO_NOT_LOG_THIS_SECRET"}
def fail(*args, **kwargs):
    raise OSError("DO_NOT_LOG_THIS_SECRET")
def run(config):
    started = threading.Event()
    def reader():
        started.set()
        sys.stdin.buffer.readline()
    threading.Thread(target=reader, daemon=True).start()
    assert started.wait(1)
    time.sleep(0.05)
    if case == "interrupt":
        raise KeyboardInterrupt()
    if case == "system_exit":
        raise SystemExit(7)
    if case == "system_exit_zero":
        raise SystemExit(0)
    if case == "report_error":
        host.frame = fail
        raise ValueError("DO_NOT_LOG_THIS_SECRET")
    if case == "cleanup_error":
        host.terminate_child = fail
    if case == "log_error":
        host.record_lifecycle = fail
        raise RuntimeError("DO_NOT_LOG_THIS_SECRET")
    if case in ("signal", "sigint", "sighup"):
        os.kill(os.getpid(), getattr(signal, {"signal": "SIGTERM", "sigint": "SIGINT", "sighup": "SIGHUP"}[case]))
    if case in ("watchdog", "watchdog_cleanup_error"):
        if case == "watchdog_cleanup_error":
            host.terminate_child = fail
        host.last_contact = host.time.monotonic() - 26
        host.watchdog()
    if case == "stdin_error":
        host.read_frame = fail
        host.stdin_loop(-1, None)
    if case == "ordinary":
        raise ValueError("DO_NOT_LOG_THIS_SECRET")
host.run = run
host.main()
raise AssertionError("main returned instead of os._exit")
'''


class ProcessExitTests(unittest.TestCase):
    def run_case(self, case, expected_code, expected_reason):
        with tempfile.TemporaryDirectory() as directory:
            child = subprocess.Popen([sys.executable, "-B", "-u", "-c", CHILD, str(SOURCE), directory, case],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                self.assertEqual(child.wait(timeout=8), expected_code, case)
                stdout, stderr = child.communicate(timeout=2)
            finally:
                if child.poll() is None:
                    child.kill()
                child.communicate()
            self.assertNotIn(b"Fatal Python error", stderr)
            records = [json.loads(line[len(b"LOCUS_SSH_HELPER "):]) for line in stderr.splitlines()
                       if line.startswith(b"LOCUS_SSH_HELPER ")]
            self.assertEqual(records[0]["event"], "started")
            files = list(Path(directory).rglob("helper-*.json"))
            self.assertEqual(len(files), 1)
            record = json.loads(files[0].read_text())
            self.assertEqual(record["hostPid"], child.pid)
            self.assertEqual(record["helperVersion"], "3")
            self.assertEqual(record["sourceHash"], hashlib.sha256(SOURCE.read_bytes()).hexdigest())
            self.assertNotIn("DO_NOT_LOG_THIS_SECRET", json.dumps(record))
            self.assertNotIn(b"DO_NOT_LOG_THIS_SECRET", stderr)
            if case != "log_error":
                self.assertEqual(record["event"], "exit")
                self.assertEqual(record["exitCode"], expected_code)
                self.assertEqual(record["exitReason"], expected_reason)
                self.assertEqual(records[-1], record)
            return record

    def test_normal_exit_with_a_blocked_buffered_reader(self):
        self.run_case("normal", 0, "completed")

    def test_base_exceptions_do_not_reach_python_finalization(self):
        for case, code, reason in [("interrupt", 130, "keyboard_interrupt"),
                                   ("system_exit", 7, "system_exit"),
                                   ("system_exit_zero", 0, "system_exit"),
                                   ("ordinary", 1, "unhandled_exception")]:
            with self.subTest(case=case):
                self.run_case(case, code, reason)

    def test_reporting_cleanup_and_logging_failures_still_exit(self):
        record = self.run_case("report_error", 1, "unhandled_exception")
        self.assertEqual(record["reportingError"], "OSError")
        record = self.run_case("cleanup_error", 1, "cleanup_error")
        self.assertEqual(record["cleanupError"], "OSError")
        self.run_case("log_error", 1, None)

    def test_background_failures_are_preserved(self):
        self.run_case("stdin_error", 1, "stdin_error")
        self.run_case("watchdog", 1, "heartbeat_timeout")
        record = self.run_case("watchdog_cleanup_error", 1, "heartbeat_timeout")
        self.assertEqual(record["cleanupError"], "OSError")

    @unittest.skipIf(os.name == "nt", "Windows kill(SIGTERM) bypasses Python signal handlers")
    def test_signals_have_durable_reasons(self):
        for case, code, reason in [("signal", 143, "signal_sigterm"), ("sigint", 130, "signal_sigint"),
                                   ("sighup", 129, "signal_sighup")]:
            with self.subTest(case=case):
                self.run_case(case, code, reason)


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.host = load_host()

    def test_eof_close_and_invalid_frame_are_distinct(self):
        for value, reason, code in [(None, "stdin_eof", 0), ({"type": "close"}, "client_close", 0),
                                    ({"type": "unknown"}, "stdin_error", 1)]:
            host = load_host()
            with patch.object(host, "read_frame", return_value=value):
                host.stdin_loop(-1, None)
            self.assertEqual(host.shutdown_status, (code, reason))

    def test_remote_journal_is_bounded_private_and_does_not_log_payloads(self):
        host = self.host
        with tempfile.TemporaryDirectory() as directory, patch.object(host.Path, "home", return_value=Path(directory)), \
                patch.object(host.os, "set_blocking"), patch.object(host.os, "write"):
            logs = Path(directory) / ".local/share/locus/claude-ssh/v1/diagnostics"
            logs.mkdir(parents=True)
            for i in range(host.LIFECYCLE_LIMIT + 3):
                (logs / ("helper-%013d-1.json" % i)).write_text("{}")
            host.initialize_lifecycle()
            host.record_lifecycle("started")
            self.assertEqual(len(list(logs.glob("helper-*.json"))), host.LIFECYCLE_LIMIT)
            host.child_pid = 12345
            host.record_lifecycle("ready")
            record = json.loads(host.lifecycle_path.read_text())
            self.assertEqual(record["claudePid"], 12345)
            if os.name != "nt":
                self.assertEqual(host.lifecycle_path.stat().st_mode & 0o077, 0)

    def test_disk_or_stderr_failure_does_not_break_lifecycle_reporting(self):
        host = self.host
        host.lifecycle = {"helperVersion": "2", "hostPid": os.getpid()}
        host.lifecycle_path = Path("unused")
        with patch.object(host, "private_write", side_effect=OSError("disk unavailable")), \
                patch.object(host.os, "set_blocking"), patch.object(host.os, "write", side_effect=BlockingIOError()):
            host.record_lifecycle("exit", exitCode=1, exitReason="stdin_error")
        self.assertEqual(host.lifecycle["exitReason"], "stdin_error")

    @unittest.skipIf(os.name == "nt", "PTY process groups are POSIX-only")
    def test_owned_child_is_terminated_reaped_and_its_status_recorded(self):
        host = self.host
        child = subprocess.Popen([sys.executable, "-B", "-c", "import time; time.sleep(30)"], start_new_session=True)
        host.child_pid = child.pid
        host.lifecycle = {}
        try:
            host.terminate_child()
            host.reap_child()
            self.assertTrue(host.child_reaped)
            self.assertLess(host.lifecycle["claudeExitCode"], 0)
            child.returncode = host.lifecycle["claudeExitCode"]
            with patch.object(host.os, "killpg") as kill:
                host.terminate_child()
            kill.assert_not_called()
        finally:
            if child.returncode is None:
                child.kill()
                child.wait(timeout=5)

    def test_reaped_children_are_never_signalled_again(self):
        host = self.host
        host.child_pid, host.child_reaped = 12345, True
        with patch.object(host.os, "killpg", create=True) as kill:
            host.terminate_child()
        kill.assert_not_called()


if __name__ == "__main__":
    unittest.main()
