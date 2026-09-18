import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2).filter((arg) => arg !== "--");
let editor = "";
let outputRoot = "E:\\LocusTemp";
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--unity-editor") editor = args[++i] ?? "";
  else if (args[i] === "--output-root") outputRoot = args[++i] ?? "";
  else if (args[i] === "--help") {
    console.log("bun run locus:test:property -- --unity-editor <Unity.exe> [--output-root <dir>]");
    process.exit(0);
  } else throw new Error(`Unknown property test option: ${args[i]}`);
}
if (!editor) throw new Error("--unity-editor is required. Tests always create an isolated project.");
if (process.platform !== "win32") throw new Error("This property regression launcher currently requires Windows.");
async function run(command, parameters) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, parameters, { stdio: "inherit", windowsHide: true });
    child.on("error", reject);
    child.on("exit", (code) => resolve(code ?? 1));
  });
}
const bundled = await run(process.execPath, ["run", "unity:bundle-json"]);
if (bundled) process.exit(bundled);
const script = fileURLToPath(new URL("./tests/unity-property/run.ps1", import.meta.url));
const manifest = fileURLToPath(new URL("./tests/unity-property/yaml-driver/Cargo.toml", import.meta.url));
const target = fileURLToPath(new URL("../src-tauri/target", import.meta.url));
const driver = fileURLToPath(new URL("../src-tauri/target/debug/locus-property-yaml-driver.exe", import.meta.url));
const built = await run("cargo", ["build", "--manifest-path", manifest, "--target-dir", target, "--quiet"]);
if (built) process.exit(built);
process.exit(await run("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, "-UnityEditor", editor, "-OutputRoot", outputRoot, "-YamlDriver", driver]));
