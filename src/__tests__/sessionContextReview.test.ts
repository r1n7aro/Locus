import { describe, expect, it } from "vitest";
import {
  buildContextReviewDraft,
  contextReviewAttachmentName,
  resolveContextReviewIntent,
} from "../composables/sessionContextReview";
import type { SkillManifest } from "../types";

function skill(source: string, name: string): SkillManifest {
  return {
    name,
    description: "",
    argumentHint: "",
    dirName: "review-context",
    source,
    relPath: "review-context/SKILL.md",
    updatedAt: 0,
    skillEnabled: true,
    skillSurface: "both",
    skillDescription: null,
    commandTrigger: "/review-context",
  };
}

describe("session context review contract", () => {
  it("prefers the project review skill and creates a reusable composer draft", () => {
    const draft = buildContextReviewDraft(
      [skill("app", "App Review"), skill("project", "Project Review")],
      "Review this context",
    );

    expect(draft).toMatchObject({
      text: "Review this context",
      localFiles: [],
      intent: {
        mode: "build",
        skills: [{
          dirName: "review-context",
          source: "project",
          name: "Project Review",
        }],
      },
    });
  });

  it("uses the app fallback and derives attachment names across path styles", () => {
    expect(resolveContextReviewIntent([]).skills).toEqual([{
      dirName: "review-context",
      source: "app",
      name: "Review Context",
    }]);
    expect(contextReviewAttachmentName("C:\\temp\\session.yaml", "fallback.yaml"))
      .toBe("session.yaml");
    expect(contextReviewAttachmentName("", "fallback.yaml")).toBe("fallback.yaml");
  });
});
