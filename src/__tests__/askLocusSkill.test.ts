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

describe("Ask Locus builtin skill", () => {
  it("uses a natural command name without declaring built-in tool dependencies", () => {
    const skill = read("knowledge/skill/ask-locus.md");
    const meta = frontmatter(skill);

    expect(meta).toContain("title: Ask Locus");
    expect(meta).toContain("path: ask-locus.md");
    expect(meta).toContain("skillSurface: both");
    expect(meta).toContain("commandEnabled: true");
    expect(meta).toContain("commandTrigger: /ask-locus");
    expect(meta).not.toContain("tools:");
    expect(meta.toLowerCase()).not.toContain("usage");
  });

  it("keeps source browsing tied to a reusable temp cache", () => {
    const skill = read("knowledge/skill/ask-locus.md");

    expect(skill).toContain("<app-temp>/ask-locus/Locus/");
    expect(skill).toContain("Repository: `https://github.com/r1n7aro/Locus`");
    expect(skill).toContain("Do not clone on every question.");
    expect(skill).toContain("older than 24 hours");
    expect(skill).toContain("git clone --depth=1 https://github.com/r1n7aro/Locus.git");
    expect(skill).toContain("git -C $repo pull --ff-only --depth=1");
    expect(skill).toContain("If refresh fails, keep the existing checkout");
    expect(skill).toContain("Do not edit files in the cached checkout.");
  });

  it("documents GitHub issue creation for confirmed Locus bugs", () => {
    const skill = read("knowledge/skill/ask-locus.md");

    expect(skill).toContain("## Bug reporting");
    expect(skill).toContain("gh auth status -h github.com");
    expect(skill).toContain("r1n7aro/Locus");
    expect(skill).toContain("gh issue create --repo r1n7aro/Locus");
    expect(skill).toContain("Ask the user to confirm the final title and body before publishing.");
    expect(skill).toContain("Return the created issue URL.");
  });

  it("answers in the user's requested language", () => {
    const skill = read("knowledge/skill/ask-locus.md");

    expect(skill).toContain("Reply in the same language as the user's question");
    expect(skill).toContain("unless the user explicitly asks for another language");
  });
});
