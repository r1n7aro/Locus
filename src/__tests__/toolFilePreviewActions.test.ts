import { describe, expect, it } from "vitest";
import {
  resolveToolFilePreviewPath,
  resolveToolFilePreviewPayload,
} from "../components/toolFilePreviewActions";

describe("resolveToolFilePreviewPath", () => {
  it("returns the single read, edited, or written file after the tool completes", () => {
    expect(resolveToolFilePreviewPath({
      name: "read",
      arguments: JSON.stringify({ filePath: "src/App.vue" }),
      status: "done",
    })).toBe("src/App.vue");
    expect(resolveToolFilePreviewPath({
      name: "edit",
      arguments: JSON.stringify({ filePath: "src/App.vue" }),
      status: "done",
    })).toBe("src/App.vue");
    expect(resolveToolFilePreviewPath({
      name: "write",
      arguments: JSON.stringify({ file_path: "notes/design.md" }),
      status: "done",
    })).toBe("notes/design.md");
  });

  it("describes write and edit highlighting without embedding full edit text", () => {
    expect(resolveToolFilePreviewPayload({
      name: "write",
      arguments: JSON.stringify({ filePath: "notes/design.md", content: "body" }),
      status: "done",
    })).toEqual({ filePath: "notes/design.md", highlight: { mode: "all" } });

    const editPayload = resolveToolFilePreviewPayload({
      name: "edit",
      arguments: JSON.stringify({
        filePath: "src/App.vue",
        oldString: "before\nold\nafter",
        newString: "before\nnew\nafter",
      }),
      output: "Edited src/App.vue [lines:12]",
      status: "done",
    });
    expect(editPayload?.highlight).toEqual({
      mode: "edit",
      targets: [expect.objectContaining({
        startLine: 12,
        highlightStartLineOffset: 1,
        highlightEndLineOffset: 1,
      })],
    });
    expect(JSON.stringify(editPayload)).not.toContain('"oldString"');
    expect(JSON.stringify(editPayload)).not.toContain('"newString"');
  });

  it("hides the action for unfinished tools and invalid arguments", () => {
    expect(resolveToolFilePreviewPath({
      name: "grep",
      arguments: JSON.stringify({ filePath: "src/App.vue" }),
      status: "done",
    })).toBe("");
    expect(resolveToolFilePreviewPath({
      name: "edit",
      arguments: JSON.stringify({ filePath: "src/App.vue" }),
      status: "running",
    })).toBe("");
    expect(resolveToolFilePreviewPath({
      name: "write",
      arguments: "{",
      status: "done",
    })).toBe("");
  });
});
