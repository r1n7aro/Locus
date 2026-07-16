import { describe, expect, it } from "vitest";
import {
  buildSkillConfigForCommandToggle,
  deriveSkillSurface,
  effectiveSkillInjectMode,
  findSkillCommandConflict,
  normalizeSkillCommandTrigger,
  resolveSkillCommandTrigger,
  skillActivationInactive,
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

  describe("activation model", () => {
    it("reads the effective inject mode as none when the surface auto side is off", () => {
      expect(effectiveSkillInjectMode("command", "excerpt")).toBe("none");
      expect(effectiveSkillInjectMode("both", "excerpt")).toBe("excerpt");
      expect(effectiveSkillInjectMode("auto", "path")).toBe("path");
      expect(effectiveSkillInjectMode("auto", "none")).toBe("none");
      expect(effectiveSkillInjectMode("auto", undefined)).toBe("none");
      // md skill documents may omit the surface entirely; only an explicit
      // non-auto surface forces none.
      expect(effectiveSkillInjectMode(undefined, "excerpt")).toBe("excerpt");
    });

    it("derives the surface from the two channel toggles", () => {
      expect(deriveSkillSurface(true, true)).toBe("both");
      expect(deriveSkillSurface(true, false)).toBe("command");
      expect(deriveSkillSurface(false, true)).toBe("auto");
      // Both channels off persists as auto + injectMode none.
      expect(deriveSkillSurface(false, false)).toBe("auto");
    });

    it("flags skills with no active channel for dimming and warnings", () => {
      expect(
        skillActivationInactive({
          skillEnabled: false,
          skillSurface: "both",
          injectMode: "excerpt",
        }),
      ).toBe(true);
      expect(
        skillActivationInactive({
          skillEnabled: true,
          skillSurface: "auto",
          injectMode: "none",
        }),
      ).toBe(true);
      expect(
        skillActivationInactive({
          skillEnabled: true,
          skillSurface: "command",
          injectMode: "none",
        }),
      ).toBe(false);
      expect(
        skillActivationInactive({
          skillEnabled: true,
          skillSurface: "auto",
          injectMode: "excerpt",
        }),
      ).toBe(false);
      // Command-only with a stored excerpt mode stays active via the command
      // channel even though its auto channel is off.
      expect(
        skillActivationInactive({
          skillEnabled: true,
          skillSurface: "command",
          injectMode: "excerpt",
        }),
      ).toBe(false);
    });
  });
});
