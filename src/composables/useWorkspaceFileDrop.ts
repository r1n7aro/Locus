import { onBeforeUnmount, ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useInternalDropTarget } from "./useInternalDrag";
import { importWorkspaceEntries, moveWorkspaceFile } from "../services/workspaceExplorer";
import { useNotificationStore } from "../stores/notification";
import { normalizeAppError } from "../services/errors";
import { t } from "../i18n";
import { WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE, type WorkbenchReferenceDragData } from "../components/workbench/workbenchReferenceDrag";
import { resolveWorkspaceFileMove, workspaceRelativeEntryPath, type WorkspaceFileDropScope } from "../components/explorer/workspaceFileMove";
import "../styles/workspace-file-drop.css";

interface FileDropTarget extends WorkspaceFileDropScope { element: HTMLElement }

const DROP_TARGET_CLASS = "is-workspace-file-drop-target";
const EXPAND_DELAY_MS = 650;
const CONFLICT_LIST_LIMIT = 10;

/** Highlights the folder under the pointer and opens it after a short hover. */
function dropFeedback(expand?: (target: FileDropTarget) => void) {
  let highlighted: HTMLElement | null = null;
  let key = "";
  let expandTimer: ReturnType<typeof setTimeout> | undefined;
  function show(target: FileDropTarget | null): void {
    const nextKey = target ? `${target.workspaceRef.checkoutId}:${target.directory}` : "";
    // Native drags report every pointer move; keep the hover timer for the same folder.
    if ((target?.element ?? null) === highlighted && nextKey === key) return;
    highlighted?.classList.remove(DROP_TARGET_CLASS);
    clearTimeout(expandTimer);
    highlighted = target?.element ?? null;
    key = nextKey;
    highlighted?.classList.add(DROP_TARGET_CLASS);
    if (target && expand) expandTimer = setTimeout(() => expand(target), EXPAND_DELAY_MS);
  }
  return { show };
}

interface NativeFileDropRegistration {
  root: () => HTMLElement | null;
  resolve: (hit: Element) => FileDropTarget | null;
  busy: () => boolean;
  feedback: ReturnType<typeof dropFeedback>;
  copy: (target: FileDropTarget, paths: string[]) => Promise<void>;
}

const nativeRegistrations = new Set<NativeFileDropRegistration>();

function elementDepth(element: Element): number {
  let depth = 0;
  for (let current = element.parentElement; current; current = current.parentElement) depth += 1;
  return depth;
}

/** Files folder under `hit`, preferring the innermost list like internal drags do. */
function nativeFileDropAt(hit: Element | null): { registration: NativeFileDropRegistration; target: FileDropTarget } | null {
  if (hit?.nodeType !== 1) return null;
  const candidates = [...nativeRegistrations]
    .flatMap((registration) => {
      const root = registration.root();
      return root && root.contains(hit) && !registration.busy() ? [{ registration, root }] : [];
    })
    .sort((left, right) => elementDepth(right.root) - elementDepth(left.root));
  for (const { registration } of candidates) {
    const target = registration.resolve(hit);
    if (target) return { registration, target };
  }
  return null;
}

/**
 * Files dragged in from the operating system arrive as native events with a
 * position. This resolves them against the same Files folders as internal
 * moves and copies the dropped paths into the folder.
 */
export function createNativeWorkspaceFileDrop() {
  let current: NativeFileDropRegistration | null = null;
  function clear(): void {
    current?.feedback.show(null);
    current = null;
  }
  return {
    /** Highlights the Files folder under `hit`. Returns whether one accepts the files. */
    update(hit: Element | null): boolean {
      const next = nativeFileDropAt(hit);
      if (current && current !== next?.registration) current.feedback.show(null);
      current = next?.registration ?? null;
      next?.registration.feedback.show(next.target);
      return !!next;
    },
    clear,
    /** Copies `paths` into the Files folder under `hit`. Returns false when there is none. */
    async drop(hit: Element | null, paths: string[]): Promise<boolean> {
      const next = nativeFileDropAt(hit);
      clear();
      if (!next) return false;
      await next.registration.copy(next.target, paths);
      return true;
    },
  };
}

function conflictMessage(conflicts: string[], directory: string): string {
  const prefix = directory && directory !== "." ? `${directory}/` : "";
  const names = conflicts.map(path => path.startsWith(prefix) ? path.slice(prefix.length) : path);
  if (names.length === 1) return t("workspace.fileImport.replaceOne", names[0]!);
  const listed = names.slice(0, CONFLICT_LIST_LIMIT);
  if (names.length > listed.length) listed.push("...");
  return t("workspace.fileImport.replaceMany", names.length, listed.join("\n"));
}

export function useWorkspaceFileDrop(options: {
  root: () => HTMLElement | null;
  resolve: (hit: Element) => FileDropTarget | null;
  expand?: (target: FileDropTarget) => void;
}) {
  const notifications = useNotificationStore();
  const busy = ref(false);
  const feedback = dropFeedback(options.expand);
  const nativeFeedback = dropFeedback(options.expand);

  async function copy(target: FileDropTarget, paths: string[]): Promise<void> {
    const directory = workspaceRelativeEntryPath(target.root, target.directory);
    if (busy.value || directory === null || paths.length === 0) return;
    const workspaceRef = { ...target.workspaceRef };
    const destination = directory || ".";
    busy.value = true;
    try {
      const result = await importWorkspaceEntries(workspaceRef, paths, destination);
      if (result.conflicts.length === 0) return;
      const approved = await confirm(conflictMessage(result.conflicts, directory), {
        title: t("workspace.fileImport.replaceTitle"), kind: "warning",
        okLabel: t("workspace.fileImport.replace"), cancelLabel: t("common.cancel"),
      });
      if (approved) await importWorkspaceEntries(workspaceRef, paths, destination, true);
    } catch (error) {
      notifications.addNotice("error", normalizeAppError(error).message);
    } finally {
      busy.value = false;
    }
  }

  const registration: NativeFileDropRegistration = {
    root: options.root, resolve: options.resolve, busy: () => busy.value, feedback: nativeFeedback, copy,
  };
  nativeRegistrations.add(registration);
  onBeforeUnmount(() => {
    nativeRegistrations.delete(registration);
    feedback.show(null);
    nativeFeedback.show(null);
  });
  useInternalDropTarget<WorkbenchReferenceDragData, { target: FileDropTarget; move: NonNullable<ReturnType<typeof resolveWorkspaceFileMove>> }>({
    id: `workspace-file-move:${crypto.randomUUID()}`,
    root: options.root,
    priority: 30,
    accepts: source => !busy.value && source.payload.type === WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE
      && (source.payload.data as WorkbenchReferenceDragData).fileSystemMove === true,
    resolve: ({ hit, source }) => {
      const target = options.resolve(hit);
      const move = target && resolveWorkspaceFileMove(source.payload.data, target);
      return target && move ? { key: `${target.workspaceRef.checkoutId}:${move.targetDirectory}`, operation: "move",
        intent: { target, move } } : null;
    },
    onTargetChange: decision => feedback.show(decision?.intent.target ?? null),
    drop: async ({ decision }) => {
      busy.value = true;
      try {
        const { workspaceRef, sourcePaths, targetDirectory } = decision.intent.move;
        // Move sequentially so one failed entry does not cancel the rest of a multi-selection.
        for (const sourcePath of sourcePaths) {
          try { await moveWorkspaceFile(workspaceRef, sourcePath, targetDirectory); }
          catch (error) { notifications.addNotice("error", normalizeAppError(error).message); }
        }
      } finally { busy.value = false; }
    },
  });
}
