import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat intent badge labels", () => {
  it("uses compact uppercase SKILL markers in the composer and transcript badges", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(richInput).toContain("label: skill.name,");
    expect(transcript).toContain("label: skill.name,");
    expect(richInput).toContain("composer-badge-mark");
    expect(transcript).toContain("chat-transcript-intent-badge-mark");
    expect(richInput).toContain("height: 28px;");
    expect(transcript).toContain("min-height: 28px;");
    expect(richInput).toContain('class="composer-badge-remove"');
    expect(richInput).toContain('@click="badge.skill ? removeSkillBadge(badge.skill) : undefined"');
    expect(richInput).toContain(">SKILL<");
    expect(transcript).toContain(">SKILL<");
    expect(richInput).not.toContain("label: `SKILL: ${skill.name}`,");
    expect(transcript).not.toContain("label: `SKILL: ${skill.name}`,");
    expect(richInput).not.toContain("label: `Skill: ${skill.name}`,");
    expect(transcript).not.toContain("label: `Skill: ${skill.name}`,");
  });
});
