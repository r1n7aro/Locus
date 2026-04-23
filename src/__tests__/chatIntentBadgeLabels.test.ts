import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat intent badge labels", () => {
  it("uses uppercase SKILL labels in the composer and transcript badges", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(richInput).toContain("label: `SKILL: ${skill.name}`,");
    expect(transcript).toContain("label: `SKILL: ${skill.name}`,");
    expect(richInput).not.toContain("label: `Skill: ${skill.name}`,");
    expect(transcript).not.toContain("label: `Skill: ${skill.name}`,");
  });
});
