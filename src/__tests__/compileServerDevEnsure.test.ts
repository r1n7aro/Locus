import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("compile server dev ensure", () => {
  it("checks the published sidecar protocol before dev startup rebuilds it", () => {
    const pkg = read("package.json");
    const tauriConfig = read("src-tauri/tauri.conf.json");
    const ensureScript = read("scripts/ensure-locus-compile-server.mjs");
    const buildScript = read("scripts/build-locus-compile-server.mjs");

    expect(pkg).toContain('"compile-server:bundle": "bun run scripts/build-locus-compile-server.mjs"');
    expect(pkg).toContain('"compile-server:ensure": "bun run scripts/ensure-locus-compile-server.mjs"');
    expect(tauriConfig).toContain(
      '"beforeDevCommand": "bun run compile-server:ensure && bun run ort:bundle && bun run github-cli:bundle && bun run dev"',
    );
    expect(ensureScript).toContain("CompileService.ProtocolVersion");
    expect(ensureScript).toContain("EXPECTED_PROTOCOL_VERSION");
    expect(ensureScript).toContain("inspectPublishedVersion");
    expect(ensureScript).toContain("skipping publish");
    expect(ensureScript).toContain("publish required");
    expect(buildScript).toContain("dotnet");
    expect(buildScript).toContain("publish");
  });
});
