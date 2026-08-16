// Publish the Locus compile-server sidecar (framework-dependent .NET DLL)
// into src-tauri/gen/compile-server/, where dev builds resolve it from and
// `tauri.conf.json` bundles it from (resources -> compile-server/).
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

const project = path.join(repoRoot, "locus_compile_server", "LocusCompileServer.csproj");
const outputDir = path.join(repoRoot, "src-tauri", "gen", "compile-server");
const outputParentDir = path.dirname(outputDir);

function run(command, args, options = {}) {
  execFileSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    ...options,
  });
}

if (!existsSync(project)) {
  throw new Error(`missing compile server project: ${project}`);
}

async function relativeFiles(root, relativeDir = "") {
  const dir = path.join(root, relativeDir);
  const entries = await readdir(dir, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));

  const files = [];
  for (const entry of entries) {
    const relativePath = path.join(relativeDir, entry.name);
    if (entry.isDirectory()) {
      files.push(...await relativeFiles(root, relativePath));
    } else if (entry.isFile()) {
      files.push(relativePath);
    }
  }
  return files;
}

async function directoriesMatch(left, right) {
  if (!existsSync(right)) return false;

  const [leftFiles, rightFiles] = await Promise.all([
    relativeFiles(left),
    relativeFiles(right),
  ]);
  if (leftFiles.length !== rightFiles.length) return false;

  for (let index = 0; index < leftFiles.length; index += 1) {
    if (leftFiles[index] !== rightFiles[index]) return false;
    const [leftBytes, rightBytes] = await Promise.all([
      readFile(path.join(left, leftFiles[index])),
      readFile(path.join(right, rightFiles[index])),
    ]);
    if (!leftBytes.equals(rightBytes)) return false;
  }
  return true;
}

async function restoreMissingUnchangedFiles(stagingDir) {
  if (!existsSync(outputDir)) return [];

  const [stagingFiles, outputFiles] = await Promise.all([
    relativeFiles(stagingDir),
    relativeFiles(outputDir),
  ]);
  const stagingFileSet = new Set(stagingFiles);
  const outputFileSet = new Set(outputFiles);
  if (outputFiles.some((file) => !stagingFileSet.has(file))) return [];

  for (const file of outputFiles) {
    const [stagingBytes, outputBytes] = await Promise.all([
      readFile(path.join(stagingDir, file)),
      readFile(path.join(outputDir, file)),
    ]);
    if (!stagingBytes.equals(outputBytes)) return [];
  }

  const missingFiles = stagingFiles.filter((file) => !outputFileSet.has(file));
  for (const file of missingFiles) {
    const destination = path.join(outputDir, file);
    await mkdir(path.dirname(destination), { recursive: true });
    await copyFile(path.join(stagingDir, file), destination);
  }
  return missingFiles;
}

async function replacePublishedDirectory(stagingDir) {
  if (!existsSync(outputDir)) {
    await rename(stagingDir, outputDir);
    return;
  }

  const backupDir = `${outputDir}.previous-${process.pid}`;
  await rm(backupDir, { recursive: true, force: true });
  try {
    await rename(outputDir, backupDir);
  } catch (error) {
    throw new Error(
      `unable to replace compile server publish at ${outputDir}; a running process may still be using it: ${error.message}`,
      { cause: error },
    );
  }

  try {
    await rename(stagingDir, outputDir);
  } catch (error) {
    await rename(backupDir, outputDir).catch(() => {});
    throw error;
  }

  try {
    await rm(backupDir, { recursive: true, force: true });
  } catch (error) {
    console.warn(
      `[locus] previous compile server publish remains at ${backupDir}; it can be removed after the process using it exits: ${error.message}`,
    );
  }
}

await mkdir(outputParentDir, { recursive: true });
let stagingDir = await mkdtemp(path.join(outputParentDir, "compile-server.publish-"));
const stagingDll = path.join(stagingDir, "LocusCompileServer.dll");

try {
  run("dotnet", [
    "publish",
    project,
    "-c",
    "Release",
    "--nologo",
    "-v",
    "minimal",
    "-o",
    stagingDir,
  ]);

  if (!existsSync(stagingDll)) {
    throw new Error(`compile server publish did not produce ${stagingDll}`);
  }

  if (await directoriesMatch(stagingDir, outputDir)) {
    console.log(`[locus] compile server publish unchanged at ${outputDir}`);
  } else {
    const restoredFiles = await restoreMissingUnchangedFiles(stagingDir);
    if (restoredFiles.length > 0) {
      console.log(
        `[locus] compile server publish restored ${restoredFiles.length} missing file(s) at ${outputDir}`,
      );
    } else {
      await replacePublishedDirectory(stagingDir);
      stagingDir = null;
      console.log(`[locus] compile server published to ${outputDir}`);
    }
  }
} finally {
  if (stagingDir) {
    await rm(stagingDir, { recursive: true, force: true });
  }
}
