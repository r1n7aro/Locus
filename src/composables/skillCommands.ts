import {
  skillSurfaceAllowsAuto,
  skillSurfaceAllowsCommand,
  type SkillConfig,
  type SkillManifest,
} from "../types";

export const BUILTIN_COMMAND_NAMES = ["/clear", "/compact", "/plan"] as const;
export const SKILL_COMMAND_NOTICE_OPERATION = "knowledgeSkillCommandTrigger";

const COMMAND_BOUNDARY_RE = /[\s,，。！？!?:：;；()[\]{}<>《》「」『』"“”'‘’]/;

export interface SkillCommandConflict {
  type: "builtin" | "skill";
  command: string;
  skillName?: string;
}

export function normalizeSkillCommandTrigger(value: string, fallback = ""): string {
  const seed = (value || "").trim() || (fallback || "").trim();
  const trimmed = seed.replace(/^\/+/, "").trim();
  return trimmed ? `/${trimmed}` : "";
}

export function isValidSkillCommandTrigger(value: string): boolean {
  const normalized = normalizeSkillCommandTrigger(value);
  if (normalized.length <= 1) return false;
  return !COMMAND_BOUNDARY_RE.test(normalized.slice(1));
}

export function resolveSkillCommandTrigger(
  skill: Pick<SkillManifest, "commandTrigger" | "name">,
): string {
  return normalizeSkillCommandTrigger(skill.commandTrigger, skill.name);
}

export function skillHasCommandEnabled(
  skill: Pick<SkillManifest, "skillEnabled" | "skillSurface">,
): boolean {
  return skill.skillEnabled !== false && skillSurfaceAllowsCommand(skill.skillSurface);
}

export function findSkillCommandConflict(
  trigger: string,
  skills: SkillManifest[],
  currentSkill?: { source: SkillManifest["source"]; dirName: string },
): SkillCommandConflict | null {
  const normalized = normalizeSkillCommandTrigger(trigger);
  if (!normalized) return null;
  const normalizedLower = normalized.toLowerCase();

  if (BUILTIN_COMMAND_NAMES.some((name) => name.toLowerCase() === normalizedLower)) {
    return {
      type: "builtin",
      command: normalized,
    };
  }

  for (const skill of skills) {
    if (
      currentSkill
      && skill.source === currentSkill.source
      && skill.dirName === currentSkill.dirName
    ) {
      continue;
    }
    if (!skillHasCommandEnabled(skill)) continue;
    if (resolveSkillCommandTrigger(skill).toLowerCase() !== normalizedLower) continue;
    return {
      type: "skill",
      command: normalized,
      skillName: skill.name,
    };
  }

  return null;
}

export function buildSkillConfigForCommandToggle(
  skill: Pick<SkillManifest, "name" | "skillEnabled" | "skillSurface">,
  commandEnabled: boolean,
  commandTrigger: string,
): SkillConfig {
  const allowsAuto = skillSurfaceAllowsAuto(skill.skillSurface);

  // description is intentionally omitted: sending the effective summary here
  // would pin it as a workspace override and shadow later `## L1` updates.
  return {
    enabled: commandEnabled ? true : allowsAuto ? skill.skillEnabled !== false : false,
    surface: commandEnabled ? (allowsAuto ? "both" : "command") : allowsAuto ? "auto" : "command",
    commandTrigger: normalizeSkillCommandTrigger(commandTrigger, skill.name),
  };
}
