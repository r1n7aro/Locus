import { readFileSync } from "node:fs";
import ts from "typescript";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { workspaceTreeItemState } from "../components/explorer/workspaceTreeItemState";
import { resolveWorkspaceSessionSelection } from "../components/workbench/workspaceSessionSelection";

// Run the production selection and drag-source handlers for document/file rows of the layout tree.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = [
  "isLayoutFileItem", "isFileSelectionItem", "fileSelectionGroupItems", "visibleWorkspaceFileItems",
  "resolveFileClickSelection", "onDragPointerDown",
];
const handlers = parsed.statements.filter((statement) => ts.isFunctionDeclaration(statement)
  && statement.name && names.includes(statement.name.text)).map((statement) => statement.getText(parsed)).join("\n");
const code = ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

type Item = { key: string; treeRow: { key: string; name: string; classes: Record<string, boolean> }; meta: Record<string, unknown> };

function row(key: string, meta: Record<string, unknown>, open = false): Item {
  return { key, treeRow: { key, name: key, classes: { "is-open": open } }, meta: { projectId: "p", ...meta } };
}

function fixture() {
  const node = (nodeId: string, extra: Record<string, unknown> = {}) => ({ nodeId, projectId: "p", ...extra });
  const mount = node("mount", { sourcePath: "D:/Shared" });
  const doc = row("knowledge:a", { kind: "knowledge", knowledge: { id: "a" }, explorerNode: node("a") }, true);
  const session = row("session:s", { kind: "session", session: { id: "s" }, explorerNode: node("s") });
  const local = row("local-file:b", { kind: "localFile", explorerNode: node("b", { sourcePath: "F:/Game/b.txt" }) });
  const mounted = row("mounted:c", { kind: "mountedFile", explorerNode: mount, mountEntry: { relativePath: "c.txt" } });
  const folder = row("folder:f", { kind: "folder", explorerNode: node("f") });
  const other = row("knowledge:d", { kind: "knowledge", knowledge: { id: "d" }, explorerNode: node("d"), projectId: "q" });
  const treeItems = ref<Item[]>([doc, session, local, folder, mounted, other]);
  const selectedFileKeys = ref(new Set<string>());
  const start = vi.fn();
  const deps = {
    treeItems, selectedFileKeys, lastFileSelectionAnchorKey: ref<string | null>(null), resolveWorkspaceSessionSelection,
    settlingLayoutDrop: ref(null), selectedSessionIds: ref(new Set<string>()), archivedTreeItems: ref([]),
    secondaryNavigation: ref(null), secondarySessionItems: ref([]), isKnowledgeTypeFolder: () => false,
    treeEditorDescriptor: () => null, topBarAcceptsTreeItem: () => false, internalDrag: { start }, itemIcon: () => undefined, itemIconClass: () => undefined,
    fileOrderDragItem: () => null, workspaceTreeItemState, explorerStore: { snapshots: { p: { nodes: [], itemStates: [] } } },
    WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE: "layout", contextMenu: ref(null), displayMenu: ref(null), workspaceMenu: ref(null),
  };
  const runtime = new Function(...Object.keys(deps), `${code}\nreturn { resolveFileClickSelection, onDragPointerDown };`)(
    ...Object.values(deps),
  ) as {
    resolveFileClickSelection: (item: Item, event?: Partial<MouseEvent>) => boolean;
    onDragPointerDown: (item: Item, event: unknown) => void;
  };
  const click = (item: Item, modifiers: Partial<MouseEvent> = {}) => runtime.resolveFileClickSelection(item, modifiers);
  return { doc, session, local, mounted, folder, other, selectedFileKeys, start, click, runtime };
}

describe("document and file multi-selection in the workspace tree", () => {
  it("toggles with Ctrl, extends with Shift across document and file rows, and opens on a plain click", () => {
    const f = fixture();
    // Ctrl keeps the open document and adds the clicked file without opening it.
    expect(f.click(f.local, { ctrlKey: true })).toBe(false);
    expect([...f.selectedFileKeys.value]).toEqual(["knowledge:a", "local-file:b"]);
    // Shift ranges over document/file rows only; sessions and folders in between are skipped.
    expect(f.click(f.mounted, { shiftKey: true })).toBe(false);
    expect([...f.selectedFileKeys.value]).toEqual(["local-file:b", "mounted:c"]);
    expect(f.click(f.mounted, { metaKey: true })).toBe(false);
    expect([...f.selectedFileKeys.value]).toEqual(["local-file:b"]);
    expect(f.click(f.doc)).toBe(true);
    expect(f.selectedFileKeys.value.size).toBe(0);
  });

  it("keeps one selection per project", () => {
    const f = fixture();
    f.click(f.local, { ctrlKey: true });
    f.click(f.other, { ctrlKey: true });
    expect([...f.selectedFileKeys.value]).toEqual(["knowledge:d"]);
  });

  it("drags every selected row when the pressed row is part of the selection", () => {
    const f = fixture();
    f.click(f.local, { ctrlKey: true });
    f.runtime.onDragPointerDown(f.doc, { currentTarget: null });
    const options = f.start.mock.calls[0]![1];
    expect(options.payload.data.items.map((item: Item) => item.key)).toEqual(["knowledge:a", "local-file:b"]);
    expect(options.preview.count).toBe(2);
    expect(options.allowedOperations).toEqual(["move", "copy"]);

    // An unpinned mounted entry cannot move in the layout, so the whole selection is copy-only.
    f.click(f.mounted, { ctrlKey: true });
    f.runtime.onDragPointerDown(f.local, { currentTarget: null });
    expect(f.start.mock.calls[1]![1].allowedOperations).toEqual(["copy"]);

    // A row outside the selection drags alone.
    f.runtime.onDragPointerDown(f.folder, { currentTarget: null });
    expect(f.start.mock.calls[2]![1].payload.data.items.map((item: Item) => item.key)).toEqual(["folder:f"]);
  });
});
