import { spawn } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { finished } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const defaultProjectRoot = path.join(repoRoot, "testproject");
const matrixScript = path.join("scripts", "locus-unity-test.mjs");

const args = process.argv.slice(2);
const bun = process.execPath;
const explicitProjects = [];
const includeFilters = [];
const excludeFilters = [];
const driverArgs = [];
let projectRoot = defaultProjectRoot;
let prepareNative = false;
let prepareUnityBundle = false;
let failFast = false;
let dryRun = false;
let listOnly = false;
let jobs = 1;
let outputDir = "";
let matrixLogStream = null;
const matrixStartedAt = new Date();

for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];

  if (arg === "--") {
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    printHelp();
    process.exit(0);
  }

  const [name, inlineValue] = splitArg(arg);
  if (name === "--project-root") {
    projectRoot = resolveFromRepo(readOptionValue(name, inlineValue, args, index));
    if (!inlineValue) index += 1;
    continue;
  }
  if (name === "--project") {
    explicitProjects.push(resolveFromRepo(readOptionValue(name, inlineValue, args, index)));
    if (!inlineValue) index += 1;
    continue;
  }
  if (name === "--include") {
    includeFilters.push(readOptionValue(name, inlineValue, args, index));
    if (!inlineValue) index += 1;
    continue;
  }
  if (name === "--exclude") {
    excludeFilters.push(readOptionValue(name, inlineValue, args, index));
    if (!inlineValue) index += 1;
    continue;
  }
  if (name === "--jobs") {
    jobs = parsePositiveInteger(name, readOptionValue(name, inlineValue, args, index));
    if (!inlineValue) index += 1;
    continue;
  }
  if (name === "--output-dir") {
    outputDir = resolveFromRepo(readOptionValue(name, inlineValue, args, index));
    if (!inlineValue) index += 1;
    continue;
  }

  if (arg === "--prepare-native") {
    prepareNative = true;
    continue;
  }
  if (arg === "--prepare-unity-bundle") {
    prepareUnityBundle = true;
    continue;
  }
  if (arg === "--fail-fast") {
    failFast = true;
    continue;
  }
  if (arg === "--dry-run") {
    dryRun = true;
    continue;
  }
  if (arg === "--list") {
    listOnly = true;
    continue;
  }

  driverArgs.push(arg);
}

if (jobs !== 1) {
  console.error(
    "[locus] Parallel Unity matrix jobs are currently disabled. tauri dev uses Vite strictPort 14901, so concurrent driver processes contend for the same dev server port. Use --jobs 1.",
  );
  process.exit(2);
}

if (!hasDriverOption(driverArgs, "--suite")) {
  driverArgs.push("--suite", "connect,native-bridge");
}

const projects = applyFilters(discoverProjects(), includeFilters, excludeFilters);
initializeOutputDir(projects);

if (projects.length === 0) {
  logError(`[locus] No Unity projects found under ${projectRoot}.`);
  await closeMatrixLog();
  process.exit(1);
}

printProjectList(projects);

if (listOnly || dryRun) {
  if (dryRun) {
    for (const [index, project] of projects.entries()) {
      log(`[locus] dry-run: ${formatCommand(project, index)}`);
    }
  }
  writeRunSummary(projects, []);
  await closeMatrixLog();
  process.exit(0);
}

if (prepareUnityBundle) {
  await runRequired(bun, ["run", "unity:bundle"], "prepare-unity-bundle.log");
} else if (prepareNative) {
  await runRequired(bun, ["run", "unity:bundle-native"], "prepare-native.log");
}

const results = [];
for (const [index, project] of projects.entries()) {
  log(`[locus] Running Unity integration tests for ${project.name} (${project.version})`);
  const result = await runProject(project, index);
  results.push({ ...project, ...result });
  writeRunSummary(projects, results);

  if (result.signal) {
    await closeMatrixLog();
    process.kill(process.pid, result.signal);
  }
  if (result.code !== 0 && failFast) {
    break;
  }
}

