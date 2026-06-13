import { spawn } from "node:child_process";
import { createWriteStream, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { finished } from "node:stream/promises";

const args = process.argv.slice(2);
const passthrough = [];
let prepareNative = false;
let prepareUnityBundle = false;

for (const arg of args) {
  if (arg === "--") {
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    printHelp();
    process.exit(0);
  }
  if (arg === "--prepare-native") {
    prepareNative = true;
    continue;
  }
  if (arg === "--prepare-unity-bundle") {
    prepareUnityBundle = true;
    continue;
  }
  passthrough.push(arg);
}

const bun = process.execPath;

if (prepareUnityBundle) {
  await runRequired(bun, ["run", "unity:bundle"]);
} else if (prepareNative) {
  await runRequired(bun, ["run", "unity:bundle-native"]);
}

const driverResult = await runUnityDriver(bun, [
  "run",
  "tauri",
  "dev",
  "--",
  "--",
  "--locus-driver",
  "unity-test",
  ...passthrough,
]);

if (driverResult.signal) {
  process.kill(process.pid, driverResult.signal);
} else if (driverResult.code && driverResult.code !== 0) {
  if (driverResult.finishedOk) {
    console.warn(
      `[locus] Tauri dev exited with ${driverResult.code} after the Unity driver reported success; treating the driver result as authoritative.`,
    );
  } else {
    printDriverFailure(driverResult);
    process.exit(driverResult.code);
  }
}

function printHelp() {
  console.log(`Usage:
  bun run locus:test:unity -- --project <UnityProject> [options]

Examples:
  bun run locus:test:unity -- --project F:\\Game --suite connect
  bun run locus:test:unity -- --project F:\\Game --suite state-probe --install-plugin
  bun run locus:test:unity -- --project F:\\Game --suite native-bridge --prepare-native --install-plugin
  bun run locus:test:unity:native -- --project F:\\Game
  bun run locus:test:unity:smoke -- --project F:\\Game
  bun run locus:test:unity -- --project F:\\Game --suite hot-reload --timeout-ms 1200000

Driver options:
  --suite <name>              connect | state-probe | native-bridge | hot-reload | all
  --connect-timeout-ms <ms>   Unity launch/connect timeout, default 60000
  --timeout-ms <ms>           Per-suite timeout, default 300000
  --poll-ms <ms>              Connection poll interval, default 500
  --no-progress-timeout-ms <ms>
                              Fail connection when status does not change, default 20000
  --install-plugin            Update the Unity project plugin before connecting
  --no-open-unity             Only connect to an already-open editor
  --no-force-edit-mode        Leave the current editor mode before hot-reload tests

Wrapper options:
  --prepare-native            Build locus_native.dll before starting Locus
  --prepare-unity-bundle      Rebuild the full locus_unity bundle before starting Locus
`);
}

function runRequired(command, commandArgs) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
      stdio: "inherit",
      shell: false,
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (signal) {
        process.kill(process.pid, signal);
        return;
      }
      if (code && code !== 0) {
        process.exit(code);
      }
      resolve();
    });
  });
}

function runUnityDriver(command, commandArgs) {
  return new Promise((resolve, reject) => {
    const logDir = mkdtempSync(join(tmpdir(), "locus-unity-test-"));
    const logPath = join(logDir, "driver.log");
    const logStream = createWriteStream(logPath, { flags: "w" });
    console.log(`[locus] Unity driver log: ${logPath}`);
    const state = {
      finishedOk: false,
      sawDriverEvent: false,
      terminalEventSeen: false,
      driverError: "",
      recentEvents: [],
      stdoutRemainder: "",
      stderrRemainder: "",
      logPath,
    };
    const child = spawn(command, commandArgs, {
      stdio: ["inherit", "pipe", "pipe"],
      shell: false,
    });

    child.stdout.on("data", (chunk) => {
      logStream.write(chunk);
      state.stdoutRemainder = consumeDriverLines(
        state.stdoutRemainder + chunk.toString(),
        state,
        child,
      );
    });
    child.stderr.on("data", (chunk) => {
      logStream.write(chunk);
      state.stderrRemainder = consumeDriverLines(
        state.stderrRemainder + chunk.toString(),
        state,
        child,
      );
    });
    child.on("error", reject);
    child.on("exit", async (code, signal) => {
      consumeDriverLines(`${state.stdoutRemainder}\n`, state, child);
      consumeDriverLines(`${state.stderrRemainder}\n`, state, child);
      logStream.end();
      try {
        await finished(logStream);
        replayDriverEventsFromLog(state, child);
      } catch {
        // Keep the child process result authoritative; log replay is diagnostic.
      }
      resolve({
        code,
        signal,
        finishedOk: state.finishedOk && state.sawDriverEvent,
        driverError: state.driverError,
        recentEvents: state.recentEvents,
        logPath: state.logPath,
      });
    });
  });
}

