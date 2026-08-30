import { describe, expect, it } from "vitest";
import {
  buildKnowledgeDocumentEditOperations,
  buildKnowledgeTextHunks,
  rebaseKnowledgeText,
} from "../components/knowledge/knowledgeCollaborativeEditing";

describe("knowledgeCollaborativeEditing", () => {
  it("merges non-overlapping local and remote replacements", () => {
    const result = rebaseKnowledgeText(
      "alpha\nbeta\ngamma",
      "alpha\nbeta-local\ngamma",
      "alpha\nbeta\ngamma-agent",
    );

    expect(result.text).toBe("alpha\nbeta-local\ngamma-agent");
    expect(result.conflicts).toEqual([]);
  });

  it("keeps the local replacement visible and reports an overlapping conflict", () => {
    const result = rebaseKnowledgeText(
      "alpha\nbeta\ngamma",
      "alpha\nbeta-local\ngamma",
      "alpha\nbeta-agent\ngamma",
    );

    expect(result.text).toBe("alpha\nbeta-local\ngamma");
    expect(result.conflicts).toHaveLength(1);
  });

  it("accepts the remote conflicting hunk while retaining other local hunks", () => {
    const result = rebaseKnowledgeText(
      "alpha\nbeta\ngamma",
      "alpha-local\nbeta-local\ngamma",
      "alpha\nbeta-agent\ngamma",
    );

    expect(result.text).toBe("alpha-local\nbeta-local\ngamma");
    expect(result.remotePreferredText).toBe("alpha-local\nbeta-agent\ngamma");
    expect(result.conflicts).toHaveLength(1);
  });

  it("recognizes an identical replacement as already synchronized", () => {
    const result = rebaseKnowledgeText(
      "alpha\nbeta",
      "alpha\nbeta-updated",
      "alpha\nbeta-updated",
    );

    expect(result.text).toBe("alpha\nbeta-updated");
    expect(result.conflicts).toEqual([]);
    expect(buildKnowledgeTextHunks("alpha\nbeta-updated", result.text)).toEqual([]);
  });

  it("keeps typing that continues while an earlier draft is being saved", () => {
    const result = rebaseKnowledgeText(
      "draft saved",
      "draft saved and continued",
      "draft saved",
    );

    expect(result.text).toBe("draft saved and continued");
    expect(result.conflicts).toEqual([]);
  });

  it("builds small replacement operations instead of replacing the whole section", () => {
    const operations = buildKnowledgeDocumentEditOperations(
      "body",
      "alpha\nbeta\ngamma",
      "alpha\nbeta-local\ngamma",
    );

    expect(operations).toHaveLength(1);
    expect(operations[0]?.section).toBe("body");
    expect(operations[0]?.oldString.length).toBeLessThan("alpha\nbeta\ngamma".length);
    expect("alpha\nbeta\ngamma".replace(
      operations[0]!.oldString,
      operations[0]!.newString,
    )).toBe("alpha\nbeta-local\ngamma");
  });

  it("guards insertion into an empty section", () => {
    expect(buildKnowledgeDocumentEditOperations("summary", "", "new summary"))
      .toEqual([{
        section: "summary",
        oldString: "",
        newString: "new summary",
        expectedEmpty: true,
      }]);
  });

  it("builds bounded multi-point save hunks for repetitive 1 MB sections", () => {
    const base = `${"a".repeat(524_288)}MIDDLE${"z".repeat(524_281)}`;
    const next = `AAA${base.slice(3, -3)}ZZZ`;
    const startedAt = performance.now();
    const hunks = buildKnowledgeTextHunks(base, next);

    expect(performance.now() - startedAt).toBeLessThan(250);
    expect(hunks).toEqual([
      { start: 0, end: 3, oldText: "aaa", newText: "AAA" },
      {
        start: base.length - 3,
        end: base.length,
        oldText: "zzz",
        newText: "ZZZ",
      },
    ]);

    const operationsStartedAt = performance.now();
    const operations = buildKnowledgeDocumentEditOperations("body", base, next);
    expect(performance.now() - operationsStartedAt).toBeLessThan(500);
    const applied = operations.reduce(
      (content, operation) => content.replace(operation.oldString, operation.newString),
      base,
    );
    expect(applied).toBe(next);
  });
});
