import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("vite dev watch", () => {
  it("ignores .NET compile-server build outputs on Windows", () => {
    const config = read("vite.config.ts");

    expect(config).toContain("\"**/locus_compile_server/**/bin/**\"");
    expect(config).toContain("\"**/locus_compile_server/**/obj/**\"");
    expect(config).toContain("apphost.exe");
    expect(config).toContain("EBUSY");
  });
});
