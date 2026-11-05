import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === "__tests__") return [];
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? sourceFiles(path) : /\.(ts|vue)$/.test(entry.name) ? [path] : [];
  });
}

// Literal command names passed to ipcInvoke(), invoke() or runtime.invoke().
// Commands forwarded through a variable are checked at their literal call site.
const INVOKE_CALL = /(?<![\w$])(?:ipcInvoke|invoke)\s*(?:<[^()]*?>)?\s*\(\s*(["'`])([\w:|-]+)\1/g;

function registeredCommands(libPath = "src-tauri/src/lib.rs"): Set<string> {
  const lib = readFileSync(resolve(libPath), "utf8");
  const marker = "tauri::generate_handler![";
  const start = lib.indexOf(marker);
  if (start < 0) throw new Error(`generate_handler! not found in ${libPath}`);
  const body = lib.slice(start + marker.length, lib.indexOf("])", start));
  return new Set(body.replace(/\/\/.*$/gm, "").split(",")
    .map((entry) => entry.trim().split("::").pop() ?? "")
    .filter(Boolean));
}

describe("IPC command registration", () => {
  it("registers every command the frontend invokes", () => {
    const registered = registeredCommands();
    // The mobile page also talks to the Locus Remote app it runs in
    // (connection, relay); those commands are registered by the app.
    const remoteApp = registeredCommands("mobile/src-tauri/src/lib.rs");
    const unregistered = new Set<string>();
    let invoked = 0;
    for (const file of sourceFiles(resolve("src"))) {
      const path = relative(process.cwd(), file).replaceAll("\\", "/");
      for (const match of readFileSync(file, "utf8").matchAll(INVOKE_CALL)) {
        const command = match[2];
        // Tauri plugin commands are registered by their plugins.
        if (command.startsWith("plugin:")) continue;
        invoked += 1;
        if (registered.has(command)) continue;
        if (path.startsWith("src/mobile/") && remoteApp.has(command)) continue;
        unregistered.add(`${path}: ${command}`);
      }
    }
    // Guard against a parser that silently matches nothing.
    expect(registered.size).toBeGreaterThan(300);
    expect(invoked).toBeGreaterThan(300);
    expect([...unregistered]).toEqual([]);
  });
});
