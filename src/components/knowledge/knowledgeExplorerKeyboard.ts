export interface KnowledgeTreeKeyboardRow {
  path: string;
  kind: "folder" | "package" | "document";
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

export type KnowledgeTreeKeyboardAction =
  | { type: "focus"; path: string }
  | { type: "expand"; path: string }
  | { type: "collapse"; path: string }
  | { type: "activate"; path: string }
  | { type: "rename"; path: string }
  | { type: "delete"; path: string }
  | { type: "select-all" }
  | { type: "clear-selection" };

export interface ResolveKnowledgeTreeKeyboardInput {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  rows: KnowledgeTreeKeyboardRow[];
  focusedPath: string | null;
}

/**
 * Map a keydown on the knowledge tree to a tree action. Pure so the roving
 * focus / expand / activate semantics stay unit-testable apart from the
 * virtualized DOM.
 */
export function resolveKnowledgeTreeKeyboardAction(
  input: ResolveKnowledgeTreeKeyboardInput,
): KnowledgeTreeKeyboardAction | null {
  const { key, ctrlKey, metaKey, rows, focusedPath } = input;

  if ((ctrlKey || metaKey) && (key === "a" || key === "A")) {
    return rows.length ? { type: "select-all" } : null;
  }
  if (key === "Escape") return { type: "clear-selection" };
  if (!rows.length) return null;

  const index = focusedPath
    ? rows.findIndex((row) => row.path === focusedPath)
    : -1;
  const focused = index >= 0 ? rows[index] : null;
  const first = rows[0]!;
  const last = rows[rows.length - 1]!;

  switch (key) {
    case "ArrowDown":
      if (!focused) return { type: "focus", path: first.path };
      return index + 1 < rows.length
        ? { type: "focus", path: rows[index + 1]!.path }
        : null;
    case "ArrowUp":
      if (!focused) return { type: "focus", path: last.path };
      return index > 0 ? { type: "focus", path: rows[index - 1]!.path } : null;
    case "Home":
      return { type: "focus", path: first.path };
    case "End":
      return { type: "focus", path: last.path };
    case "ArrowRight": {
      if (!focused) return { type: "focus", path: first.path };
      if (focused.kind === "document" || !focused.hasChildren) return null;
      if (!focused.expanded) return { type: "expand", path: focused.path };
      const next = rows[index + 1];
      return next && next.depth > focused.depth
        ? { type: "focus", path: next.path }
        : null;
    }
    case "ArrowLeft": {
      if (!focused) return { type: "focus", path: first.path };
      if (focused.kind !== "document" && focused.expanded && focused.hasChildren) {
        return { type: "collapse", path: focused.path };
      }
      for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
        if (rows[cursor]!.depth < focused.depth) {
          return { type: "focus", path: rows[cursor]!.path };
        }
      }
      return null;
    }
    case "Enter":
    case " ":
      return focused ? { type: "activate", path: focused.path } : null;
    case "F2":
      return focused && focused.kind !== "package"
        ? { type: "rename", path: focused.path }
        : null;
    case "Delete":
      return focused ? { type: "delete", path: focused.path } : null;
    default:
      return null;
  }
}
