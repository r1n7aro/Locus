import {
  skillSurfaceAllowsAuto,
  skillSurfaceAllowsCommand,
  type KnowledgeInjectMode,
  type SkillConfig,
  type SkillManifest,
  type SkillSurface,
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

// ── Skill activation model ────────────────────────────────────
// A skill has two independent channels gated by one master switch:
//   - command channel: the slash trigger (surface command side)
//   - auto channel: structure injection + model recall, active only when the
//     surface auto side is on AND injectMode is path/excerpt
// The UI edits the channels as "inject mode" (none/path/excerpt) and a
// "command" toggle; the surface value is derived from the two.

export type SkillInjectSelection = "none" | "path" | "excerpt";

/** Effective auto-channel inject mode: none whenever the surface auto side is off. */
export function effectiveSkillInjectMode(
  surface: SkillSurface | null | undefined,
  injectMode: KnowledgeInjectMode | null | undefined,
): SkillInjectSelection {
  if (surface != null && !skillSurfaceAllowsAuto(surface)) return "none";
  return injectMode === "path" || injectMode === "excerpt" ? injectMode : "none";
}

/**
 * Derive the stored surface from the two channel toggles. Both channels off
 * is persisted as `auto` + injectMode none, which the recall gate treats as
 * inactive; SkillSurface has no dedicated "none" variant.
 */
export function deriveSkillSurface(commandOn: boolean, autoOn: boolean): SkillSurface {
  if (commandOn && autoOn) return "both";
  if (commandOn) return "command";
  return "auto";
}

/**
 * True when the skill will not take effect through any channel — either the
 * master switch is off, or both the command and auto channels are off. Used
 * for the tree dimming and the detail-page warning.
 */
export function skillActivationInactive(item: {
  skillEnabled?: boolean | null;
  skillSurface?: SkillSurface | null;
  injectMode?: KnowledgeInjectMode | null;
}): boolean {
  if (item.skillEnabled === false) return true;
  if (skillSurfaceAllowsCommand(item.skillSurface ?? undefined)) return false;
  return effectiveSkillInjectMode(item.skillSurface, item.injectMode) === "none";
}
