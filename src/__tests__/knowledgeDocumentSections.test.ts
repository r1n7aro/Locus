import { describe, expect, it } from "vitest";
import { getKnowledgeDocumentEditorSections } from "../components/knowledge/knowledgeDocumentSections";

describe("knowledgeDocumentSections", () => {
  it("hides summary for design documents by default", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "design",
        summary: null,
        summaryEnabled: false,
        maintenanceRules: null,
        aiMaintained: false,
        explicitMaintenanceRules: false,
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
        summaryEnabled: false,
        maintenanceRules: null,
        aiMaintained: false,
        explicitMaintenanceRules: true,
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
        summaryEnabled: true,
        maintenanceRules: "- Refresh after release changes",
        aiMaintained: false,
        explicitMaintenanceRules: true,
      }),
    ).toEqual({
      summary: true,
      maintenanceRules: true,
      body: true,
    });
  });

  it("shows summary when the config is enabled", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "reference",
        summary: null,
        summaryEnabled: true,
        maintenanceRules: null,
        aiMaintained: false,
        explicitMaintenanceRules: false,
      }),
    ).toEqual({
      summary: true,
      maintenanceRules: false,
      body: true,
    });
  });

  it("shows maintenance rules whenever ai maintained is enabled", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "reference",
        summary: null,
        summaryEnabled: false,
        maintenanceRules: null,
        aiMaintained: true,
        explicitMaintenanceRules: true,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: true,
      body: true,
    });
  });

  it("keeps cached optional content hidden when the switches are off", () => {
    expect(
      getKnowledgeDocumentEditorSections({
        type: "design",
        summary: "Cached summary",
        summaryEnabled: false,
        maintenanceRules: "- Cached rule",
        aiMaintained: false,
        explicitMaintenanceRules: false,
      }),
    ).toEqual({
      summary: false,
      maintenanceRules: false,
      body: true,
    });
  });
});
