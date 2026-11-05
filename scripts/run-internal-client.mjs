import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const command = args.shift() || "dev";
if (!["dev", "dev-mcp", "build"].includes(command)) throw new Error("Use dev, dev-mcp, or build");
if (args.some(arg => /--config|--features|--all-features/.test(arg))) {
  throw new Error("Internal build owns its feature and configuration arguments");
}
const result = spawnSync(process.execPath, [path.join(root, "scripts/run-tauri.mjs"), command,
  "--features", "server-client", "--config", "src-tauri/tauri.internal.conf.json", ...args], {
  cwd: root, stdio: "inherit", env: { ...process.env, LOCUS_BUILD_EDITION: "internal" },
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
