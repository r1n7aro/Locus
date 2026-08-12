import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string): string {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function frontmatter(markdown: string): string {
  const match = markdown.match(/^---\r?\n([\s\S]*?)\r?\n---/);
  return match?.[1] ?? "";
}

describe("Grill builtin skill", () => {
  it("is an explicit command-only workflow", () => {
    const skill = read("knowledge/skill/grill.md");
    const meta = frontmatter(skill);

    expect(meta).toContain("skillSurface: command");
    expect(meta).toContain("commandTrigger: /grill");
    expect(meta).toContain('argumentHint: "[requirement-or-idea]"');
    expect(meta).not.toMatch(/^(?:title|path|commandEnabled):/m);
  });

  it("can investigate implementation facts without mutating the project", () => {
    const meta = frontmatter(read("knowledge/skill/grill.md"));

    expect(meta).toContain("  - ask");
    expect(meta).toContain("  - knowledge_query");
    expect(meta).toContain("  - code_symbol_search");
    expect(meta).toContain("  - unity_asset_search");
    expect(meta).not.toContain("  - write");
    expect(meta).not.toContain("  - edit");
    expect(meta).not.toContain("  - todowrite");
  });

  it("keeps questioning until implementation choices are settled", () => {
    const skill = read("knowledge/skill/grill.md");

    expect(skill).toContain("usually 2–5 per round");
    expect(skill).toContain("your recommended answer");
    expect(skill).toContain("materially different valid implementations");
    expect(skill).toContain("Implementation brief");
    expect(skill).toContain("Do not start implementation in the same turn.");
  });

  it("stays lightweight and avoids workflow-management artifacts", () => {
    const skill = read("knowledge/skill/grill.md");

    expect(skill).not.toContain("handoff");
    expect(skill).not.toContain("issue tracker");
    expect(skill).not.toContain("CONTEXT.md");
    expect(skill).not.toContain("ADR");
    expect(skill.toLowerCase()).not.toContain("session");
  });
});
