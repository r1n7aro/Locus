// Ensure the dev compile-server sidecar matches the source protocol. The dev
// path should be fast when the published DLL is already current, while still
// repairing stale src-tauri/gen/compile-server outputs after protocol bumps.
import { execFileSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, readdir, rm, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const csharpSource = path.join(repoRoot, "locus_compile_server", "CompileService.cs");
const rustManagerSource = path.join(repoRoot, "src-tauri", "src", "csharp_compile", "manager.rs");
const project = path.join(repoRoot, "locus_compile_server", "LocusCompileServer.csproj");
const outputDir = path.join(repoRoot, "src-tauri", "gen", "compile-server");
const outputDll = path.join(outputDir, "LocusCompileServer.dll");
const inspectTimeoutMs = 5000;

function parseRequiredInt(source, pattern, label) {
  const match = source.match(pattern);
  if (!match) {
    throw new Error(`unable to read ${label}`);
  }
  return Number(match[1]);
}

async function readExpectedVersions() {
  const [csharp, rust] = await Promise.all([
    readFile(csharpSource, "utf8"),
    readFile(rustManagerSource, "utf8"),
  ]);
  const csharpProtocol = parseRequiredInt(
    csharp,
    /public\s+const\s+int\s+ProtocolVersion\s*=\s*(\d+)\s*;/,
    "CompileService.ProtocolVersion",
  );
  const csharpWrapper = parseRequiredInt(
    csharp,
    /public\s+const\s+int\s+WrapperContractVersion\s*=\s*(\d+)\s*;/,
    "CompileService.WrapperContractVersion",
  );
  const rustProtocol = parseRequiredInt(
    rust,
    /const\s+EXPECTED_PROTOCOL_VERSION:\s*i64\s*=\s*(\d+)\s*;/,
    "EXPECTED_PROTOCOL_VERSION",
  );
  const rustWrapper = parseRequiredInt(
    rust,
    /const\s+EXPECTED_WRAPPER_CONTRACT_VERSION:\s*i64\s*=\s*(\d+)\s*;/,
    "EXPECTED_WRAPPER_CONTRACT_VERSION",
  );

  if (csharpProtocol !== rustProtocol || csharpWrapper !== rustWrapper) {
    throw new Error(
      `compile server source version mismatch: C# protocol ${csharpProtocol}/contract ${csharpWrapper}, Rust expects ${rustProtocol}/contract ${rustWrapper}`,
    );
  }

  return {
    protocolVersion: csharpProtocol,
    wrapperContractVersion: csharpWrapper,
  };
}

function encodeMessage(message) {
  const body = Buffer.from(JSON.stringify(message), "utf8");
  return Buffer.concat([
    Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "utf8"),
    body,
  ]);
}

function tryParseResponse(buffer) {
  const headerEnd = buffer.indexOf("\r\n\r\n");
  if (headerEnd < 0) return null;

  const header = buffer.slice(0, headerEnd).toString("utf8");
  const lengthMatch = header.match(/Content-Length:\s*(\d+)/i);
  if (!lengthMatch) {
    throw new Error("compile server response missing Content-Length");
  }

  const length = Number(lengthMatch[1]);
  const bodyStart = headerEnd + 4;
  const bodyEnd = bodyStart + length;
  if (buffer.length < bodyEnd) return null;

  return JSON.parse(buffer.slice(bodyStart, bodyEnd).toString("utf8"));
}

