import type { UserMessageDraft } from "./chatMessageDraft";
import type {
  SkillIntentItem,
  SkillManifest,
  UserIntentMeta,
} from "../types";

const FALLBACK_REVIEW_CONTEXT_SKILL: SkillIntentItem = {
  dirName: "review-context",
  source: "app",
  name: "Review Context",
};

export function resolveContextReviewIntent(
  skills: readonly SkillManifest[],
): UserIntentMeta {
  const manifest = skills.find((skill) => (
    skill.dirName === "review-context" && skill.source === "project"
  )) ?? skills.find((skill) => (
    skill.dirName === "review-context" && skill.source === "app"
  ));
  const skill: SkillIntentItem = manifest
    ? {
        dirName: manifest.dirName,
        source: manifest.source,
        name: manifest.name,
      }
    : FALLBACK_REVIEW_CONTEXT_SKILL;
  return {
    kind: "user_intent_v1",
    mode: "build",
    skills: [skill],
  };
}

export function buildContextReviewDraft(
  skills: readonly SkillManifest[],
  prompt: string,
): UserMessageDraft {
  const intent = resolveContextReviewIntent(skills);
  return {
    text: prompt,
    images: [],
    assetRefs: [],
    localFiles: [],
    consoleTexts: [],
    intent: {
      mode: intent.mode,
      skills: intent.skills,
    },
  };
}

export function contextReviewAttachmentName(filePath: string, fallback: string): string {
  return filePath.replace(/\\/g, "/").split("/").filter(Boolean).pop() || fallback;
}
