import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();
const srcRoot = resolve(cwd, "src");

function collectVueFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stats = statSync(full);
    if (stats.isDirectory()) collectVueFiles(full, out);
    else if (entry.endsWith(".vue")) out.push(full);
  }
  return out;
}

describe("native <select> ban", () => {
  // Native <select> popups ignore the app theme. Every dropdown must be the
  // themed BaseDropdown (use `teleport` inside scroll containers).
  it("uses BaseDropdown instead of native <select> in every component", () => {
    const offenders = collectVueFiles(srcRoot)
      .filter((file) => /<select[\s>/]/.test(readFileSync(file, "utf8")))
      .map((file) => relative(cwd, file).replaceAll("\\", "/"));
    expect(offenders).toEqual([]);
  });
});
