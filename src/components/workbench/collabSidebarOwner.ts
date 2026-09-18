import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../../types/workbench";

export function isCollabEditor(editor: WorkbenchEditorInput): boolean {
  return editor.resource.kind === "collaboration" || editor.resource.kind === "checkout"
    || (editor.resource.kind === "section" && editor.resource.section === "collab");
}

/** Follow the owning editor when its tab moves; focus alone must not retarget Git actions. */
export function findCollabSidebarOwner(
  groups: Record<string, WorkbenchEditorGroup>,
  target: { editorId?: string; checkoutId: string },
): { paneId: string; editor: WorkbenchEditorInput } | null {
  for (const group of Object.values(groups)) {
    const editor = group.tabs.find(candidate => candidate.editorId === target.editorId);
    if (editor && isCollabEditor(editor) && editor.availability !== "unavailable"
      && editor.checkoutBinding?.checkoutId === target.checkoutId) {
      return { paneId: group.paneId, editor };
    }
  }
  return null;
}
