import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const TEXT_EXTENSIONS = new Set([".css", ".js", ".jsx", ".ts", ".tsx", ".vue"]);
const cwd = process.cwd();
const srcRoot = resolve(cwd, "src");
const typographyFile = resolve(srcRoot, "styles", "typography.css");
const fontProperty = ["font", "family"].join("-");
const legacyFontToken = ["--font", "mono"].join("-");
const fontFamilyRe = new RegExp(`${fontProperty}\\s*:\\s*([^;]+);`, "g");
const legacyFontTokenRe = new RegExp(`var\\(${legacyFontToken}\\s*,`);
const allowedFontFamilies = new Set([
  "inherit",
  "var(--font-ui)",
  "var(--font-prose)",
  "var(--font-mono-inline)",
  "var(--font-mono-identifier)",
  "var(--font-mono-block)",
  "var(--font-mono-editor)",
  "var(--font-mono-display)",
]);

function collectTextFiles(dir: string): string[] {
  const files: string[] = [];

    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const fullPath = join(dir, entry.name);

      if (entry.isDirectory()) {
        if (entry.name === "__tests__") continue;
        files.push(...collectTextFiles(fullPath));
        continue;
      }

    if (TEXT_EXTENSIONS.has(extname(entry.name))) {
      files.push(fullPath);
    }
  }

  return files;
}

describe("typography tokens", () => {
  it("uses only semantic font tokens outside typography.css", () => {
    const violations: string[] = [];

    for (const file of collectTextFiles(srcRoot)) {
      if (file === typographyFile) continue;

      const content = readFileSync(file, "utf8");
      const relPath = relative(cwd, file).replaceAll("\\", "/");

      if (legacyFontTokenRe.test(content)) {
        violations.push(`${relPath}: uses legacy mono font fallback token`);
      }

      for (const match of content.matchAll(fontFamilyRe)) {
        const value = match[1]?.trim();
        if (value && !allowedFontFamilies.has(value)) {
          violations.push(`${relPath}: ${fontProperty}: ${value}`);
        }
      }
    }

    expect(violations).toEqual([]);
  });
});
