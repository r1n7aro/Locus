import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildUpdateJson } from "./release-notes.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const docsDir = path.resolve(__dirname, "..");
const outputDir = path.join(docsDir, "data");
const outputPath = path.join(outputDir, "update.json");

const updateJson = await buildUpdateJson(docsDir);
await mkdir(outputDir, { recursive: true });
await writeFile(outputPath, `${JSON.stringify(updateJson, null, 2)}\n`, "utf8");