function replayDriverEventsFromLog(state, child) {
  state.stdoutRemainder = "";
  state.stderrRemainder = "";
  const text = readFileSync(state.logPath, "utf8");
  consumeDriverLines(`${text}\n`, state, child);
}

function consumeDriverLines(buffer, state, child) {
  const lines = buffer.split(/\r?\n/);
  const remainder = lines.pop() ?? "";
  for (const line of lines) {
    parseDriverEventLine(line, state, child);
  }
  return remainder;
}

function parseDriverEventLine(line, state, child) {
  const marker = "LOCUS_DRIVER_JSON ";
  const index = line.indexOf(marker);
  if (index < 0) {
    return;
  }

  const jsonText = extractJsonObject(line.slice(index + marker.length));
  if (!jsonText) {
    return;
  }

  try {
    const event = JSON.parse(jsonText);
    state.sawDriverEvent = true;
    rememberDriverEvent(state, event);
    if (event?.event === "error") {
      state.driverError = event?.payload?.message ?? JSON.stringify(event.payload ?? {});
      if (!state.terminalEventSeen) {
        state.terminalEventSeen = true;
        terminateChildTree(child);
      }
    }
    if (event?.event === "finished" && event?.payload?.ok === true) {
      state.finishedOk = true;
      if (!state.terminalEventSeen) {
        state.terminalEventSeen = true;
        terminateChildTree(child);
      }
    }
  } catch {
    // Keep stdout/stderr behavior identical; malformed log lines should not
    // mask the actual child process result.
  }
}

function terminateChildTree(child) {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null || child.killed) {
    return;
  }

  if (process.platform === "win32") {
    spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    return;
  }

  child.kill("SIGTERM");
}

function rememberDriverEvent(state, event) {
  state.recentEvents.push(event);
  if (state.recentEvents.length > 12) {
    state.recentEvents.shift();
  }
}

function printDriverFailure(result) {
  console.error("[locus] Unity integration test failed.");
  if (result.logPath) {
    console.error(`[locus] driver log: ${result.logPath}`);
  }
  if (result.driverError) {
    console.error(`[locus] driver error: ${result.driverError}`);
  }
  if (result.recentEvents?.length) {
    console.error("[locus] recent driver events:");
    for (const event of result.recentEvents) {
      console.error(`[locus] ${JSON.stringify(event)}`);
    }
  }
  if (result.logPath) {
    const tail = readTextFileTail(result.logPath, 160);
    if (tail) {
      console.error("[locus] driver log tail:");
      console.error(tail);
    }
  }
}

function readTextFileTail(filePath, maxLines) {
  try {
    const text = readFileSync(filePath, "utf8");
    const lines = text.trimEnd().split(/\r?\n/);
    return lines.slice(-maxLines).join("\n");
  } catch {
    return "";
  }
}

function extractJsonObject(text) {
  const start = text.indexOf("{");
  if (start < 0) {
    return "";
  }

  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = start; index < text.length; index += 1) {
    const ch = text[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === "\\") {
        escaped = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }

    if (ch === '"') {
      inString = true;
    } else if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        return text.slice(start, index + 1);
      }
    }
  }

  return "";
}
