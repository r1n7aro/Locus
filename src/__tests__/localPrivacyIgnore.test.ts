import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

function ignoredPaths(paths: string[]): string[] {
  const result = spawnSync("git", ["check-ignore", "--stdin"], {
    cwd: process.cwd(),
    encoding: "utf8",
    input: `${paths.join("\n")}\n`,
  });
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(result.stderr || "git check-ignore failed");
  }

  return result.stdout.trim().split(/\r?\n/).filter(Boolean);
}

describe("local privacy ignore rules", () => {
  it("ignores developer overrides, credentials, diagnostics, and runtime data", () => {
    const paths = [
      ".env",
      ".env.local",
      ".env.development.local",
      ".envrc",
      ".direnv/cache",
      ".dev.vars",
      "settings.local.json",
      "config.local.toml",
      ".locus-dev.local.json",
      ".mcp.json",
      ".npmrc",
      ".pypirc",
      ".netrc",
      ".docker/config.json",
      ".aws/credentials",
      "credentials.json",
      "secrets.json",
      "developer.key",
      "developer.pem",
      "developer.pfx",
      "session.har",
      "renderer.heapsnapshot",
      "crash.dmp",
      "locus.db",
      "locus.db-wal",
      ".cursor/chat.json",
      ".codex-logs/runtime.log",
      ".locus-hr-out/driver.log",
    ];

    expect(ignoredPaths(paths)).toEqual(paths);
  });

  it("keeps shareable example configuration visible to Git", () => {
    const examples = [
      ".env.example",
      ".env.development.example",
      ".dev.vars.example",
      "credentials.example.json",
      "secrets.example.json",
    ];

    expect(ignoredPaths(examples)).toEqual([]);
  });
});
