import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildUpdateManifests } from "./release-notes.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const docsDir = path.resolve(__dirname, "..");
const outputDir = path.join(docsDir, "data");
const outputFiles = {
  stable: ["update.json", "update-stable.json"],
  experimental: ["update-experimental.json"],
};

const manifests = await buildUpdateManifests(docsDir);
await mkdir(outputDir, { recursive: true });

for (const [channel, fileNames] of Object.entries(outputFiles)) {
  const manifest = manifests[channel];

  for (const fileName of fileNames) {
    const outputPath = path.join(outputDir, fileName);
    if (!manifest) {
      await rm(outputPath, { force: true });
      continue;
    }

    await writeFile(outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  }
}
