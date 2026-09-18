import { describe, expect, it } from "vitest";
import { documentSessionKey } from "../document/documentIdentity";
import { DocumentSessionCache } from "../document/documentSessionCache";
import {
  buildDocumentTextEditOperations,
  buildDocumentTextHunks,
  rebaseDocumentText,
} from "../document/documentText";
import {
  detectTextDocumentLineEnding,
  normalizeTextDocumentLineEndings,
  serializeTextDocumentLineEndings,
} from "../document/textDocumentFormat";

describe("document infrastructure", () => {
  it("isolates resource types, checkouts, generations and rematerialized workspaces", () => {
    const workspace = { checkoutId: "a", expectedGeneration: 1, expectedMaterializationEpoch: 2 };
    const keys = [
      documentSessionKey(workspace, ["file", "same"]),
      documentSessionKey(workspace, ["directory", "same"]),
      documentSessionKey({ ...workspace, checkoutId: "b" }, ["file", "same"]),
      documentSessionKey({ ...workspace, expectedGeneration: 2 }, ["file", "same"]),
      documentSessionKey({ ...workspace, expectedMaterializationEpoch: 3 }, ["file", "same"]),
      documentSessionKey(null, ["file", "same"]),
    ];
    expect(new Set(keys).size).toBe(keys.length);
    expect(documentSessionKey(null, ["a:b", "c"]))
      .not.toBe(documentSessionKey(null, ["a", "b:c"]));
  });

  it("pins dirty drafts above capacity and evicts them when they become clean without touching recency", () => {
    const cache = new DocumentSessionCache<{ dirty: boolean }>({ capacity: 2, canEvict: (v) => !v.dirty });
    for (const key of ["a", "b", "c"]) cache.set(key, { dirty: true });
    expect(cache.size).toBe(3);
    cache.replace("a", { dirty: false });
    expect(cache.keys()).toEqual(["b", "c"]);
    cache.replace("b", { dirty: false });
    expect(cache.keys()).toEqual(["b", "c"]);
    cache.set("d", { dirty: false });
    expect(cache.keys()).toEqual(["c", "d"]);
  });

  it("preserves BOM, trailing whitespace, final empty fields and blank lines in text edits", () => {
    const base = "\ufeffid,value\r\n001,  ,\r\n\r\n";
    const next = "\ufeffid,value\r\n002,  ,\r\n\r\n";
    const operations = buildDocumentTextEditOperations(base, next);
    expect(operations.reduce((text, edit) => text.replace(edit.oldString, edit.newString), base)).toBe(next);
    expect(buildDocumentTextHunks("x\n", "x\n\n")).toHaveLength(1);
    expect(rebaseDocumentText(base, next, base).text).toBe(next);
    expect(rebaseDocumentText("x\n", "x \n", "x\n\n").text).toBe("x \n\n");
  });

  it("reports overlapping text changes without inserting conflict markers into document content", () => {
    const result = rebaseDocumentText("id,width\n1,100\n", "id,width\n1,200\n", "id,width\n1,300\n");
    expect(result.conflicts).toHaveLength(1);
    expect(result.text).toBe("id,width\n1,200\n");
    expect(result.remotePreferredText).toBe("id,width\n1,300\n");
  });

  it.each(["\n", "\r\n", "\r"] as const)("round trips uniform %j newlines without trimming", (lineEnding) => {
    const original = `\ufefffirst ${lineEnding}${lineEnding}last,${lineEnding}`;
    expect(detectTextDocumentLineEnding(original)).toBe(lineEnding);
    expect(serializeTextDocumentLineEndings(normalizeTextDocumentLineEndings(original), lineEnding))
      .toBe(original);
  });
});
