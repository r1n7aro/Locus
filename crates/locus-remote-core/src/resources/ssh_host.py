"""Ephemeral SSH stdio host. Claude owns a real PTY and all model requests.

Only the Mod's same-host loopback HTTP is translated into SSH frames. No port
forwarding, public listener, API proxy, repository copy or system service.
"""
import base64
from collections import deque
from concurrent.futures import ThreadPoolExecutor
import http.server
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import tempfile
import time
import urllib.request
import uuid

PROTOCOL = 1
HELPER_VERSION = "3"
LIFECYCLE_LIMIT = 128
MIN_VERSION = (2, 1, 299)
# Native multi-agent conversations need these Mods hooks (keep in sync with
# claude_code_cli::NATIVE_AGENTS_MIN_VERSION). Automatic setup installs it.
NATIVE_AGENTS_VERSION = (2, 1, 312)
INSTALL_VERSION = "2.1.312"
MAX_FRAME = 32 * 1024 * 1024
output_lock = threading.Lock()
closing = threading.Event()
pending_lock = threading.Lock()
pending = {}
options_lock = threading.Lock()
options = {}
last_contact = time.monotonic()
child_pid = None
child_reaped = False
shutdown_status = None
lifecycle = None
lifecycle_path = None
keychain_unlock_tested = False
archive_lock = threading.Lock()
ARCHIVE_LIMIT = 256 * 1024 * 1024
ARCHIVE_CHUNK = 192 * 1024


def frame(value, announce_history_size=False):
    if lifecycle is not None and value.get("type") in ("progress", "hello", "error", "exit"):
        value = dict(value, helper=dict(lifecycle))
    # ASCII JSON: len is the byte count including LF. Serialize payloads once.
    encoded = json.dumps(value, ensure_ascii=True, separators=(",", ":")) + "\n"
    progress = None
    if announce_history_size:
        progress = {"type": "progress", "protocol": PROTOCOL,
                    "stage": "history_transfer", "bytes": len(encoded)}
    elif value.get("type") in ("archive_chunk", "archive_result"):
        progress = {"type": "progress", "protocol": PROTOCOL, "stage": "archive_transfer",
                    "id": value["id"], "bytes": len(encoded)}
    with output_lock:
        # Marker + payload are one ordered pair: a heartbeat or another lane
        # cannot slip between them and falsely renew a bulk transfer deadline.
        if progress is not None:
            sys.stdout.write(json.dumps(progress, separators=(",", ":")) + "\n")
            sys.stdout.flush()
        sys.stdout.write(encoded)
        sys.stdout.flush()


def read_frame():
    line = sys.stdin.buffer.readline(MAX_FRAME + 1)
    if not line:
        return None
    if len(line) > MAX_FRAME or not line.endswith(b"\n"):
        raise ValueError("SSH frame exceeds the size limit")
    return json.loads(line)


def private_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as file:
        file.write(data)
        file.flush()
        os.fsync(file.fileno())
    os.replace(temporary, path)


class HostSignal(BaseException):
    def __init__(self, signum):
        self.signum = signum


def exception_status(error):
    if isinstance(error, HostSignal):
        return 128 + error.signum, "signal_" + signal.Signals(error.signum).name.lower()
    if isinstance(error, KeyboardInterrupt):
        return 130, "keyboard_interrupt"
    if isinstance(error, SystemExit):
        code = error.code if isinstance(error.code, int) else (0 if error.code is None else 1)
        return code if 0 <= code <= 255 else 1, "system_exit"
    return 1, "stdout_closed" if isinstance(error, BrokenPipeError) else "unhandled_exception"


def request_shutdown(reason, code=0):
    global shutdown_status
    # Keep the initiating reason, not a secondary broken pipe during teardown.
    if shutdown_status is None:
        shutdown_status = (code, reason)
    closing.set()


def handle_signal(signum, _frame):
    # Do not take Python locks inside a signal handler.
    raise HostSignal(signum)


def initialize_lifecycle():
    global lifecycle, lifecycle_path
    source = globals().get("_locus_source")
    if source is None and "__file__" in globals():
        source = Path(__file__).read_bytes()
    started = time.time_ns() // 1000000
    lifecycle = {"helperVersion": HELPER_VERSION,
                 "sourceHash": hashlib.sha256(source).hexdigest() if isinstance(source, bytes) else None,
                 "hostPid": os.getpid(), "claudePid": None, "startedAt": started}
    directory = Path.home() / ".local/share/locus/claude-ssh/v1/diagnostics"
    try:
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        lifecycle_path = directory / ("helper-%d-%d.json" % (started, os.getpid()))
        # Only lifecycle boundaries touch disk; never log traffic or credentials.
        old = sorted(directory.glob("helper-*.json"), key=lambda p: p.name)
        for path in old[:max(0, len(old) - LIFECYCLE_LIMIT + 1)]:
            try:
                path.unlink()
            except OSError:
                pass
    except OSError:
        lifecycle_path = None


def record_lifecycle(event, **fields):
    if lifecycle is None:
        return
    lifecycle.update(fields, event=event, claudePid=child_pid if child_pid and child_pid > 0 else None,
                     updatedAt=time.time_ns() // 1000000)
    encoded = json.dumps(lifecycle, ensure_ascii=True, separators=(",", ":"))
    try:
        if lifecycle_path is not None:
            private_write(lifecycle_path, encoded + "\n")
    except Exception:
        pass
    try:
        # Never acquire Python's buffered stderr lock or wait for a dead SSH peer.
        os.set_blocking(2, False)
        os.write(2, ("LOCUS_SSH_HELPER " + encoded + "\n").encode("ascii"))
    except (OSError, ValueError):
        pass


def cleanup(action):
    try:
        action()
    except BaseException as error:
        if lifecycle is not None:
            lifecycle["cleanupError"] = type(error).__name__[:80]
        request_shutdown("cleanup_error", 1)


def reap_child():
    global child_reaped
    if not child_pid or child_pid < 0 or child_reaped:
        return
    deadline = time.monotonic() + 1
    while True:
        try:
            pid, status = os.waitpid(child_pid, os.WNOHANG)
        except ChildProcessError:
            child_reaped = True
            return
        if pid:
            child_reaped = True
            if lifecycle is not None:
                lifecycle["claudeExitCode"] = os.waitstatus_to_exitcode(status)
            return
        if time.monotonic() >= deadline:
            raise TimeoutError("Child reap deadline expired")
        time.sleep(0.01)


def exit_process(code, reason):
    try:
        # Signals during cleanup must not interrupt the unconditional _exit.
        if threading.current_thread() is threading.main_thread():
            for name in ("SIGINT", "SIGTERM", "SIGHUP"):
                if hasattr(signal, name):
                    signal.signal(getattr(signal, name), signal.SIG_IGN)
        closing.set()
        cleanup(terminate_child)
        cleanup(reap_child)
        if lifecycle is not None and lifecycle.get("cleanupError") and code == 0:
            code = 1
            if reason == "completed":
                reason = "cleanup_error"
        record_lifecycle("exit", exitCode=code, exitReason=reason)
    finally:
        # Includes failed cleanup/logging and live daemon threads in buffered I/O.
        os._exit(code)


def mac_keychain(password=None):
    """Return lock state; optionally unlock through Security.framework, never argv.

    No credential is extracted. This only unlocks the existing login keychain
    in this helper's security session, inherited by its Claude subprocesses.
    """
    if sys.platform != "darwin":
        return None
    import ctypes
    security = ctypes.CDLL("/System/Library/Frameworks/Security.framework/Security")
    core = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    security.SecKeychainSetUserInteractionAllowed.argtypes = [ctypes.c_ubyte]
    security.SecKeychainSetUserInteractionAllowed(False)
    security.SecKeychainOpen.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    security.SecKeychainUnlock.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_ubyte]
    security.SecKeychainGetStatus.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
    core.CFRelease.argtypes = [ctypes.c_void_p]
    keychain = ctypes.c_void_p()
    if security.SecKeychainOpen(os.fsencode(Path.home() / "Library/Keychains/login.keychain-db"), ctypes.byref(keychain)):
        if password is not None:
            raise RuntimeError("Cannot open the remote macOS login keychain")
        return None
    try:
        if password is not None:
            if not isinstance(password, str) or not password or len(password.encode()) > 4096 or "\0" in password:
                raise ValueError("Invalid macOS keychain password")
            raw = bytearray(password.encode("utf-8"))
            password = None
            buffer = (ctypes.c_char * len(raw)).from_buffer(raw)
            try:
                status = security.SecKeychainUnlock(keychain, len(raw), buffer, True)
            finally:
                ctypes.memset(buffer, 0, len(raw))
            if status:
                raise RuntimeError("macOS keychain unlock failed; check the saved unlock password")
        bits = ctypes.c_uint32()
        if security.SecKeychainGetStatus(keychain, ctypes.byref(bits)):
            raise RuntimeError("Cannot check the remote macOS keychain state")
        return bool(bits.value & 1)
    finally:
        core.CFRelease(keychain)


def unlock_for_connection(config):
    global keychain_unlock_tested
    if sys.platform != "darwin" or not config.get("keychainUnlock"):
        return
    frame({"type": "keychain_unlock_required", "protocol": PROTOCOL, "platform": "darwin"})
    # No reader thread exists yet. Consume heartbeats without losing the secret
    # response; abort on EOF or any other protocol message. Never print it.
    while True:
        reply = read_frame()
        if reply and reply.get("type") == "ping":
            frame({"type": "pong", "id": reply.get("id")})
            continue
        break
    if not isinstance(reply, dict) or reply.get("type") != "keychain_unlock":
        raise RuntimeError("Missing macOS keychain unlock response")
    password = reply.pop("password", None)
    try:
        if password is not None:
            if not mac_keychain(password):
                raise RuntimeError("The remote macOS keychain is still locked")
            keychain_unlock_tested = True
    finally:
        password = None
        reply.clear()


