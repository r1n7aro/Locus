import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function markdownFiles(root: string): string[] {
  const absoluteRoot = resolve(cwd, root);
  const output: string[] = [];
  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) visit(path);
      else if (entry.isFile() && entry.name.endsWith(".md")) output.push(path);
    }
  };
  visit(absoluteRoot);
  return output;
}

function frontmatter(content: string): string {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  expect(match, "Markdown Skill must start with YAML frontmatter").not.toBeNull();
  return match?.[1] ?? "";
}

describe("Locus Markdown Skill format", () => {
  it("stores every single-file Skill summary in canonical frontmatter", () => {
    const files = markdownFiles("knowledge/skill");
    expect(files.length).toBeGreaterThan(0);

    for (const file of files) {
      const content = readFileSync(file, "utf8");
      const metadata = frontmatter(content);
      expect(metadata, file).toMatch(/^summary:\s*(?:>-|>\+?|\|-?|\|\+?|.+)$/m);
      expect(content, file).not.toMatch(/^## (?:Summary|Content|L1)\s*$/m);
      for (const obsolete of [
        "type",
        "path",
        "title",
        "summaryEnabled",
        "commandEnabled",
        "createdAt",
        "updatedAt",
      ]) {
        expect(metadata, `${file}: ${obsolete}`).not.toMatch(
          new RegExp(`^${obsolete}:`, "m"),
        );
      }
      expect(metadata, `${file}: readOnly`).not.toMatch(/^readOnly:\s*false\s*$/m);
    }
  });

  it("stores every package root summary in SKILL.md frontmatter", () => {
    const files = markdownFiles("skills").filter((file) => file.endsWith("SKILL.md"));
    expect(files.length).toBeGreaterThan(0);

    for (const file of files) {
      const content = readFileSync(file, "utf8");
      const metadata = frontmatter(content);
      expect(metadata, file).toMatch(/^summary:\s*(?:>-|>\+?|\|-?|\|\+?|.+)$/m);
      expect(content, file).not.toMatch(/^## (?:Summary|Content|L1)\s*$/m);

      const manifestPath = join(dirname(file), "skill.json");
      const manifestText = readFileSync(manifestPath, "utf8");
      const manifest = JSON.parse(manifestText);
      expect(manifest.description, manifestPath).toEqual(expect.any(String));
      expect(manifest.description.trim(), manifestPath).not.toBe("");
      expect(manifestText, manifestPath).not.toMatch(
        /"(?:argument-hint|disable-model-invocation|user-invocable)"/,
      );
    }
  });
});