printSummary(results);
writeRunSummary(projects, results);
await closeMatrixLog();
process.exit(results.every((result) => result.code === 0) ? 0 : 1);

function printHelp() {
  console.log(`Usage:
  bun run locus:test:unity:matrix -- [matrix options] [driver options]

Examples:
  bun run locus:test:unity:matrix
  bun run locus:test:unity:matrix -- --suite connect,state-probe
  bun run locus:test:unity:matrix:smoke
  bun run scripts/locus-unity-test-matrix.mjs --exclude 2021 --suite connect,native-bridge
  bun run scripts/locus-unity-test-matrix.mjs --project F:\\Game --project F:\\Other --suite connect

Matrix options:
  --project-root <dir>       Directory containing Unity projects, default testproject
  --project <dir>            Run a specific project; repeat to run multiple projects
  --include <text>           Keep projects whose name, path, or version contains text
  --exclude <text>           Skip projects whose name, path, or version contains text
  --prepare-native           Build locus_native.dll once before all project runs
  --prepare-unity-bundle     Rebuild the full locus_unity bundle once before all project runs
  --fail-fast                Stop after the first failed project
  --dry-run                  Print the commands without running Unity
  --list                     List discovered projects and exit
  --jobs <n>                 Currently only 1 is supported
  --output-dir <dir>         Write matrix.log, summary.json, project.log, and driver.log files

Driver options:
  Any option accepted by scripts/locus-unity-test.mjs, for example:
  --suite <name> --install-plugin --connect-timeout-ms <ms> --timeout-ms <ms>
  The matrix default suite is connect,native-bridge when --suite is omitted.

Parallel note:
  The current desktop dev path uses Vite strictPort 14901 through tauri dev.
  Running several matrix jobs at once needs per-worker devUrl/port isolation or a built-app runner.
`);
}

function splitArg(arg) {
  const [name, value = ""] = arg.split(/=(.*)/s, 2);
  return [name, value];
}

function readOptionValue(name, inlineValue, values, index) {
  if (inlineValue) {
    return inlineValue;
  }
  const next = values[index + 1];
  if (!next) {
    console.error(`[locus] ${name} requires a value.`);
    process.exit(2);
  }
  return next;
}

function parsePositiveInteger(name, value) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed < 1) {
    console.error(`[locus] ${name} requires a positive integer.`);
    process.exit(2);
  }
  return parsed;
}

function resolveFromRepo(value) {
  return path.resolve(repoRoot, value);
}

function hasDriverOption(values, optionName) {
  return values.some((value) => value === optionName || value.startsWith(`${optionName}=`));
}

function discoverProjects() {
  const paths = explicitProjects.length > 0 ? explicitProjects : discoverProjectDirectories(projectRoot);
  return paths.map(readProject).sort(compareProjects);
}

function discoverProjectDirectories(root) {
  if (!existsSync(root)) {
    return [];
  }
  return readdirSync(root)
    .map((name) => path.join(root, name))
    .filter((candidate) => {
      try {
        return statSync(candidate).isDirectory() && existsSync(projectVersionPath(candidate));
      } catch {
        return false;
      }
    });
}

function readProject(projectPath) {
  const versionPath = projectVersionPath(projectPath);
  if (!existsSync(versionPath)) {
    console.error(`[locus] Missing ProjectSettings/ProjectVersion.txt: ${projectPath}`);
    process.exit(1);
  }

  const versionText = readFileSync(versionPath, "utf8");
  const version = /m_EditorVersion:\s*(.+)/.exec(versionText)?.[1]?.trim() ?? "unknown";
  return {
    name: path.basename(projectPath),
    path: projectPath,
    version,
  };
}

function projectVersionPath(projectPath) {
  return path.join(projectPath, "ProjectSettings", "ProjectVersion.txt");
}

function compareProjects(a, b) {
  return a.version.localeCompare(b.version, undefined, { numeric: true }) || a.name.localeCompare(b.name);
}

