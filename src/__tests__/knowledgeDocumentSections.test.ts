import { describe, expect, it } from "vitest";
import { getKnowledgeDocumentEditorSections } from "../components/knowledge/knowledgeDocumentSections";

describe("knowledgeDocumentSections", () => {
  it("hides summary for design documents by default", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "design",
        summary: null,
        maintenanceRules: null,
        aiMaintained: false,
        effectiveAiMaintained: false,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: false,
      body: true,
    });
  });

  it("shows maintenance rules when the explicit rules config is enabled", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "memory",
        summary: null,
        maintenanceRules: null,
        aiMaintained: false,
        effectiveAiMaintained: false,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: true,
      body: true,
    });
  });

  it("keeps existing optional sections visible", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "skill",
        summary: "Quick guide",
        maintenanceRules: "- Refresh after release changes",
        aiMaintained: false,
        effectiveAiMaintained: false,
      }),
    ).toEqual({
      summary: true,
      maintenanceRules: true,
      body: true,
    });
  });

  it("keeps an absent summary disabled", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "reference",
        summary: null,
        maintenanceRules: null,
        aiMaintained: false,
        effectiveAiMaintained: false,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: false,
      body: true,
    });
  });

  it("shows maintenance rules whenever ai maintained is enabled", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "reference",
        summary: null,
        maintenanceRules: null,
        aiMaintained: true,
        effectiveAiMaintained: true,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: true,
      body: true,
    });
  });

  it("enables optional content by field presence", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "design",
        summary: "Cached summary",
        maintenanceRules: "- Cached rule",
        aiMaintained: false,
        effectiveAiMaintained: false,
      }),
    ).toEqual({
      summary: true,
      maintenanceRules: true,
      body: true,
    });
  });
});