def cli_auth_status(cli, env):
    if not cli:
        return False
    try:
        result = subprocess.run([cli, "auth", "status"], env=env, capture_output=True, timeout=20)
        return result.returncode == 0 and json.loads(result.stdout).get("loggedIn") is True
    except (subprocess.TimeoutExpired, ValueError, OSError):
        # Raw CLI output can contain account details. Return status only.
        return False


def apply_runtime_env(env, config):
    settings = config.get("runtimeSettings", {})
    window = settings.get("autoCompactWindow")
    if window is not None:
        if type(window) is not int or not 100_000 <= window <= 1_000_000:
            raise ValueError("Claude Code auto-compact window must be between 100000 and 1000000 tokens")
        # Set this on the remote Claude child, not on the local SSH client.
        # The native override also wins over saved per-model windows.
        env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"] = str(window)
    env.pop("DISABLE_NONESSENTIAL_TRAFFIC", None)
    for key in ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "DISABLE_TELEMETRY", "DISABLE_ERROR_REPORTING"):
        if settings.get("telemetry", False):
            env.pop(key, None)
        else:
            env[key] = "1"


def runtime_cli_settings(config, permission_mode):
    # Keep in sync with RuntimeSettings::cli_settings: refuse other sessions' peers.
    settings = {"idleCompaction": bool(config.get("runtimeSettings", {}).get("idleCompaction", False)),
                "crossSessionInbound": "refuse"}
    if permission_mode == "bypassPermissions":
        settings["skipDangerousModePermissionPrompt"] = True
    return settings


def settings_and_env():
    source = Path(os.environ.get("CLAUDE_CONFIG_DIR", str(Path.home() / ".claude")))
    try:
        settings = json.loads((source / "settings.json").read_text())
    except FileNotFoundError:
        settings = {}
    env = dict(os.environ)
    for key, value in settings.get("env", {}).items():
        if isinstance(value, str) and (key.startswith("ANTHROPIC_") or key in (
            "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_SUBAGENT_MODEL", "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY")):
            env[key] = value
    env["PATH"] = str(Path.home() / ".local/bin") + ":" + env.get("PATH", "/usr/bin:/bin")
    return source, settings, env


def executable_identity(cli):
    path = Path(cli).resolve()
    stat = path.stat()
    return {"path": str(path), "size": stat.st_size, "mtimeNs": stat.st_mtime_ns, "inode": stat.st_ino}


def runtime_manifest():
    return Path.home() / ".local/share/locus/claude-ssh/v1/runtime.json"


def version_of(cli):
    if not cli:
        return "", False
    try:
        cached = json.loads(runtime_manifest().read_text())
        if cached.get("version") == 1 and cached.get("protocol") == PROTOCOL and cached.get("executable") == executable_identity(cli):
            value = cached.get("cliVersion", "")
            match = re.search(r"(\d+)\.(\d+)\.(\d+)", value)
            if match:
                return value, tuple(map(int, match.groups())) >= MIN_VERSION
    except (OSError, ValueError, TypeError):
        pass
    value = subprocess.check_output([cli, "--version"], stderr=subprocess.DEVNULL, timeout=30).decode().strip()
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", value)
    return value, bool(match and tuple(map(int, match.groups())) >= MIN_VERSION)


def inspect(config):
    source, settings, env = settings_and_env()
    apply_runtime_env(env, config)
    cli = config.get("cliPath") or shutil.which("claude", path=env["PATH"])
    if cli and not os.path.isfile(cli):
        raise ValueError("The configured remote Claude executable does not exist")
    # Independent read-only checks, after this connection's keychain unlock.
    # Authentication is never cached across SSH connections.
    with ThreadPoolExecutor(max_workers=2) as checks:
        version_check = checks.submit(version_of, cli)
        auth_check = checks.submit(cli_auth_status, cli, env)
        version, supported = version_check.result()
        authenticated = auth_check.result()
    model = env.get("ANTHROPIC_MODEL") or settings.get("model")
    info = {"pythonInstalled": True, "cliVersion": version or None, "cliSupported": supported,
            "model": model, "authenticated": bool(authenticated), "home": str(Path.home()),
            "platform": sys.platform, "keychainUnlocked": mac_keychain(),
            "keychainUnlockTested": keychain_unlock_tested}
    return cli, source, env, info


def version_at_least(value, minimum):
    match = re.search(r"(\d+)\.(\d+)\.(\d+)", value or "")
    return bool(match and tuple(map(int, match.groups())) >= minimum)


def install_claude(env):
    frame({"type": "progress", "message": "Installing Claude Code " + INSTALL_VERSION})
    # This downloads the official installer only, never a model request.
    if shutil.which("curl", path=env["PATH"]):
        installer = subprocess.check_output(["curl", "-fsSL", "--max-time", "60", "https://claude.ai/install.sh"], env=env, timeout=65)
    else:
        installer = urllib.request.urlopen("https://claude.ai/install.sh", timeout=30).read(2 * 1024 * 1024)
    if len(installer) >= 2 * 1024 * 1024:
        raise RuntimeError("Unexpectedly large Claude installer")
    process = subprocess.run(["bash", "-s", "--", INSTALL_VERSION], input=installer,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=600, env=env)
    if process.returncode:
        raise RuntimeError("Remote Claude installation failed: " + process.stdout.decode(errors="replace")[-1600:])


def prepare(config, mode):
    cli, source, env, info = inspect(config)
    if mode == "inspect":
        return cli, source, env, info
    if not config.get("authorizedSetup"):
        raise PermissionError("Authorize remote environment setup in Locus settings before connecting")
    # A native-agent launch requires NATIVE_AGENTS_VERSION. Setup while native
    # agents are enabled installs it when it can; a failure there keeps a
    # Mods-capable CLI, and new conversations then use Locus subagents.
    launch_native = config.get("nativeAgents") is True
    wants_native = launch_native or config.get("nativeMultiAgent") is True
    native_ready = version_at_least(info["cliVersion"], NATIVE_AGENTS_VERSION)
    if config.get("cliPath") and not info["cliSupported"]:
        raise RuntimeError("The selected Claude executable is too old for Mods. Clear the custom path to install automatically.")
    if config.get("cliPath") and launch_native and not native_ready:
        raise RuntimeError("The selected Claude executable is too old for native agents (Claude Code " + INSTALL_VERSION
                           + " or later). Clear the custom path to install automatically.")
    if not config.get("cliPath") and (not info["cliSupported"] or (wants_native and not native_ready)):
        try:
            install_claude(env)
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
            if not info["cliSupported"] or launch_native:
                raise
            frame({"type": "progress", "message": "Claude Code " + INSTALL_VERSION
                   + " could not be installed; native agents stay off. " + str(error)[-400:]})
        else:
            cli, source, env, info = inspect({**config, "cliPath": None})
            if not info["cliSupported"]:
                raise RuntimeError("The installed CLI does not support the required Mods protocol")
            if launch_native and not version_at_least(info["cliVersion"], NATIVE_AGENTS_VERSION):
                raise RuntimeError("The installed Claude Code does not support native agents")
    root = Path.home() / ".local/share/locus/claude-ssh/v1"
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    if os.path.commonpath([str(root.resolve()), str(Path.home().resolve())]) != str(Path.home().resolve()):
        raise PermissionError("Remote runtime must remain inside this user's home")
    private_write(root / "runtime.json", json.dumps({"version": 1, "protocol": PROTOCOL,
                  "cliVersion": info["cliVersion"], "executable": executable_identity(cli)}))
    return cli, source, env, info


def validate_plugin(cli, plugin, env):
    # Reject a broken Mod before starting a TUI that can never report ready.
    # This command validates code only; it sends no prompt or model request.
    try:
        result = subprocess.run([cli, "plugin", "validate", str(plugin)], env={**env, "NO_COLOR": "1"},
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=20)
    except subprocess.TimeoutExpired:
        raise RuntimeError("Claude plugin validation timed out") from None
    if result.returncode:
        detail = result.stdout.decode(errors="replace")[-8000:].strip()
        raise RuntimeError("Claude plugin validation failed." + ("\n" + detail if detail else ""))


def options_view(value, view):
    if view == "input":
        result = {key: value.get(key) for key in ("active", "inputId", "terminalText", "imageCount")}
        if value.get("insertion"):
            result["insertion"] = {key: value["insertion"].get(key) for key in ("inputId", "terminalText", "imageCount")}
        # Same as mod_bridge::options_view: user stops of native subagents.
        if isinstance(value.get("agentControl"), dict):
            result["agentControl"] = value["agentControl"]
        return result
    if view == "prompt":
        return {"systemPrompt": value.get("systemPrompt")}
    if view == "session":
        return {key: value.get(key) for key in ("active", "attachmentMappings")}
    return value


ASYNC_EVENTS = frozenset(("text", "thinking", "tool_start", "step_start"))
# Native subagent registration, routing and stops are answered at the Locus
# host's ingress (or by its agent pump): never behind a lane's tool call.
CONTROL_EVENTS = frozenset(("input_ready", "input_images", "input_staging_error", "input_queued", "input_consumed", "turn_interrupted", "bridge_health", "bridge_fault",
                            "agent_spawn", "agent_finished", "agent_route", "agent_notification", "control_done"))
PENDING_LIMIT = 256


