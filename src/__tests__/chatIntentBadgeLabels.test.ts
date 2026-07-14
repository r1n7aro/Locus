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
    expect(transcript).toContain('badge.kind === "skill" ? "SKILL" : "MODE"');
    expect(richInput).not.toContain("label: `SKILL: ${skill.name}`,");
    expect(transcript).not.toContain("label: `SKILL: ${skill.name}`,");
    expect(richInput).not.toContain("label: `Skill: ${skill.name}`,");
    expect(transcript).not.toContain("label: `Skill: ${skill.name}`,");
  });

  it("renders the plan badge with the same segmented pill structure as skill badges", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    // Composer: MODE mark segment + shared grid styling with the skill badge.
    expect(richInput).toContain(">MODE<");
    expect(richInput).toMatch(/\.composer-badge\.plan,\s*\.composer-badge\.skill\s*\{/);
    expect(richInput).toContain('@click="removePlanBadge"');
    // The old flat pill (whole-badge click target, bespoke plan styling) is gone.
    expect(richInput).not.toMatch(/\.composer-badge\.plan\s*\{/);
    expect(richInput).not.toMatch(/\.composer-badge\.plan:hover/);

    // Transcript: plan shares the segmented layout and the accent theme —
    // no hardcoded blue palette detached from --accent-color.
    expect(transcript).toMatch(/\.chat-transcript-intent-badge\.plan,\s*\.chat-transcript-intent-badge\.skill\s*\{/);
    expect(transcript).not.toContain("#1d4ed8");
    expect(transcript).not.toContain("#3b82f6");
  });

  it("enters sticky plan mode immediately when /plan is picked on an idle session", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain("chatStore.setSessionPlanMode(sessionId, true)");
    expect(richInput).toContain("if (sessionId && chatStore.activeSessionPlanMode) return;");
    expect(richInput).toContain("applyPlanIntentBadge()");
  });
});
