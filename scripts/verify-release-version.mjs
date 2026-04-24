import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildUpdateJson, parseAllReleaseNotes } from "../docs/scripts/release-notes.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

async function readJson(relativePath) {
  const filePath = path.join(repoRoot, relativePath);
  const content = await readFile(filePath, "utf8");
  return JSON.parse(content);
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

const docsDir = path.join(repoRoot, "docs");
const parsedReleaseNotes = await parseAllReleaseNotes(docsDir);
const generatedUpdateJson = await buildUpdateJson(docsDir);
const updateManifestPath = "docs/data/update.json";
const existingUpdateJson = await readJson(updateManifestPath);
const versions = {
  "docs/overview/latest-version.mdx": parsedReleaseNotes.zh.version,
  "docs/en/overview/latest-version.mdx": parsedReleaseNotes.en.version,
  [updateManifestPath]: existingUpdateJson.version,
  "package.json": (await readJson("package.json")).version,
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

if (stableStringify(existingUpdateJson) !== stableStringify(generatedUpdateJson)) {
  throw new Error(`${updateManifestPath} 与 latest-version.mdx 生成结果不一致，请先运行 bun run release:generate`);
}

console.log(`版本校验通过：${uniqueVersions[0]}`);
