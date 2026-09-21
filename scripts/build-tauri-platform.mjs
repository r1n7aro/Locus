import { execFileSync } from "node:child_process";
import { resolveMacosTarget } from "./macos-build-target.mjs";

// Preserve the Windows full-installer preparation command verbatim.
const script = resolveMacosTarget() ? "build:tauri:macos" : "build:tauri:with_embed_python_git";
execFileSync(process.execPath, ["run", script], { stdio: "inherit" });