function applyFilters(projects, includes, excludes) {
  return projects.filter((project) => {
    if (includes.length > 0 && !matchesAny(project, includes)) {
      return false;
    }
    return !matchesAny(project, excludes);
  });
}

function matchesAny(project, filters) {
  const haystack = `${project.name}\n${project.path}\n${project.version}`.toLowerCase();
  return filters.some((filter) => haystack.includes(filter.toLowerCase()));
}

function printProjectList(projects) {
  log(`[locus] Unity matrix projects (${projects.length}):`);
  for (const project of projects) {
    log(`[locus] - ${project.name} ${project.version} ${project.path}`);
  }
}

function formatCommand(project, index) {
  const outputArgs = outputDir ? ["--output-dir", projectOutputDir(project, index)] : [];
  return [bun, "run", matrixScript, "--project", project.path, ...outputArgs, ...driverArgs]
    .map((part) => (/\s/.test(part) ? JSON.stringify(part) : part))
    .join(" ");
}

function runRequired(command, commandArgs, logFileName) {
  return new Promise((resolve, reject) => {
    const logStream = outputDir
      ? createWriteStream(path.join(outputDir, logFileName), { flags: "w" })
      : null;
    const child = spawn(command, commandArgs, {
      stdio: logStream ? ["inherit", "pipe", "pipe"] : "inherit",
      shell: false,
    });
    if (logStream) {
      child.stdout.on("data", (chunk) => {
        process.stdout.write(chunk);
        safeWriteStream(logStream, chunk);
        writeMatrixLog(chunk);
      });
      child.stderr.on("data", (chunk) => {
        process.stderr.write(chunk);
        safeWriteStream(logStream, chunk);
        writeMatrixLog(chunk);
      });
    }
    child.on("error", async (error) => {
      logError(`[locus] Failed to start required command ${command} ${commandArgs.join(" ")}: ${error.message}`);
      if (logStream) {
        await endWritableStream(logStream);
      }
      await closeMatrixLog();
      process.exit(1);
    });
    child.on("exit", async (code, signal) => {
      if (logStream) {
        await endWritableStream(logStream);
      }
      if (signal) {
        await closeMatrixLog();
        process.kill(process.pid, signal);
        return;
      }
      if (code && code !== 0) {
        await closeMatrixLog();
        process.exit(code);
      }
      resolve();
    });
  });
}

function runProject(project, index) {
  return new Promise((resolve, reject) => {
    const startedAt = Date.now();
    const projectDir = outputDir ? projectOutputDir(project, index) : "";
    const projectLogPath = projectDir ? path.join(projectDir, "project.log") : "";
    const driverLogPath = projectDir ? path.join(projectDir, "driver.log") : "";
    if (projectDir) {
      mkdirSync(projectDir, { recursive: true });
      writeFileSync(
        path.join(projectDir, "project.json"),
        `${JSON.stringify({ ...project, outputDir: projectDir, startedAt: new Date().toISOString() }, null, 2)}\n`,
      );
    }
    const projectLogStream = projectDir
      ? createWriteStream(projectLogPath, { flags: "w" })
      : null;
    const outputArgs = projectDir ? ["--output-dir", projectDir] : [];
    const child = spawn(bun, ["run", matrixScript, "--project", project.path, ...outputArgs, ...driverArgs], {
      cwd: repoRoot,
      stdio: projectLogStream ? ["inherit", "pipe", "pipe"] : "inherit",
      shell: false,
    });
    if (projectLogStream) {
      child.stdout.on("data", (chunk) => {
        process.stdout.write(chunk);
        safeWriteStream(projectLogStream, chunk);
        writeMatrixLog(chunk);
      });
      child.stderr.on("data", (chunk) => {
        process.stderr.write(chunk);
        safeWriteStream(projectLogStream, chunk);
        writeMatrixLog(chunk);
      });
    }
    child.on("error", async (error) => {
      if (projectLogStream) {
        safeWriteStream(projectLogStream, `[locus] Failed to start project command: ${error.message}\n`);
        await endWritableStream(projectLogStream);
      }
      resolve({
        code: 1,
        signal: null,
        durationMs: Date.now() - startedAt,
        outputDir: projectDir || undefined,
        projectLogPath: projectLogPath || undefined,
        driverLogPath: driverLogPath || undefined,
        error: error.message,
      });
    });
    child.on("exit", async (code, signal) => {
      if (projectLogStream) {
        await endWritableStream(projectLogStream);
      }
      resolve({
        code: code ?? (signal ? 1 : 0),
        signal,
        durationMs: Date.now() - startedAt,
        outputDir: projectDir || undefined,
        projectLogPath: projectLogPath || undefined,
        driverLogPath: driverLogPath || undefined,
      });
    });
  });
}

