import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateReleaseNotesMetadata } from "./release-notes.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const docsDir = path.resolve(__dirname, "..");

await validateReleaseNotesMetadata(docsDir);
console.log("latest-version 元数据校验通过");
