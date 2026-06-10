import { describe, expect, it } from "vitest";
import {
  buildSkillConfigForCommandToggle,
  findSkillCommandConflict,
  normalizeSkillCommandTrigger,
  resolveSkillCommandTrigger,
  skillHasCommandEnabled,
} from "../composables/skillCommands";
import type { SkillManifest } from "../types";

function makeSkill(overrides: Partial<SkillManifest> = {}): SkillManifest {
  return {
    name: "create-skill",
    description: "",
    argumentHint: "",
    dirName: "create-skill",
    source: "project",
    relPath: "skill/create-skill.md",
    updatedAt: 0,
    skillEnabled: true,
    skillSurface: "command",
    skillDescription: null,
    commandTrigger: "/create-skill",
    ...overrides,
  };
}

describe("skillCommands", () => {
  it("normalizes skill command triggers with a leading slash", () => {
    expect(normalizeSkillCommandTrigger("build-tool")).toBe("/build-tool");
    expect(normalizeSkillCommandTrigger(" /build-tool ")).toBe("/build-tool");
  });

  it("resolves the manifest trigger with fallback to skill name", () => {
    expect(resolveSkillCommandTrigger(makeSkill({ commandTrigger: "" }))).toBe("/create-skill");
  });

  it("checks only registered skill commands for conflicts", () => {
    const conflict = findSkillCommandConflict("/asset-audit", [
      makeSkill({
        name: "asset-audit",
        dirName: "asset-audit",
        relPath: "skill/asset-audit.md",
        commandTrigger: "/asset-audit",
      }),
      makeSkill({
        name: "semantic-only",
        dirName: "semantic-only",
        relPath: "skill/semantic-only.md",
        commandTrigger: "/asset-audit",
        skillSurface: "auto",
      }),
    ]);

    expect(conflict).toMatchObject({
      type: "skill",
      command: "/asset-audit",
      skillName: "asset-audit",
    });
  });

  it("treats auto-only skills as unregistered in the command list", () => {
    expect(
      skillHasCommandEnabled(makeSkill({ skillSurface: "auto" })),
    ).toBe(false);
  });

  it("preserves auto recall when command entry is turned off from both", () => {
    expect(
      buildSkillConfigForCommandToggle(
        makeSkill({ skillSurface: "both", skillDescription: "desc" }),
        false,
        "/custom-trigger",
      ),
    ).toEqual({
      enabled: true,
      surface: "auto",
      commandTrigger: "/custom-trigger",
    });
  });
});