function printSummary(results) {
  log("[locus] Unity matrix summary:");
  for (const result of results) {
    const status = result.code === 0 ? "PASS" : "FAIL";
    log(
      `[locus] ${status} ${result.name} ${result.version} (${Math.round(result.durationMs / 1000)}s)`,
    );
  }

  const failed = results.filter((result) => result.code !== 0);
  if (failed.length > 0) {
    logError(`[locus] ${failed.length}/${results.length} Unity matrix project(s) failed.`);
  } else {
    log(`[locus] ${results.length}/${results.length} Unity matrix project(s) passed.`);
  }
}

function initializeOutputDir(projects) {
  if (!outputDir) {
    return;
  }
  mkdirSync(outputDir, { recursive: true });
  matrixLogStream = createWriteStream(path.join(outputDir, "matrix.log"), { flags: "w" });
  log(`[locus] Unity matrix output: ${outputDir}`);
  writeRunSummary(projects, []);
}

function projectOutputDir(project, index) {
  return path.join(outputDir, `${String(index + 1).padStart(2, "0")}-${sanitizeFileName(project.name)}`);
}

function sanitizeFileName(value) {
  return value.replace(/[<>:"/\\|?*\x00-\x1f]+/g, "_");
}

function writeRunSummary(projects, results) {
  if (!outputDir) {
    return;
  }
  writeFileSync(
    path.join(outputDir, "summary.json"),
    `${JSON.stringify(
      {
        startedAt: matrixStartedAt.toISOString(),
        updatedAt: new Date().toISOString(),
        finished: results.length === projects.length,
        ok: results.length === projects.length && results.every((result) => result.code === 0),
        projectRoot,
        outputDir,
        prepareNative,
        prepareUnityBundle,
        failFast,
        jobs,
        driverArgs,
        projects: projects.map((project, index) => ({
          ...project,
          outputDir: projectOutputDir(project, index),
        })),
        results,
      },
      null,
      2,
    )}\n`,
  );
}

function log(message) {
  console.log(message);
  writeMatrixLog(`${message}\n`);
}

function logError(message) {
  console.error(message);
  writeMatrixLog(`${message}\n`);
}

async function closeMatrixLog() {
  if (!matrixLogStream) {
    return;
  }
  const stream = matrixLogStream;
  matrixLogStream = null;
  await endWritableStream(stream);
}

function writeMatrixLog(chunk) {
  safeWriteStream(matrixLogStream, chunk);
}

function safeWriteStream(stream, chunk) {
  if (!stream || stream.destroyed || stream.writableEnded || stream.closed) {
    return false;
  }
  try {
    stream.write(chunk);
    return true;
  } catch (error) {
    if (error?.code !== "ERR_STREAM_WRITE_AFTER_END") {
      console.error(`[locus] log write failed: ${error?.message ?? error}`);
    }
    return false;
  }
}

async function endWritableStream(stream) {
  if (!stream || stream.destroyed || stream.writableEnded || stream.closed) {
    return;
  }
  stream.end();
  try {
    await finished(stream);
  } catch (error) {
    if (error?.code !== "ERR_STREAM_WRITE_AFTER_END") {
      console.error(`[locus] log close failed: ${error?.message ?? error}`);
    }
  }
}
