import { describe, expect, it } from "vitest";
import {
  resolveKnowledgeTreeKeyboardAction,
  type KnowledgeTreeKeyboardRow,
} from "../components/knowledge/knowledgeExplorerKeyboard";

const rows: KnowledgeTreeKeyboardRow[] = [
  {
    path: "skill/builtin",
    kind: "folder",
    depth: 1,
    expanded: true,
    hasChildren: true,
  },
  {
    path: "skill/builtin/create-skill.md",
    kind: "document",
    depth: 2,
    expanded: false,
    hasChildren: false,
  },
  {
    path: "skill/psd-to-ugui",
    kind: "package",
    depth: 1,
    expanded: false,
    hasChildren: true,
  },
];

function resolve(
  key: string,
  focusedPath: string | null,
  modifiers: { ctrlKey?: boolean; metaKey?: boolean } = {},
) {
  return resolveKnowledgeTreeKeyboardAction({
    key,
    ctrlKey: modifiers.ctrlKey ?? false,
    metaKey: modifiers.metaKey ?? false,
    rows,
    focusedPath,
  });
}

describe("resolveKnowledgeTreeKeyboardAction", () => {
  it("moves the roving focus with arrow up/down and home/end", () => {
    expect(resolve("ArrowDown", null)).toEqual({
      type: "focus",
      path: "skill/builtin",
    });
    expect(resolve("ArrowDown", "skill/builtin")).toEqual({
      type: "focus",
      path: "skill/builtin/create-skill.md",
    });
    expect(resolve("ArrowDown", "skill/psd-to-ugui")).toBeNull();
    expect(resolve("ArrowUp", "skill/builtin")).toBeNull();
    expect(resolve("Home", "skill/psd-to-ugui")).toEqual({
      type: "focus",
      path: "skill/builtin",
    });
    expect(resolve("End", "skill/builtin")).toEqual({
      type: "focus",
      path: "skill/psd-to-ugui",
    });
  });

  it("expands collapsed branches and dives into expanded ones with ArrowRight", () => {
    expect(resolve("ArrowRight", "skill/psd-to-ugui")).toEqual({
      type: "expand",
      path: "skill/psd-to-ugui",
    });
    expect(resolve("ArrowRight", "skill/builtin")).toEqual({
      type: "focus",
      path: "skill/builtin/create-skill.md",
    });
    expect(resolve("ArrowRight", "skill/builtin/create-skill.md")).toBeNull();
  });

  it("collapses expanded branches and walks to the parent with ArrowLeft", () => {
    expect(resolve("ArrowLeft", "skill/builtin")).toEqual({
      type: "collapse",
      path: "skill/builtin",
    });
    expect(resolve("ArrowLeft", "skill/builtin/create-skill.md")).toEqual({
      type: "focus",
      path: "skill/builtin",
    });
    expect(resolve("ArrowLeft", "skill/psd-to-ugui")).toBeNull();
  });

  it("activates, renames, and deletes the focused row", () => {
    expect(resolve("Enter", "skill/builtin")).toEqual({
      type: "activate",
      path: "skill/builtin",
    });
    expect(resolve(" ", "skill/psd-to-ugui")).toEqual({
      type: "activate",
      path: "skill/psd-to-ugui",
    });
    expect(resolve("F2", "skill/builtin/create-skill.md")).toEqual({
      type: "rename",
      path: "skill/builtin/create-skill.md",
    });
    // Packages cannot be renamed from the tree.
    expect(resolve("F2", "skill/psd-to-ugui")).toBeNull();
    expect(resolve("Delete", "skill/builtin/create-skill.md")).toEqual({
      type: "delete",
      path: "skill/builtin/create-skill.md",
    });
  });

  it("handles select-all and escape regardless of focus", () => {
    expect(resolve("a", null, { ctrlKey: true })).toEqual({
      type: "select-all",
    });
    expect(resolve("A", "skill/builtin", { metaKey: true })).toEqual({
      type: "select-all",
    });
    expect(resolve("Escape", null)).toEqual({ type: "clear-selection" });
  });

  it("ignores unrelated keys and plain letters", () => {
    expect(resolve("a", "skill/builtin")).toBeNull();
    expect(resolve("Tab", "skill/builtin")).toBeNull();
  });
});
