import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildUpdateManifests,
  parseAllChannelReleaseNotes,
} from "../docs/scripts/release-notes.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

async function readJson(relativePath) {
  const filePath = path.join(repoRoot, relativePath);
  const content = await readFile(filePath, "utf8");
  return JSON.parse(content);
}

async function readJsonOptional(relativePath) {
  try {
    return await readJson(relativePath);
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function readCargoVersion(relativePath) {
  const filePath = path.join(repoRoot, relativePath);
  const content = await readFile(filePath, "utf8");
  const match = content.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) {
    throw new Error(`无法在 ${relativePath} 中找到 version 字段`);
  }
  return match[1];
}

function stableStringify(value) {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(",")}]`;
  }

  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(",")}}`;
  }

  return JSON.stringify(value);
}

function normalizeReleaseChannel(value) {
  return value === "experimental" ? "experimental" : "stable";
}

const docsDir = path.join(repoRoot, "docs");
const parsedReleaseNotesByChannel = await parseAllChannelReleaseNotes(docsDir);
const generatedManifests = await buildUpdateManifests(docsDir);
const packageJson = await readJson("package.json");
const releaseChannel = normalizeReleaseChannel(packageJson.releaseChannel);
if (packageJson.releaseChannel !== releaseChannel) {
  throw new Error("package.json 的 releaseChannel 必须为 stable 或 experimental");
}
const releaseNotesForChannel = parsedReleaseNotesByChannel[releaseChannel];

if (!releaseNotesForChannel) {
  throw new Error(`缺少 ${releaseChannel} 通道的 latest-version 元数据`);
}

const versions = {
  "package.json": packageJson.version,
  "src-tauri/tauri.conf.json": (await readJson("src-tauri/tauri.conf.json")).version,
  "src-tauri/Cargo.toml": await readCargoVersion("src-tauri/Cargo.toml"),
};

for (const [file, version] of Object.entries(versions)) {
  if (typeof version !== "string" || version.length === 0) {
    throw new Error(`${file} 的 version 不能为空`);
  }
}

const uniqueVersions = [...new Set(Object.values(versions))];

if (uniqueVersions.length !== 1) {
  const details = Object.entries(versions)
    .map(([file, version]) => `${file}: ${version}`)
    .join("\n");
  throw new Error(`版本号不一致：\n${details}`);
}

const appVersion = uniqueVersions[0];
const releaseMetadataVersions = {
  [`${releaseChannel}:docs/overview`]: releaseNotesForChannel.zh.version,
  [`${releaseChannel}:docs/en/overview`]: releaseNotesForChannel.en.version,
};

for (const [source, version] of Object.entries(releaseMetadataVersions)) {
  if (version !== appVersion) {
    throw new Error(`${source} 的 version ${version} 与当前应用版本 ${appVersion} 不一致`);
  }
}

const manifestOutputs = {
  stable: ["docs/data/update.json", "docs/data/update-stable.json"],
  experimental: ["docs/data/update-experimental.json"],
};

for (const [channel, paths] of Object.entries(manifestOutputs)) {
  const generatedManifest = generatedManifests[channel];

  for (const manifestPath of paths) {
    const existingManifest = await readJsonOptional(manifestPath);

    if (!generatedManifest) {
      if (existingManifest) {
        throw new Error(`${manifestPath} 已存在，但缺少 ${channel} 通道 release notes 生成源`);
      }
      continue;
    }

    if (!existingManifest) {
      throw new Error(`${manifestPath} 缺失，请先运行 bun run release:generate`);
    }

    if (stableStringify(existingManifest) !== stableStringify(generatedManifest)) {
      throw new Error(`${manifestPath} 与 latest-version.mdx 生成结果不一致，请先运行 bun run release:generate`);
    }
  }
}

const releaseManifestPath = releaseChannel === "experimental"
  ? "docs/data/update-experimental.json"
  : "docs/data/update.json";
const releaseManifest = await readJson(releaseManifestPath);
if (releaseManifest.version !== appVersion) {
  throw new Error(`${releaseManifestPath} 的 version ${releaseManifest.version} 与当前应用版本 ${appVersion} 不一致`);
}
if (normalizeReleaseChannel(releaseManifest.channel) !== releaseChannel) {
  throw new Error(`${releaseManifestPath} 的 channel ${releaseManifest.channel} 与 package.json releaseChannel ${releaseChannel} 不一致`);
}

console.log(`版本校验通过：${appVersion} (${releaseChannel})`);