function inspectPublishedVersion(expectedProtocolVersion) {
  return new Promise((resolve) => {
    if (!existsSync(outputDll)) {
      resolve({ ok: false, reason: "missing" });
      return;
    }

    const child = spawn("dotnet", [outputDll], {
      cwd: outputDir,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = Buffer.alloc(0);
    let stderr = "";
    let settled = false;

    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.stdin.destroy();
      child.kill();
      resolve(result);
    };

    const timer = setTimeout(() => {
      finish({
        ok: false,
        reason: `inspect timed out after ${inspectTimeoutMs}ms`,
        stderr,
      });
    }, inspectTimeoutMs);

    child.on("error", (error) => {
      finish({ ok: false, reason: error.message, stderr });
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString("utf8");
    });
    child.stdout.on("data", (chunk) => {
      try {
        stdout = Buffer.concat([stdout, chunk]);
        const response = tryParseResponse(stdout);
        if (!response) return;
        if (response.error) {
          finish({
            ok: false,
            reason: response.error.message ?? "initialize returned an error",
            stderr,
          });
          return;
        }
        finish({ ok: true, result: response.result, stderr });
      } catch (error) {
        finish({
          ok: false,
          reason: error instanceof Error ? error.message : String(error),
          stderr,
        });
      }
    });
    child.on("exit", (code, signal) => {
      if (!settled) {
        finish({
          ok: false,
          reason: `compile server exited before initialize response (${signal ?? code})`,
          stderr,
        });
      }
    });

    child.stdin.write(
      encodeMessage({
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: { protocolVersion: expectedProtocolVersion },
      }),
    );
  });
}

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

async function publishCompileServer() {
  if (!existsSync(project)) {
    throw new Error(`missing compile server project: ${project}`);
  }

  await rm(outputDir, { recursive: true, force: true });
  await mkdir(outputDir, { recursive: true });

  run("dotnet", [
    "publish",
    project,
    "-c",
    "Release",
    "--nologo",
    "-v",
    "minimal",
    "-o",
    outputDir,
  ]);

  if (!existsSync(outputDll)) {
    throw new Error(`compile server publish did not produce ${outputDll}`);
  }
}

async function fileMtimeMs(filePath) {
  try {
    return (await stat(filePath)).mtimeMs;
  } catch {
    return 0;
  }
}

async function latestCompileServerSourceMtimeMs() {
  const root = path.join(repoRoot, "locus_compile_server");
  let latest = 0;

  async function walk(dir) {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name === "bin" || entry.name === "obj") {
        continue;
      }
      const entryPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        await walk(entryPath);
      } else if (entry.isFile() && (entry.name.endsWith(".cs") || entry.name.endsWith(".csproj"))) {
        latest = Math.max(latest, await fileMtimeMs(entryPath));
      }
    }
  }

  await walk(root);
  return latest;
}

function describeObserved(result) {
  if (!result.ok) return result.reason;
  const protocol = result.result?.protocolVersion ?? "?";
  const contract = result.result?.wrapperContractVersion ?? "?";
  return `protocol ${protocol}, wrapper contract ${contract}`;
}

function matchesExpected(result, expected) {
  return (
    result.ok &&
    result.result?.protocolVersion === expected.protocolVersion &&
    result.result?.wrapperContractVersion === expected.wrapperContractVersion
  );
}

const expected = await readExpectedVersions();
const current = await inspectPublishedVersion(expected.protocolVersion);
const latestSourceMtimeMs = await latestCompileServerSourceMtimeMs();
const outputMtimeMs = await fileMtimeMs(outputDll);
const outputCurrentForSource = outputMtimeMs >= latestSourceMtimeMs;

if (matchesExpected(current, expected) && outputCurrentForSource) {
  console.log(
    `[locus] compile server current (protocol ${expected.protocolVersion}, wrapper contract ${expected.wrapperContractVersion}); skipping publish.`,
  );
  process.exit(0);
}

const sourceReason = outputCurrentForSource ? "" : "; source changed after published DLL";
console.log(
  `[locus] compile server publish required: expected protocol ${expected.protocolVersion}, wrapper contract ${expected.wrapperContractVersion}; found ${describeObserved(current)}${sourceReason}.`,
);
await publishCompileServer();

const updated = await inspectPublishedVersion(expected.protocolVersion);
if (!matchesExpected(updated, expected)) {
  throw new Error(
    `compile server publish verification failed: expected protocol ${expected.protocolVersion}, wrapper contract ${expected.wrapperContractVersion}; found ${describeObserved(updated)}`,
  );
}

console.log(`[locus] compile server published to ${outputDir}`);
