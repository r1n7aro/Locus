import type { WorkbenchEditorInput } from "../../types/workbench";

/**
 * Editors rendered by `WorkbenchSessionEditor` (mirrors the slot condition in
 * `DevelopmentWorkbench.vue`). They are the only editors a group unmounts while
 * hidden: their transcript lives in the shared chat state and their composer,
 * selection and scroll position survive in the work-state snapshot.
 */
export function isWorkbenchSessionEditorInput(editor: WorkbenchEditorInput): boolean {
  if (editor.availability === "unavailable") return false;
  const resource = editor.resource;
  return resource.kind === "session"
    || resource.kind === "newSession"
    || (resource.kind === "section" && resource.section === "archived" && !!resource.sessionId);
}

export interface EditorGroupMountInput {
  tabs: readonly WorkbenchEditorInput[];
  activeEditorId: string | null;
  /** The surface still on screen while the active editor loads. */
  presentedEditorId: string | null;
  /** Previously active editors of this group, most recent first. */
  recentEditorIds: readonly string[];
  /**
   * Editors mounted before this selection; null or undefined for the first one.
   * Recent editors only keep their place: an unmounted tab mounts when it is
   * activated, never in the background when a slot frees up.
   */
  mountedEditorIds?: ReadonlySet<string> | null;
  /** Maximum mounted evictable editors in this group; zero or less never unloads them. */
  limit: number;
  isEvictable: (editor: WorkbenchEditorInput) => boolean;
  /** Evictable editors that must stay mounted (running or waiting sessions). */
  retainedEditorIds: ReadonlySet<string>;
}

/**
 * Editor ids one editor group keeps mounted. Non-evictable editors always stay.
 * Evictable editors mount lazily, when first activated, so restoring many tabs
 * loads only the visible ones. They stay mounted in this order: the active and
 * presented editors, the retained ones (which may exceed the limit), then the
 * most recently active among those already mounted, up to `limit` (all of them
 * when the limit is zero or less).
 */
export function selectMountedEditorIds(input: EditorGroupMountInput): Set<string> {
  const limit = input.limit > 0 ? input.limit : Number.POSITIVE_INFINITY;
  const mounted = new Set<string>();
  const evictable = new Set<string>();
  for (const editor of input.tabs) {
    if (input.isEvictable(editor)) evictable.add(editor.editorId);
    else mounted.add(editor.editorId);
  }
  let count = 0;
  const keep = (editorId: string | null) => {
    if (!editorId || !evictable.has(editorId) || mounted.has(editorId)) return;
    mounted.add(editorId);
    count += 1;
  };
  keep(input.activeEditorId);
  keep(input.presentedEditorId);
  for (const editorId of evictable) {
    if (input.retainedEditorIds.has(editorId)) keep(editorId);
  }
  const previouslyMounted = input.mountedEditorIds ?? null;
  for (const editorId of input.recentEditorIds) {
    if (count >= limit) break;
    if (!previouslyMounted || previouslyMounted.has(editorId)) keep(editorId);
  }
  return mounted;
}