class OrderedBridgeQueue:
    """Accept display deltas locally; confirm their delivery in FIFO order.

    Adjacent text is coalesced while the previous delivery crosses SSH. Critical
    calls share this queue, so tools/checkpoints/completion cannot overtake it.
    Input/interrupt controls bypass it to interrupt a tool awaiting its result.
    Native JSONL remains the durable source on reconnect. Acceptance here never
    acknowledges a tool, checkpoint, or completed turn before Locus persists it.
    """
    MAX_BYTES = 4 * 1024 * 1024
    MAX_ITEMS = 512
    MERGE_BYTES = 64 * 1024

    def __init__(self):
        self.condition = threading.Condition()
        self.items = deque()
        self.bytes = 0
        self.active = None
        self.error = None

    @staticmethod
    def size(payload):
        return len(json.dumps(payload, ensure_ascii=True, separators=(",", ":")))

    def submit(self, payload, asynchronous=False):
        if asynchronous and payload.get("type") not in ASYNC_EVENTS:
            raise ValueError("Only display events may be acknowledged before delivery")
        size = self.size(payload)
        with self.condition:
            if self.error or closing.is_set():
                raise RuntimeError("SSH event queue is closed")
            previous = self.items[-1] if self.items else None
            merged = None
            if asynchronous and payload.get("type") in ("text", "thinking") and previous and previous["async"]:
                before = previous["payload"]
                if {k: v for k, v in before.items() if k != "text"} == {k: v for k, v in payload.items() if k != "text"}:
                    candidate = {**before, "text": before["text"] + payload["text"]}
                    merged_size = self.size(candidate)
                    if merged_size <= self.MERGE_BYTES:
                        merged = candidate, merged_size
            increase = merged[1] - previous["size"] if merged else size
            if self.bytes + increase > self.MAX_BYTES or (not merged and len(self.items) + bool(self.active) >= self.MAX_ITEMS):
                raise BufferError("SSH display queue exceeded its limit; resume from native history")
            if merged:
                previous["payload"], previous["size"] = merged
                self.bytes += increase
                return None
            item = {"payload": dict(payload), "size": size, "async": asynchronous,
                    "ticket": uuid.uuid4().hex, "receipt": {"event": threading.Event(), "value": None}}
            if not asynchronous:
                with pending_lock:
                    if len(pending) >= PENDING_LIMIT:
                        raise BufferError("Too many pending critical bridge calls")
                    pending[item["ticket"]] = item["receipt"]
            self.items.append(item)
            self.bytes += size
            self.condition.notify()
            return item

    def fail(self, message):
        with self.condition:
            self.error = message
            for item in [*self.items, *([self.active] if self.active else [])]:
                if not item["receipt"]["event"].is_set():
                    item["receipt"]["value"] = {"bridgeError": message}
                    item["receipt"]["event"].set()
                if item["async"]:
                    with pending_lock:
                        pending.pop(item["ticket"], None)
            self.items.clear()
            self.bytes = 0
            self.condition.notify_all()

    def run(self):
        try:
            while not closing.is_set():
                with self.condition:
                    if not self.items:
                        self.condition.wait(0.1)
                        continue
                    item = self.items.popleft()
                    self.active = item
                with pending_lock:
                    pending[item["ticket"]] = item["receipt"]
                frame({"type": "call", "id": item["ticket"], "payload": item["payload"]})
                while not item["receipt"]["event"].wait(0.1):
                    if closing.is_set():
                        raise RuntimeError("SSH disconnected before event delivery")
                value = item["receipt"]["value"]
                if item["async"]:
                    with pending_lock:
                        pending.pop(item["ticket"], None)
                    if not isinstance(value, dict) or value.get("ok") is not True:
                        raise RuntimeError("Locus did not accept an ordered display event")
                elif isinstance(value, dict) and "bridgeError" in value:
                    raise RuntimeError("Locus bridge closed before a critical acknowledgement")
                with self.condition:
                    if self.error:
                        raise RuntimeError(self.error)
                    self.bytes -= item["size"]
                    self.active = None
        except Exception:
            closing.set()
        finally:
            self.fail("SSH event delivery stopped; resume from native history")


class BridgeLanes:
    """One ordered lane per agent (the main agent, and each native subagent by
    agentId). Each lane keeps its FIFO order, but a long tool call in one never
    holds up another: Locus serves subagents in parallel with the main agent.
    """
    MAX_LANES = 32

    def __init__(self):
        self.lock = threading.Lock()
        self.lanes = {}

    def lane(self, payload):
        agent = payload.get("agentId")
        key = agent if isinstance(agent, str) and agent else ""
        with self.lock:
            queue = self.lanes.get(key)
            if queue is None:
                if len(self.lanes) >= self.MAX_LANES:
                    raise BufferError("Too many concurrent Claude subagents")
                queue = self.lanes[key] = OrderedBridgeQueue()
                threading.Thread(target=queue.run, daemon=True).start()
            return queue

    def submit(self, payload, asynchronous=False):
        return self.lane(payload).submit(payload, asynchronous)

    def fail(self, message):
        with self.lock:
            lanes = list(self.lanes.values())
        for queue in lanes:
            queue.fail(message)


