import { describe, expect, it } from "vitest";
import {
  buildKnowledgeDocumentContent,
  splitKnowledgeDocumentContent,
} from "../components/knowledge/knowledgeDocumentContent";

describe("knowledgeDocumentContent", () => {
  it("splits default memory content into rules and body editors", () => {
    const parts = splitKnowledgeDocumentContent(
      "# Project Understanding\n\n## Maintenance Rules\n- Keep only durable notes\n\n## Notes\n- Core loop is exploration\n",
      "Project Understanding",
    );

    expect(parts.maintenanceRules).toBe("- Keep only durable notes");
    expect(parts.body).toBe("- Core loop is exploration");
    expect(parts.bodyHeading).toBe("## Notes");
    expect(parts.hasExplicitRulesSection).toBe(true);
  });

  it("rebuilds a canonical memory document from split editors", () => {
    const content = buildKnowledgeDocumentContent({
      title: "Project Understanding",
      maintenanceRules: "- Keep only durable notes",
      body: "- Core loop is exploration",
      bodyHeading: "## Notes",
    });

    expect(content).toBe(
      "# Project Understanding\n\n## Maintenance Rules\n- Keep only durable notes\n\n## Notes\n- Core loop is exploration\n",
    );
  });

  it("preserves custom body sections when the content already starts with a heading", () => {
    const content = buildKnowledgeDocumentContent({
      title: "Mistake Notebook",
      maintenanceRules: "- Record verified regressions",
      body: "## Regression\n- Fixed by clearing cache",
      bodyHeading: "## Notes",
    });

    expect(content).toBe(
      "# Mistake Notebook\n\n## Maintenance Rules\n- Record verified regressions\n\n## Regression\n- Fixed by clearing cache\n",
    );
  });
});
