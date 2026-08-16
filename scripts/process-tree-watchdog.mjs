import { spawnSync } from "node:child_process";

const targetPid = Number.parseInt(process.argv[2] ?? "", 10);

if (!Number.isSafeInteger(targetPid) || targetPid <= 0) {
  process.exit(2);
}

let finished = false;
let targetPoll = null;

function isProcessRunning(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

function terminateProcessTree(pid) {
  if (!isProcessRunning(pid)) {
    return;
  }

  if (process.platform === "win32") {
    spawnSync("taskkill", ["/pid", String(pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }

  try {
    process.kill(pid, "SIGTERM");
  } catch {
    // The target can exit between the liveness check and the signal.
  }
}

function finish({ terminate }) {
  if (finished) {
    return;
  }
  finished = true;
  if (targetPoll) {
    clearInterval(targetPoll);
  }
  if (terminate) {
    terminateProcessTree(targetPid);
  }
  process.exit(0);
}

// The launcher owns this pipe. Normal exit, terminal closure, and forced
// launcher termination all close it at the OS level, so the watchdog can
// still reap the managed process tree when JavaScript cleanup cannot run.
process.stdin.on("end", () => finish({ terminate: true }));
process.stdin.on("close", () => finish({ terminate: true }));
process.stdin.on("error", () => finish({ terminate: true }));
process.stdin.resume();

targetPoll = setInterval(() => {
  if (!isProcessRunning(targetPid)) {
    finish({ terminate: false });
  }
}, 500);
targetPoll.unref();