class Bridge(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # Headers and tiny JSON bodies are separate writes. Nagle + delayed ACK
    # otherwise adds about 40 ms to every streamed token callback.
    disable_nagle_algorithm = True

    def log_message(self, *_args):
        pass

    def respond(self, status, value):
        data = json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        try:
            if self.headers.get("Authorization") != "Bearer " + self.server.token:
                return self.respond(401, {"error": "Unauthorized"})
            if self.headers.get("Origin") or self.headers.get("Host") != self.server.authority:
                return self.respond(403, {"error": "Invalid origin or host"})
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 1024 * 1024:
                return self.respond(413, {"error": "Invalid bridge body size"})
            payload = json.loads(self.rfile.read(length))
            if payload.get("protocol") != PROTOCOL or closing.is_set():
                return self.respond(410, {"error": "SSH bridge is closed or incompatible"})
            if self.path == "/options":
                with options_lock:
                    return self.respond(200, options_view(options, payload.get("view")))
            if self.path == "/poll":
                ticket = payload.get("ticket")
                with pending_lock:
                    receipt = pending.get(ticket)
                if not receipt:
                    return self.respond(410, {"error": "Receipt unavailable; do not resubmit"})
                return self.finish_receipt(ticket, receipt, 10)
            if self.path not in ("/ready", "/event", "/call"):
                return self.respond(404, {"error": "Unknown bridge endpoint"})
            if self.path == "/call" and (not isinstance(payload.get("callId"), str) or not isinstance(payload.get("input"), dict)):
                return self.respond(400, {"error": "Invalid tool envelope"})
            if self.path == "/ready":
                native_id = str(uuid.UUID(payload["sessionId"]))
                self.server.native_id = native_id
                private_write(self.server.session_root / "native-session.json", json.dumps({"version": 1, "nativeId": native_id}))
                payload = {"ready": True, "sessionId": payload.get("sessionId"), "model": payload.get("model")}
            if self.path == "/call":
                record_dispatch(self.server.session_root, self.server.native_id, payload["callId"])
            if self.path == "/event" and payload.get("type") == "bridge_health":
                with options_lock:
                    if not options.get("active") or payload.get("runId") != options.get("inputId"):
                        return self.respond(409, {"error": "Stale Claude bridge health"})
                if payload.get("state") not in ("running", "complete"):
                    return self.respond(400, {"error": "Invalid Claude bridge health"})
                # Health is a disposable sample, not a durable tool outcome.
                # Awaiting its SSH receipt can strand reportingHealth in the Mod
                # and suppress all future samples, even after traffic resumes.
                # Forward every fresh sample without consuming a tool receipt;
                # only the receiving Rust watchdog can renew the actual lease.
                frame({"type": "bridge_health", "payload": payload})
                return self.respond(200, {"ok": True})
            # Worker restart and fault receipts must bypass a long-running tool
            # in the ordered lane, otherwise the host cannot retire a lost Mod.
            if not (self.path == "/ready" or (self.path == "/event" and payload.get("type") in CONTROL_EVENTS)):
                asynchronous = self.path == "/event" and payload.get("type") in ASYNC_EVENTS
                if asynchronous:
                    with options_lock:
                        if not options.get("active") or payload.get("runId") != options.get("inputId"):
                            return self.respond(409, {"error": "Stale display event"})
                    if payload.get("type") in ("text", "thinking") and not isinstance(payload.get("text"), str):
                        return self.respond(400, {"error": "Invalid display text"})
                item = self.server.events.submit(payload, asynchronous)
                if asynchronous:
                    return self.respond(200, {"ok": True})
                return self.finish_receipt(item["ticket"], item["receipt"], 0.1)
            ticket = uuid.uuid4().hex
            receipt = {"event": threading.Event(), "value": None}
            with pending_lock:
                if len(pending) >= PENDING_LIMIT:
                    return self.respond(503, {"error": "Too many pending calls"})
                pending[ticket] = receipt
            frame({"type": "call", "id": ticket, "payload": payload})
            self.finish_receipt(ticket, receipt, 0.1)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except (BufferError, RuntimeError):
            closing.set()
            self.server.events.fail("SSH event queue unavailable")
            self.respond(503, {"error": "SSH event queue unavailable; resume from native history"})
        except Exception:
            self.respond(400, {"error": "Invalid local bridge request"})

    def finish_receipt(self, ticket, receipt, timeout):
        if not receipt["event"].wait(timeout):
            return self.respond(202, {"pending": ticket})
        with pending_lock:
            if pending.pop(ticket, None) is None:
                return self.respond(410, {"error": "Receipt already consumed"})
        value = receipt["value"]
        self.respond(503 if isinstance(value, dict) and "bridgeError" in value else 200, value)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 16
    # Each waiting Mod hook holds one request; native subagents add their own.
    slots = threading.BoundedSemaphore(96)

    def process_request(self, request, client_address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()

    def get_request(self):
        socket, address = super().get_request()
        socket.settimeout(20)
        return socket, address


def terminate_child():
    if child_pid is not None and child_pid > 0 and not child_reaped:
        try:
            os.killpg(child_pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        # Kill the owned PTY group even when a child ignores SIGTERM.
        time.sleep(0.15)
        try:
            os.killpg(child_pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def stdin_loop(master, root):
    global options, last_contact
    try:
        while not closing.is_set():
            value = read_frame()
            if value is None:
                request_shutdown("stdin_eof")
                break
            last_contact = time.monotonic()
            kind = value.get("type")
            if kind == "configure":
                uploads = value.get("uploads", [])
                if uploads:
                    import re
                    input_id = value.get("uploadId") or value["options"].get("inputId", "")
                    if not re.fullmatch(r"LOCUS_INPUT_[a-f0-9]{32}", input_id):
                        raise ValueError("Invalid attachment input ID")
                    directory = root / "attachments" / input_id
                    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
                    total = 0
                    for attachment in uploads:
                        name = attachment["name"]
                        if not re.fullmatch(r"(?:image|file)-[0-9]+\.(?:png|jpg|gif|webp|txt)", name):
                            raise ValueError("Invalid attachment filename")
                        data = base64.b64decode(attachment["data"], validate=True)
                        total += len(data)
                        if total > 20 * 1024 * 1024:
                            raise ValueError("Attachments exceed 20 MiB")
                        target = directory / name
                        with target.open("xb") as output:
                            output.write(data)
                with options_lock:
                    options = value["options"]
            elif kind == "input":
                data = base64.b64decode(value["data"], validate=True)
                while data:
                    data = data[os.write(master, data):]
            elif kind == "reply":
                with pending_lock:
                    receipt = pending.get(value["id"])
                    if receipt:
                        receipt["value"] = value["value"]
                        receipt["event"].set()
            elif kind == "ping":
                frame({"type": "pong", "id": value.get("id")})
            elif kind == "archive":
                threading.Thread(target=archive_download, args=(root, value), daemon=True).start()
            elif kind == "close":
                request_shutdown("client_close")
                break
            else:
                raise ValueError("Unknown SSH frame")
    except BaseException as error:
        if lifecycle is not None:
            lifecycle["errorType"] = type(error).__name__[:80]
        request_shutdown("stdin_error", 1)
    finally:
        closing.set()


def watchdog():
    # Independent of stdout: neither reporting nor cleanup may bypass _exit.
    try:
        while not closing.wait(0.5):
            if time.monotonic() - last_contact > 25:
                request_shutdown("heartbeat_timeout", 1)
                return
        # Give the main thread its bounded cleanup window, even if stdout stalls.
        time.sleep(2)
    except BaseException as error:
        if lifecycle is not None:
            lifecycle["errorType"] = type(error).__name__[:80]
        request_shutdown("watchdog_error", 1)
    finally:
        code, reason = shutdown_status or (1, "watchdog_error")
        exit_process(code, reason)


def native_storage(root, shared_profile=None):
    """Bind native history to its real profile and exact project, never glob
    all projects in a shared profile (forks can contain the same source UUID).
    Existing isolated Linux/legacy sessions retain their recorded layout.
    """
    path = root / "native-storage.json"
    project_name = re.sub(r"[^a-zA-Z0-9]", "-", str(root / "terminal"))
    if path.is_file():
        binding = json.loads(path.read_text())
        if binding.get("version") != 1:
            raise ValueError("Unsupported Claude native storage binding")
        return Path(binding["profile"]), Path(binding["project"])
    profile = root / "profile"
    if shared_profile is not None:
        if any((profile / "projects").glob("*/*.jsonl")):
            raise RuntimeError("This legacy isolated session cannot switch to shared macOS authentication; start a new conversation")
        profile = shared_profile.expanduser().absolute()
    project = profile / "projects" / project_name
    private_write(path, json.dumps({"version": 1, "profile": str(profile), "project": str(project)}))
    return profile, project


def native_transcripts(root, native_id):
    binding = root / "native-storage.json"
    if binding.is_file():
        _, project = native_storage(root)
        path = project / (native_id + ".jsonl")
        return [path] if path.is_file() else []
    return list((root / "profile/projects").glob("*/" + native_id + ".jsonl"))


def history_transcripts(root, native_id):
    """Read a prepared fork before Claude has materialized its new identity.

    Only the frozen transcript in THIS profile is eligible. Looking up the
    original session would include turns written after the fork boundary.
    Do not use this fallback for restore destinations or CLI resume selection:
    --fork-session still owns creation of the child transcript.
    """
    paths = native_transcripts(root, native_id)
    if paths or not (root / "fork.json").is_file():
        return paths
    state = json.loads((root / "fork.json").read_text())
    if state.get("version") != 1 or state.get("new_id") != native_id:
        raise ValueError("Invalid prepared Claude fork identity")
    source_id = str(uuid.UUID(state["source_id"]))
    if source_id == native_id:
        raise ValueError("Invalid prepared Claude fork source")
    return native_transcripts(root, source_id)


def is_queued_input(row):
    attachment = row.get("attachment", {})
    return (row.get("type") == "attachment" and attachment.get("type") == "queued_command"
            and (attachment.get("humanTurn") is True or attachment.get("origin", {}).get("kind") == "human")
            and isinstance(attachment.get("prompt"), str))


def queued_input_content(row):
    content = [{"type": "text", "text": row["attachment"]["prompt"]}]
    for rendered in row.get("rendered", []):
        blocks = rendered.get("content", [])
        if isinstance(blocks, list):
            content.extend(block for block in blocks if isinstance(block, dict) and block.get("type") == "image")
    return content


def hidden_native_user(row):
    """Display-only classification; mirror recovery.rs and its shared fixtures."""
    if row.get("type") != "user" or row.get("isCompactSummary") is True:
        return False
    content = row.get("message", {}).get("content", [])
    if isinstance(content, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content):
        return False
    text = content if isinstance(content, str) else "\n".join(b["text"] for b in content if isinstance(b, dict) and isinstance(b.get("text"), str)) if isinstance(content, list) else ""
    pending = re.match(r'^<locus-pending-user id="([0-9a-fA-F-]{36})">\n', text)
    if pending:
        uuid.UUID(pending[1])
        return False
    if any(row.get(key) is True for key in ("isMeta", "isVisibleInTranscriptOnly", "isVirtual")):
        return True
    if (row.get("origin") or {}).get("kind") == "human":
        return False
    return text.lstrip().startswith(("<command-name>", "<command-message>", "<command-args>",
        "<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>",
        "<bash-input>", "<bash-stdout>", "<bash-stderr>", "<bash-exit-code>", "<task-notification>"))


def canonical_history(rows, hidden_user_uuids=None):
    """Follow the active native branch, including parallel tool-result siblings.
    Assistant content blocks are separate JSONL rows with one API message id.
    """
    nodes = {r["uuid"]: (i, r) for i, r in enumerate(rows) if r.get("uuid") and not r.get("isSidechain")}
    if len(nodes) != sum(bool(r.get("uuid")) and not r.get("isSidechain") for r in rows):
        raise ValueError("Duplicate native message UUIDs; automatic recovery stopped")
    leaves = [r["uuid"] for r in rows if r.get("uuid") in nodes and (r.get("type") in ("user", "assistant") or is_queued_input(r))]
    if not leaves:
        return []
    included = set()
    def ancestors(node):
        visited = set()
        while node:
            if node in visited:
                raise ValueError("Claude transcript contains a parent cycle")
            if node in included:
                break
            if node not in nodes:
                raise ValueError("Claude transcript has a missing parent; automatic recovery stopped")
            visited.add(node)
            _, row = nodes[node]
            included.add(node)
            parent = row.get("parentUuid")
            logical = row.get("logicalParentUuid")
            # On resume Claude may rewrite the file to summary + preserved
            # segment; logicalParentUuid then points forward, not to old data.
            if not parent and row.get("subtype") == "compact_boundary" and logical in nodes and nodes[logical][0] < nodes[node][0]:
                parent = logical
            node = parent
    ancestors(leaves[-1])
    # A result for parallel call A need not be an ancestor of call B's result.
    # Include only siblings belonging to selected assistant API messages.
    while True:
        previous = len(included)
        message_ids = {nodes[node][1].get("message", {}).get("id") for node in included if nodes[node][1].get("type") == "assistant"}
        message_ids.discard(None)
        tools = set()
        for node, (index, row) in nodes.items():
            if index > nodes[leaves[-1]][0]:
                continue
            if row.get("type") == "assistant" and row.get("message", {}).get("id") in message_ids:
                ancestors(node)
                for block in row["message"].get("content", []):
                    if isinstance(block, dict) and block.get("type") == "tool_use":
                        tools.add(block["id"])
        for node, (index, row) in nodes.items():
            content = row.get("message", {}).get("content", [])
            if index <= nodes[leaves[-1]][0] and isinstance(content, list) and any(isinstance(b, dict) and b.get("tool_use_id") in tools for b in content):
                ancestors(node)
        if len(included) == previous:
            break
    messages, assistant_groups = [], {}
    for node, (_, row) in sorted(nodes.items(), key=lambda item: item[1][0]):
        if node not in included:
            continue
        if is_queued_input(row):
            messages.append({"key": node, "role": "user", "uuids": [node], "content": queued_input_content(row),
                             "timestamp": row.get("timestamp"), "queuedInput": True})
            continue
        if row.get("type") not in ("user", "assistant"):
            continue
        message = row.get("message", {})
        if row.get("isApiErrorMessage") is True or message.get("model") == "<synthetic>":
            continue
        content = message.get("content", [])
        if isinstance(content, str):
            content = [{"type": "text", "text": content}]
        if not isinstance(content, list):
            raise ValueError("Unsupported Claude native message content")
        text = "".join(b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text")
        pending_input = re.match(r'^<locus-pending-user id="([0-9a-fA-F-]{36})">\n', text)
        if pending_input:
            uuid.UUID(pending_input[1])
        # Retain hidden rows in the ancestry graph, not as human chat bubbles.
        if hidden_native_user(row):
            if hidden_user_uuids is not None:
                hidden_user_uuids.append(node)
            continue
        if row["type"] != "user" and row.get("isMeta") and not row.get("isCompactSummary") and not pending_input:
            continue
        if row["type"] == "assistant":
            key = "assistant:" + str(message.get("id") or node)
            entry = assistant_groups.get(key)
            if entry is None:
                entry = {"key": key, "role": "assistant", "uuids": [], "content": [], "parts": [], "timestamp": row.get("timestamp"), "model": message.get("model"), "usage": None}
                assistant_groups[key] = entry
                messages.append(entry)
            entry["uuids"].append(node)
            entry["parts"].append({"uuid": node, "content": content, "timestamp": row.get("timestamp")})
            entry["content"].extend(content)
            if message.get("usage"):
                entry["usage"] = message["usage"]
            if isinstance(message.get("stop_reason"), str):
                entry["stopReason"] = message["stop_reason"]
        else:
            results = [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"]
            if results:
                for block in results:
                    messages.append({"key": node + ":" + block["tool_use_id"], "role": "tool", "uuids": [node], "content": block.get("content", ""), "toolCallId": block["tool_use_id"], "isError": bool(block.get("is_error")), "timestamp": row.get("timestamp")})
            else:
                messages.append({"key": node, "role": "user", "uuids": [node], "content": content, "timestamp": row.get("timestamp"), "compactSummary": bool(row.get("isCompactSummary"))})
    return messages


def record_dispatch(root, native_id, call_id):
    # Claude can invoke a tool before its assistant JSONL row is flushed. Persist
    # this marker before sending work over SSH; it is never a synthetic message.
    key = hashlib.sha256(call_id.encode()).hexdigest()
    path = root / "tool-dispatch" / str(uuid.UUID(native_id)) / (key + ".json")
    if path.exists():
        prior = json.loads(path.read_text())
        if prior.get("callId") != call_id or prior.get("version") not in (1, 2):
            raise ValueError("Invalid previous tool dispatch receipt")
        return
    private_write(path,
                  json.dumps({"version": 1, "callId": call_id}))


def settle_dispatches(root, native_id, transcript):
    # A result proven present in native JSONL remains confirmed even if a later
    # native compaction prunes the old rows. This never invents a tool result.
    results = {}
    for line in transcript.splitlines():
        row = json.loads(line)
        content = row.get("message", {}).get("content", [])
        if row.get("type") == "user" and isinstance(content, list):
            for block in content:
                if isinstance(block, dict) and block.get("type") == "tool_result":
                    results[block["tool_use_id"]] = hashlib.sha256(json.dumps(block, sort_keys=True).encode()).hexdigest()
    for path in (root / "tool-dispatch" / native_id).glob("*.json"):
        record = json.loads(path.read_text())
        call_id = record.get("callId")
        if call_id in results and record.get("settled") is not True:
            private_write(path, json.dumps({"version": 2, "callId": call_id, "settled": True, "resultHash": results[call_id]}))


def archive_name(name):
    if name == "transcript.jsonl":
        return True
    return bool(re.fullmatch(r"(?:tasks/[a-zA-Z0-9_-]+\.json|dispatch/[a-f0-9]+\.json|subagents/agent-[a-zA-Z0-9_-]{1,128}\.(?:jsonl|meta\.json)|attachments/LOCUS_INPUT_[a-f0-9]{32}/(?:image|file)-[0-9]+\.(?:png|jpg|gif|webp|txt))", name))


def archive_sources(root, native_id, allow_missing=False, frozen_fork=False):
    paths = history_transcripts(root, native_id) if frozen_fork else native_transcripts(root, native_id)
    if len(paths) != 1 and not (allow_missing and not paths):
        raise ValueError("The bound native transcript is unavailable")
    profile, project = native_storage(root)
    for path in paths:
        safe_archive_path(path, project)
    files = {"transcript.jsonl": paths[0]} if paths else {}
    # Native subagent transcripts live beside the transcript they belong to.
    subagents = paths[0].parent / paths[0].stem / "subagents" if paths else project / native_id / "subagents"
    for prefix, directory, pattern in [("tasks", profile / "tasks" / native_id, "*.json"),
                                       ("dispatch", root / "tool-dispatch" / native_id, "*.json"),
                                       ("subagents", subagents, "agent-*"),
                                       ("attachments", root / "attachments", "*/*")]:
        for path in directory.glob(pattern):
            name = prefix + "/" + path.relative_to(directory).as_posix()
            if path.is_file() and archive_name(name):
                safe_archive_path(path, directory)
                files[name] = path
    if len(files) > 10000:
        raise ValueError("Too many native archive files")
    return files


def safe_archive_path(path, directory):
    if directory.resolve() not in path.resolve().parents:
        raise ValueError("Native archive cannot follow external file links")
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError("Native archive cannot follow file links")
        if part == directory:
            break


def stable_archive_bytes(path, transcript=False):
    if path.stat().st_size > ARCHIVE_LIMIT:
        raise ValueError("Native archive file exceeds 256 MiB")
    original = path.stat()
    data = path.read_bytes()
    # A growing transcript is safe up to its last complete record. Never commit
    # a partial JSONL line, nor mistake an in-place rewrite for an append.
    if transcript and not data.endswith(b"\n"):
        data = data[:data.rfind(b"\n") + 1]
    with path.open("rb") as check:
        same_prefix = check.read(len(data)) == data
    if not same_prefix or original.st_ino != path.stat().st_ino:
        raise RuntimeError("Native file changed while reading; retain the previous backup")
    return data


def archive_download(root, request):
    ticket = request.get("id")
    if not archive_lock.acquire(blocking=False):
        frame({"type": "archive_error", "id": ticket, "message": "Native archive synchronization is busy"})
        return
    try:
        native_id = str(uuid.UUID(request["nativeId"]))
        cursor = request.get("cursor", {})
        if not isinstance(cursor, dict) or len(cursor) > 10000:
            raise ValueError("Invalid native archive cursor")
        sources = archive_sources(root, native_id, frozen_fork=True)
        transcript = stable_archive_bytes(sources["transcript.jsonl"], True)
        checkpoint = request.get("checkpoint")
        if checkpoint and not any(json.loads(line).get("uuid") == checkpoint for line in transcript.splitlines()):
            raise RuntimeError("Native transcript has not flushed the expected checkpoint")
        settle_dispatches(root, native_id, transcript)
        manifest, changes, total, transferred, incomplete = {}, [], 0, 0, False
        for name, path in sorted(sources.items()):
            # A running subagent's transcript also grows record by record.
            data = transcript if name == "transcript.jsonl" else stable_archive_bytes(path, name.endswith(".jsonl"))
            if name == "transcript.jsonl":
                incomplete = path.stat().st_size > len(data)
            total += len(data)
            if total > ARCHIVE_LIMIT:
                raise ValueError("Native archive exceeds 256 MiB")
            digest = hashlib.sha256(data).hexdigest()
            manifest[name] = {"size": len(data), "sha256": digest}
            previous = cursor.get(name, {})
            if previous == manifest[name]:
                continue
            offset = 0
            old_size = previous.get("size", 0)
            if name == "transcript.jsonl" and isinstance(old_size, int) and 0 < old_size <= len(data) and hashlib.sha256(data[:old_size]).hexdigest() == previous.get("sha256"):
                offset = old_size
            changes.append({"name": name, "offset": offset, "baseHash": previous.get("sha256") if offset else None})
            for start in range(offset, len(data), ARCHIVE_CHUNK):
                chunk = data[start:start + ARCHIVE_CHUNK]
                frame({"type": "archive_chunk", "id": ticket, "name": name, "data": base64.b64encode(chunk).decode()})
                transferred += len(chunk)
        frame({"type": "archive_result", "id": ticket, "value": {"version": 1, "nativeId": native_id,
               "sourceRoot": str(root), "manifest": manifest, "changes": changes, "transferredBytes": transferred, "incompleteTail": incomplete}})
    except Exception as error:
        frame({"type": "archive_error", "id": ticket, "message": str(error)})
    finally:
        archive_lock.release()


def restore_path(root, native_id, name):
    if not archive_name(name):
        raise ValueError("Invalid native restore filename")
    profile, project = native_storage(root)
    if name == "transcript.jsonl":
        target, boundary = project / (native_id + ".jsonl"), project
    elif name.startswith("tasks/"):
        target, boundary = profile / "tasks" / native_id / name.split("/", 1)[1], profile
    elif name.startswith("dispatch/"):
        target, boundary = root / "tool-dispatch" / native_id / name.split("/", 1)[1], root
    elif name.startswith("subagents/"):
        target, boundary = project / native_id / "subagents" / name.split("/", 1)[1], project
    else:
        target, boundary = root / name, root
    safe_archive_path(target, boundary)
    return target


def restore_prepare(root, config, request):
    if config.get("authorizedSetup") is not True:
        raise PermissionError("Authorize this target before restoring a native archive")
    native_id = str(uuid.UUID(request["nativeId"]))
    manifest, expected = request["manifest"], request.get("expected", {})
    if not isinstance(manifest, dict) or not isinstance(expected, dict) or len(manifest) > 10000 or "transcript.jsonl" not in manifest:
        raise ValueError("Invalid native restore manifest")
    if any(not archive_name(name) or not isinstance(item, dict) or not isinstance(item.get("size"), int) or item["size"] < 0
           or not re.fullmatch(r"[a-f0-9]{64}", item.get("sha256", "")) for name, item in manifest.items()):
        raise ValueError("Invalid native restore file descriptor")
    if sum(item["size"] for item in manifest.values()) > ARCHIVE_LIMIT:
        raise ValueError("Native restore exceeds 256 MiB")
    shared = Path(os.environ.get("CLAUDE_CONFIG_DIR", str(Path.home() / ".claude"))) if sys.platform == "darwin" else None
    native_storage(root, shared)
    current, data = {}, {}
    for name, path in archive_sources(root, native_id, allow_missing=True).items():
        # Restoration must not overwrite a growing or torn target transcript.
        raw = stable_archive_bytes(path)
        if name == "transcript.jsonl" and raw and not raw.endswith(b"\n"):
            raise RuntimeError("The target native transcript has an incomplete record")
        current[name] = {"size": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
        data[name] = raw
        if current[name] != expected.get(name) and current[name] != manifest.get(name):
            raise RuntimeError("The target conversation has unrecognized changes; it was not overwritten")
    staging = tempfile.TemporaryDirectory(prefix="restore-", dir=root)
    needs = []
    for name, descriptor in manifest.items():
        target = restore_path(root, native_id, name)
        if target.is_symlink():
            raise ValueError("Native restore cannot replace file links")
        if current.get(name) == descriptor:
            continue
        offset = 0
        old = data.get(name, b"")
        if name == "transcript.jsonl" and old and len(old) <= descriptor["size"] and request.get("prefixHashes", {}).get(name) == hashlib.sha256(old).hexdigest():
            offset = len(old)
        staged = Path(staging.name) / name
        staged.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        staged.write_bytes(old[:offset])
        os.chmod(staged, 0o600)
        needs.append({"name": name, "offset": offset})
    return {"id": str(uuid.uuid4()), "nativeId": native_id, "manifest": manifest,
            "current": current, "staging": staging, "needs": needs, "received": 0}


def restore_chunk(state, request):
    if not state or request.get("uploadId") != state["id"]:
        raise ValueError("Invalid native restore upload")
    name = request.get("name")
    if not any(item["name"] == name for item in state["needs"]):
        raise ValueError("Unrequested native restore file")
    data = base64.b64decode(request["data"], validate=True)
    path = Path(state["staging"].name) / name
    if len(data) > ARCHIVE_CHUNK or path.stat().st_size + len(data) > state["manifest"][name]["size"]:
        raise ValueError("Invalid native restore chunk size")
    with path.open("ab") as file:
        file.write(data)
    state["received"] += len(data)


def restore_commit(root, state, request):
    if not state or request.get("uploadId") != state["id"]:
        raise ValueError("Invalid native restore commit")
    native_id = state["nativeId"]
    now = {}
    for name, path in archive_sources(root, native_id, allow_missing=True).items():
        data = stable_archive_bytes(path)
        now[name] = {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    if now != state["current"]:
        raise RuntimeError("The target conversation changed during upload; restore was not committed")
    for needed in state["needs"]:
        name = needed["name"]
        path = Path(state["staging"].name) / name
        data = path.read_bytes()
        expected = state["manifest"][name]
        if len(data) != expected["size"] or hashlib.sha256(data).hexdigest() != expected["sha256"]:
            raise ValueError("Native restore checksum mismatch")
        if name == "transcript.jsonl" and data and not data.endswith(b"\n"):
            raise ValueError("Native restore contains a torn transcript")
        with path.open("r+b") as file:
            os.fsync(file.fileno())
    # Publish transcript last. No CLI is started until the whole upload is
    # acknowledged and the local database changes its active server binding.
    for needed in sorted(state["needs"], key=lambda item: item["name"] == "transcript.jsonl"):
        name = needed["name"]
        target = restore_path(root, native_id, name)
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.replace(Path(state["staging"].name) / name, target)
    for name in set(state["current"]) - set(state["manifest"]):
        restore_path(root, native_id, name).unlink()
    private_write(root / "native-session.json", json.dumps({"version": 1, "nativeId": native_id}))
    return {"nativeId": native_id, "transferredBytes": state["received"], "sourceRoot": str(root), "manifest": state["manifest"]}


def archive_read_listener():
    """Reusable download-only connection. Every request owns its session scope.

    Never start Claude, unlock its keychain, or accept restore/write operations.
    Existing dispatch settlement inside archive_download remains part of backup.
    """
    frame({"type": "hello", "protocol": PROTOCOL, "info": {"pythonInstalled": True,
           "platform": sys.platform, "cliSupported": False, "authenticated": False}})
    while True:
        request = read_frame()
        if request is None or request.get("type") == "close":
            request_shutdown("stdin_eof" if request is None else "client_close")
            return
        if request.get("type") == "ping":
            frame({"type": "pong", "id": request.get("id")})
            continue
        try:
            if request.get("type") != "archive":
                raise ValueError("Download connections do not accept restore operations")
            key = str(uuid.UUID(request["sessionKey"]))
            uuid.UUID(request["nativeId"])
            root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / key
            archive_download(root, request)
        except (KeyError, TypeError, ValueError, AttributeError):
            frame({"type": "archive_error", "id": request.get("id"), "message": "Invalid archive download request"})


def archive_listener(config):
    root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / str(uuid.UUID(config["sessionKey"]))
    frame({"type": "hello", "protocol": PROTOCOL, "info": {"pythonInstalled": True,
           "platform": sys.platform, "sessionRoot": str(root), "cliSupported": False, "authenticated": False}})
    restore = None
    try:
        while True:
            request = read_frame()
            if request is None or request.get("type") == "close":
                request_shutdown("stdin_eof" if request is None else "client_close")
                return
            kind = request.get("type")
            if kind == "ping":
                frame({"type": "pong", "id": request.get("id")})
            elif kind == "archive":
                archive_download(root, request)
            elif kind == "restore_chunk":
                restore_chunk(restore, request)
            elif kind in ("restore_begin", "restore_commit"):
                try:
                    if kind == "restore_begin":
                        if restore:
                            restore["staging"].cleanup()
                        restore = restore_prepare(root, config, request)
                        value = {"uploadId": restore["id"], "needs": restore["needs"]}
                    else:
                        value = restore_commit(root, restore, request)
                        restore["staging"].cleanup()
                        restore = None
                    frame({"type": "archive_result", "id": request["id"], "value": value})
                except Exception as error:
                    frame({"type": "archive_error", "id": request["id"], "message": str(error)})
            else:
                raise ValueError("Unsupported archive operation")
    finally:
        if restore:
            restore["staging"].cleanup()


def read_native_history(config):
    root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / str(uuid.UUID(config["sessionKey"]))
    native_id = config.get("nativeId")
    if not native_id:
        binding = json.loads((root / "native-session.json").read_text())
        if binding.get("version") != 1:
            raise ValueError("Unsupported native Claude identity binding")
        native_id = binding["nativeId"]
    native_id = str(uuid.UUID(native_id))
    paths = history_transcripts(root, native_id)
    if len(paths) != 1:
        raise ValueError("The bound remote Claude conversation is missing or ambiguous; no new conversation was created")
    path = paths[0]
    rows, incomplete = [], False
    before = path.stat()
    with path.open("rb") as file:
        while True:
            line = file.readline(MAX_FRAME + 1)
            if not line:
                break
            if len(line) > MAX_FRAME:
                raise ValueError("A native transcript row exceeds the recovery size limit")
            if not line.endswith(b"\n"):
                incomplete = True
                break
            value = json.loads(line)
            if value.get("uuid") and not value.get("isSidechain"):
                # API request blobs, environment dumps and credentials are not
                # transferred. Only message ancestry and actual messages matter.
                row = {k: value[k] for k in ("uuid", "parentUuid", "logicalParentUuid", "subtype", "type", "isSidechain", "isMeta", "isCompactSummary", "isApiErrorMessage", "isVisibleInTranscriptOnly", "isVirtual", "origin", "timestamp", "message") if k in value}
                if is_queued_input(value):
                    row["attachment"] = value["attachment"]
                    row["rendered"] = value.get("rendered", [])
                rows.append(row)
    after = path.stat()
    if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
        raise RuntimeError("The remote conversation changed during synchronization; retry before continuing")
    hidden_user_uuids = []
    messages = canonical_history(rows, hidden_user_uuids)
    pending_tools, results = set(), set()
    for message in messages:
        if message["role"] == "assistant":
            pending_tools.update(b["id"] for b in message["content"] if b.get("type") == "tool_use")
        elif message["role"] == "tool":
            results.add(message["toolCallId"])
    dispatches = sorted((root / "tool-dispatch" / native_id).glob("*.json"))
    if len(dispatches) > 10000:
        raise ValueError("Too many tool dispatch receipts to synchronize safely")
    for dispatch in dispatches:
        if dispatch.stat().st_size > 65536:
            raise ValueError("Invalid tool dispatch receipt")
        record = json.loads(dispatch.read_text())
        if record.get("version") not in (1, 2) or not isinstance(record.get("callId"), str):
            raise ValueError("Invalid tool dispatch receipt")
        if record.get("settled") is not True:
            pending_tools.add(record["callId"])
    pending_tools.difference_update(results)
    profile, _ = native_storage(root)
    task_files = sorted((profile / "tasks" / native_id).glob("*.json"))
    if len(task_files) > 1000:
        raise ValueError("Too many native tasks to synchronize safely")
    todos, tasks = [], []
    for task_file in task_files:
        if task_file.stat().st_size > 1024 * 1024:
            raise ValueError("Native task exceeds the recovery size limit")
        task = json.loads(task_file.read_text())
        if task.get("status") in ("pending", "in_progress", "completed"):
            tasks.append(task)
            todos.append({"content": task.get("subject", task.get("description", "")), "status": task["status"], "priority": "medium"})
    todo_kind = "tasks"
    if not tasks:
        confirmed = {m["toolCallId"] for m in messages if m["role"] == "tool" and not m.get("isError")}
        for message in messages:
            if message["role"] != "assistant":
                continue
            for block in message["content"]:
                if block.get("type") == "tool_use" and block.get("name") in ("mcp__locus__todowrite", "TodoWrite") and block.get("id") in confirmed:
                    items = block.get("input", {}).get("todos")
                    if not isinstance(items, list) or any(not isinstance(item, dict) or not all(isinstance(item.get(key), str) for key in ("content", "status")) for item in items):
                        raise ValueError("Invalid confirmed Claude todo list")
                    todos = [{"content": item["content"], "status": item["status"], "priority": item.get("priority") if isinstance(item.get("priority"), str) else "medium"} for item in items]
                    todo_kind = "todos"
    final = path.stat()
    if (before.st_size, before.st_mtime_ns, before.st_ino) != (final.st_size, final.st_mtime_ns, final.st_ino):
        raise RuntimeError("The remote conversation changed during synchronization; retry before continuing")
    snapshot = {"version": 1, "nativeId": native_id, "messages": messages, "todos": todos,
                "taskSnapshot": {"version": 1, "kind": todo_kind, "tasks": tasks, "items": todos},
                "incompleteTail": incomplete, "pendingToolIds": sorted(pending_tools),
                "hiddenUserUuids": hidden_user_uuids}
    encoded = json.dumps(snapshot, ensure_ascii=True, separators=(",", ":")).encode()
    if len(encoded) > MAX_FRAME - 65536:
        raise ValueError("The remote history exceeds the recovery transfer limit; no partial history was applied")
    snapshot["digest"] = hashlib.sha256(encoded).hexdigest()
    return snapshot


def prepare_fork(config, shared_profile=None):
    base = Path.home() / ".local/share/locus/claude-ssh/v1/sessions"
    source = base / str(uuid.UUID(config["sourceKey"]))
    target = base / str(uuid.UUID(config["sessionKey"]))
    if source == target:
        raise ValueError("Cannot fork a Claude profile onto itself")
    source_id, new_id, at = (str(uuid.UUID(config[key])) for key in ("sourceId", "newId", "at"))
    paths = history_transcripts(source, source_id)
    if len(paths) != 1:
        raise ValueError("Claude native transcript is unavailable or ambiguous")
    snapshot = None
    for _ in range(21):
        snapshot = transcript_snapshot(paths[0], at, config.get("afterCompact", False))
        if snapshot is not None:
            break
        time.sleep(0.1)
    if snapshot is None:
        raise ValueError("Claude has not flushed the requested fork boundary")
    _, project = native_storage(target, shared_profile)
    private_write(project / (source_id + ".jsonl"), snapshot)
    private_write(target / "fork.json", json.dumps({"version": 1, "source_id": source_id, "new_id": new_id}))
    restore_tasks(target, new_id, config.get("todos"))


def transcript_snapshot(path, at, after_compact):
    snapshot, found, anchored = [], False, False
    with path.open(encoding="utf-8") as file:
        for line in file:
            try:
                value = json.loads(line)
            except ValueError:
                if found:
                    break
                raise ValueError("Claude transcript contains an incomplete row before the requested boundary")
            # After a compaction summary Claude records the /compact command echo
            # and then the post-compaction attachments, including the Locus
            # carry-forward (SessionStart context). They belong to the summary.
            if found and (value.get("attachment", {}).get("type") == "queued_command" or (value.get("uuid") and value.get("type") != "attachment" and not (value.get("type") == "user" and value.get("isMeta") is True) and not (after_compact and hidden_native_user(value)))):
                break
            snapshot.append(line.rstrip("\n") + "\n")
            if value.get("uuid") == at:
                anchored = True
            if anchored and (not after_compact or value.get("isCompactSummary") is True):
                found = True
    return "".join(snapshot) if found else None


def restore_tasks(root, new_id, snapshot):
    if snapshot is None:
        return
    if snapshot.get("version") != 1:
        raise ValueError("Unsupported Claude todo snapshot")
    if snapshot.get("kind") != "tasks":
        return
    profile, _ = native_storage(root)
    target = profile / "tasks" / new_id
    target.mkdir(parents=True, exist_ok=True, mode=0o700)
    for task in snapshot["tasks"]:
        task_id = str(task["id"])
        if not re.fullmatch(r"[a-zA-Z0-9_-]{1,128}", task_id):
            raise ValueError("Invalid native task ID")
        with (target / (task_id + ".json")).open("x", encoding="utf-8") as file:
            json.dump(task, file)
            file.flush()
            os.fsync(file.fileno())


def prepare_rewind(config):
    root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / str(uuid.UUID(config["sessionKey"]))
    source_id, new_id, at = (str(uuid.UUID(config[key])) for key in ("sourceId", "newId", "at"))
    paths = native_transcripts(root, source_id)
    if not paths:
        state = json.loads((root / "fork.json").read_text())
        if state.get("version") != 1 or state["new_id"] != source_id:
            raise ValueError("Claude source transcript is unavailable")
        paths = native_transcripts(root, str(uuid.UUID(state["source_id"])))
    if len(paths) != 1:
        raise ValueError("Claude native transcript is unavailable or ambiguous")
    snapshot = transcript_snapshot(paths[0], at, config.get("afterCompact", False))
    if snapshot is None:
        raise ValueError("Claude has not flushed the requested rewind boundary")
    rows, pending_calls = [], set()
    for line in snapshot.splitlines():
        value = json.loads(line)
        content = value.get("message", {}).get("content", [])
        content = content if isinstance(content, list) else []
        if value.get("isCompactSummary") or (value.get("type") == "user" and not value.get("isMeta") and not any(block.get("type") == "tool_result" for block in content)):
            pending_calls.clear()
        for block in content:
            if block.get("type") == "tool_use":
                pending_calls.add(block["id"])
            elif block.get("type") == "tool_result":
                pending_calls.discard(block["tool_use_id"])
        value["sessionId"] = new_id
        if "session_id" in value:
            value["session_id"] = new_id
        rows.append(json.dumps(value, ensure_ascii=True) + "\n")
    if pending_calls:
        raise ValueError("Choose a rewind boundary after all tool results; pending native tools cannot be replayed")
    with paths[0].with_name(new_id + ".jsonl").open("x", encoding="utf-8") as file:
        file.writelines(rows)
        file.flush()
        os.fsync(file.fileno())
    restore_tasks(root, new_id, config.get("todos"))


def terminal_text(output):
    text = output.decode("utf-8", errors="ignore")
    text = re.sub(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", "", text)
    text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
    return re.sub(r"\s+", "", text)


def startup_trust_input(output, cwd, authorized):
    """Confirm only the CLI's first-run trust prompt for our owned empty CWD.
    Existing shared login/preferences are never edited by the helper. The CLI
    records the normal project trust choice itself, within this SSH session.
    """
    if not authorized:
        return None
    base = Path.home() / ".local/share/locus/claude-ssh/v1/sessions"
    try:
        relative = cwd.relative_to(base)
        if len(relative.parts) != 2 or relative.parts[1] != "terminal":
            return None
        if str(uuid.UUID(relative.parts[0])) != relative.parts[0] or cwd.resolve() != cwd.absolute():
            return None
    except ValueError:
        return None
    text = terminal_text(output)
    shown = re.search(r"Accessingworkspace:(.*?)Quicksafetycheck:", text)
    if not shown or shown.group(1) != re.sub(r"\s+", "", str(cwd)):
        return None
    if "❯No,exitYes,Itrustthisfolder" in text:
        return b"\x1b[B"
    if "No,exit❯Yes,Itrustthisfolder" in text:
        return b"\r"
    return None


class StartupTrustPrompt:
    """Let each native render settle before changing/confirming its selection."""
    def __init__(self, cwd, authorized):
        self.cwd, self.authorized = cwd, authorized
        self.output, self.selecting, self.done = b"", False, False
        self.pending, self.due = None, None

    def advance(self, data=b"", now=None):
        if self.done or not self.authorized:
            return None
        now = time.monotonic() if now is None else now
        if data:
            self.output = (self.output + data)[-65536:]
            if self.selecting:
                text = terminal_text(self.output)
                yes, no = text.rfind("❯Yes,Itrustthisfolder"), text.rfind("❯No,exit")
                approval = b"\r" if yes > no else None
            else:
                approval = startup_trust_input(self.output, self.cwd, self.authorized)
            if approval != self.pending:
                self.pending, self.due = approval, now + 0.5 if approval else None
        if self.pending and now >= self.due:
            key = self.pending
            self.pending, self.due, self.output = None, None, b""
            self.selecting, self.done = key != b"\r", key == b"\r"
            return key
        return None


def acquire_session_lease(root, timeout=35):
    """Fence recovery against a disconnected host that is still flushing rows.

    The watchdog stops an orphan within 25 seconds. Wait for its actual process
    lease instead of guessing from a local SSH exit or starting two native TUIs.
    Never kill a process owned by a different connection.
    """
    import fcntl
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    lease = (root / "session.lock").open("a+b")
    deadline = time.monotonic() + timeout
    try:
        while True:
            try:
                fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return lease
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError("Claude transport disconnected: Remote Claude session is still shutting down; no input was submitted")
                time.sleep(0.1)
    except BaseException:
        lease.close()
        raise


def run(config):
    global child_pid, options, last_contact
    if config.get("protocol") != PROTOCOL:
        raise ValueError("Unsupported Locus SSH protocol")
    mode = config.get("mode", "session")
    if mode not in ("inspect", "prepare", "session", "fork", "rewind", "history", "archive", "archive_read"):
        raise ValueError("Invalid SSH operation")
    frame({"type": "progress", "protocol": PROTOCOL, "source": "helper",
           "stage": "history" if mode in ("history", "archive", "archive_read") else "environment"})
    if mode == "archive_read":
        return archive_read_listener()
    if mode == "archive":
        return archive_listener(config)
    if mode == "history":
        # Reading a saved conversation needs only SSH file permissions. Do not
        # start Claude, inspect its login, unlock a keychain or install anything.
        if config.get("requireIdle"):
            key = str(uuid.UUID(config["sessionKey"]))
            root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / key
            frame({"type": "progress", "protocol": PROTOCOL, "stage": "history_lock"})
            with acquire_session_lease(root):
                frame({"type": "progress", "protocol": PROTOCOL, "stage": "history_read"})
                history = read_native_history(config)
        else:
            frame({"type": "progress", "protocol": PROTOCOL, "stage": "history_read"})
            history = read_native_history(config)
        frame({"type": "progress", "protocol": PROTOCOL, "stage": "history_serialize"})
        info = {"pythonInstalled": True, "cliSupported": False, "authenticated": False,
                "platform": sys.platform, "home": str(Path.home()), "history": history}
        frame({"type": "hello", "protocol": PROTOCOL, "info": info}, announce_history_size=True)
        return
    unlock_for_connection(config)
    cli, source, env, info = prepare(config, mode)
    if mode == "fork":
        prepare_fork(config, source if sys.platform == "darwin" else None)
    if mode == "rewind":
        prepare_rewind(config)
    if mode != "session":
        frame({"type": "hello", "protocol": PROTOCOL, "info": info})
        return
    if not info["authenticated"]:
        if info.get("keychainUnlocked") is False:
            raise RuntimeError("Remote macOS keychain is locked; test and save its unlock password in Claude SSH settings")
        raise RuntimeError("Remote Claude is not logged in. Configure Claude authentication on that server first.")
    key = str(uuid.UUID(config["sessionKey"]))
    root = Path.home() / ".local/share/locus/claude-ssh/v1/sessions" / key
    frame({"type": "progress", "protocol": PROTOCOL, "stage": "tui"})
    with acquire_session_lease(root):
        return run_session(config, cli, source, env, info, root)


def run_session(config, cli, source, env, info, root):
    global child_pid, options, last_contact
    shared_profile = source if sys.platform == "darwin" else None
    profile, _ = native_storage(root, shared_profile)
    if shared_profile is not None and profile != shared_profile.expanduser().absolute():
        raise RuntimeError("Claude session uses an isolated profile; start a new conversation to use the shared macOS login")
    cwd, plugin = root / "terminal", root / "mod"
    for directory in (cwd, profile, plugin):
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    private_write(plugin / ".claude-plugin/plugin.json", json.dumps({"name": "locus", "version": "0.3.0", "description": "Locus SSH local tool bridge"}))
    private_write(plugin / "hooks/hooks.json", '{"modules":["./register.js"]}')
    private_write(plugin / "hooks/register.js", config["modSource"])
    validate_plugin(cli, plugin, env)
    gateway = bool(env.get("ANTHROPIC_AUTH_TOKEN") or env.get("ANTHROPIC_API_KEY"))
    credential = profile / ".credentials.json"
    if shared_profile is None and not gateway and (source / ".credentials.json").is_file():
        private_write(credential, (source / ".credentials.json").read_text())
    elif shared_profile is None and credential.exists():
        credential.unlink()
    try:
        preferences = json.loads((profile / ".claude.json").read_text())
    except FileNotFoundError:
        preferences = {}
    preferences.update({"hasCompletedOnboarding": True, "theme": "dark", "lastOnboardingVersion": "2.1.299"})
    preferences.setdefault("projects", {})[str(cwd)] = {"hasTrustDialogAccepted": True, "hasClaudeMdExternalIncludesApproved": False}
    if env.get("ANTHROPIC_API_KEY"):
        preferences["customApiKeyResponses"] = {"approved": [env["ANTHROPIC_API_KEY"][-20:]], "rejected": []}
    if shared_profile is None:
        private_write(profile / ".claude.json", json.dumps(preferences))
    server = Server(("127.0.0.1", 0), Bridge)
    server.session_root = root
    server.token = config["token"]
    server.authority = "127.0.0.1:" + str(server.server_port)
    options = config["options"]
    if shared_profile is None:
        env["CLAUDE_CONFIG_DIR"] = str(profile)
    # macOS preserves the exact existing config environment and Keychain service.
    env.pop("CLAUDE_CODE_PROJECT_DIR_NAME", None)
    env.update({"LOCUS_CLAUDE_BRIDGE_URL": "http://" + server.authority,
                "LOCUS_CLAUDE_BRIDGE_TOKEN": server.token, "LOCUS_CLAUDE_TUI": "1", "TERM": "xterm-256color",
                "LOCUS_CLAUDE_NATIVE_TOOL_SEARCH": "1" if config.get("nativeToolSearch") else "0",
                "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION": "false",
                "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"})
    # Preserve legacy sessions' inherited search behavior. Only the native
    # Locus route pins search on, before any tool description is cached.
    if config.get("nativeToolSearch"):
        env["ENABLE_TOOL_SEARCH"] = "true"
    env.pop("CLAUDECODE", None)
    env.pop("CLAUDE_CODE_ENTRYPOINT", None)
    env.pop("CLAUDE_CODE_TASK_LIST_ID", None)
    permission_mode = config.get("permissionMode", "auto")
    if permission_mode not in ("auto", "bypassPermissions"):
        raise ValueError("Invalid Claude permission mode")
    # Keep in sync with tui.rs: native-agent conversations add only the
    # delegation tools; other launches keep the historical arguments.
    native_agents = config.get("nativeAgents") is True
    allowed = ["TaskCreate", "TaskGet", "TaskList", "TaskUpdate", "WebSearch", "WebFetch", "ToolSearch"]
    if native_agents:
        allowed += ["Agent", "SendMessage"]
        for key in ("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "CLAUDE_AUTO_BACKGROUND_TASKS"):
            env.pop(key, None)
        # Same as claude_launch::NATIVE_AGENT_ENV: fork subagents would force
        # every Agent into the background.
        env["CLAUDE_CODE_FORK_SUBAGENT"] = "0"
    args = [cli, "--plugin-dir", str(plugin), "--setting-sources", "", "--strict-mcp-config",
            "--allowedTools", *allowed,
            "--disallowedTools", "mcp__locus__web_fetch",
            "--permission-mode", permission_mode, "--system-prompt-snapshot", "off"]
    # Every loading mode pins the adapted builtins; without the list Claude also
    # offers NotebookEdit and other tools the Mod always denies.
    native_tools = config.get("nativeTools")
    if not isinstance(native_tools, str) or not native_tools or "ToolSearch" not in native_tools.split(",") or "TaskList" not in native_tools.split(","):
        raise ValueError("Claude launch requires the pinned builtin tool list")
    if ("Agent" in native_tools.split(",")) != native_agents:
        raise ValueError("Claude native agent tools do not match the launch request")
    args += ["--tools", native_tools]
    args += ["--settings", json.dumps(runtime_cli_settings(config, permission_mode))]
    if config.get("model"):
        args += ["--model", config["model"]]
    if config.get("resume"):
        resume = str(uuid.UUID(config["resume"]))
        fork_path = root / "fork.json"
        fork = json.loads(fork_path.read_text()) if fork_path.is_file() else None
        if fork and fork.get("version") != 1:
            raise ValueError("Unsupported Claude fork state")
        if fork and fork["new_id"] == resume and not native_transcripts(root, resume):
            args += ["--resume", str(uuid.UUID(fork["source_id"])), "--fork-session", "--session-id", resume]
        else:
            args += ["--resume", resume]
    host_pid = os.getpid()
    child_pid, master = pty.fork()
    if child_pid == 0:
        try:
            for name in ("SIGINT", "SIGTERM", "SIGHUP"):
                if hasattr(signal, name):
                    signal.signal(getattr(signal, name), signal.SIG_DFL)
            # Linux also stops Claude if this helper itself is killed abruptly.
            if sys.platform.startswith("linux"):
                import ctypes
                if ctypes.CDLL(None).prctl(1, signal.SIGKILL, 0, 0, 0) != 0 or os.getppid() != host_pid:
                    os._exit(1)
            server.server_close()
            os.chdir(cwd)
            os.execvpe(cli, args, env)
        finally:
            # Failed exec must not run the parent's cleanup or lifecycle journal.
            os._exit(127)
    try:
        import fcntl
        import struct
        import termios
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        last_contact = time.monotonic()
        # Start after fork: the child must not inherit locks held by queue threads.
        server.events = BridgeLanes()
        threading.Thread(target=watchdog, daemon=True).start()
        threading.Thread(target=server.serve_forever, daemon=True).start()
        threading.Thread(target=stdin_loop, args=(master, root), daemon=True).start()
        info["sessionRoot"] = str(root)
        record_lifecycle("ready")
        frame({"type": "hello", "protocol": PROTOCOL, "info": info, "pid": child_pid, "hostPid": host_pid, "root": str(root)})
        trust = StartupTrustPrompt(cwd, config.get("authorizedSetup") is True)
        while not closing.is_set():
            if time.monotonic() - last_contact > 25:
                request_shutdown("heartbeat_timeout", 1)
                raise TimeoutError("SSH heartbeat expired; remote Claude was stopped")
            readable, _, _ = select.select([master], [], [], 0.2)
            data = b""
            if readable:
                try:
                    data = os.read(master, 8192)
                except OSError:
                    request_shutdown("pty_closed")
                    break
                if not data:
                    request_shutdown("claude_exit")
                    break
                frame({"type": "terminal", "data": base64.b64encode(data).decode()})
            if not hasattr(server, "native_id"):
                approval = trust.advance(data)
                if approval:
                    os.write(master, approval)
        if lifecycle is not None:
            code, reason = shutdown_status or (0, "completed")
            lifecycle.update(exitCode=code, exitReason=reason)
        frame({"type": "exit"})
    except BaseException as error:
        code, reason = exception_status(error)
        request_shutdown(reason, code)
        raise
    finally:
        closing.set()
        cleanup(terminate_child)
        cleanup(server.server_close)
        cleanup(lambda: os.close(master))
        cleanup(reap_child)


def main():
    code, reason = 1, "bootstrap_error"
    try:
        os.umask(0o077)
        for name in ("SIGINT", "SIGTERM", "SIGHUP"):
            if hasattr(signal, name):
                signal.signal(getattr(signal, name), handle_signal)
        initialize_lifecycle()
        record_lifecycle("started")
        config = read_frame()
        if config is None:
            request_shutdown("stdin_eof")
        else:
            run(config)
        code, reason = shutdown_status or (0, "completed")
    except BaseException as error:
        code, reason = exception_status(error)
        request_shutdown(reason, code)
        code, reason = shutdown_status
        if lifecycle is not None:
            lifecycle.update(errorType=type(error).__name__[:80], exitCode=code, exitReason=reason)
        try:
            frame({"type": "error", "message": str(error) or reason})
        except BaseException as reporting_error:
            if lifecycle is not None:
                lifecycle["reportingError"] = type(reporting_error).__name__[:80]
    finally:
        exit_process(code, reason)


if __name__ == "__main__":
    main()
