import type { InjectionKey } from "vue";
import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { WorkspaceRef } from "./project";
import type { ToolFilePreviewWindowPayload } from "./toolFilePreviewWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import { isWorkbenchWindowLabel } from "./workbenchWindow";

export const WORKBENCH_FILE_OPEN_EVENT = "workbench-file-open";

export interface WorkbenchFileOpenRequest extends ToolFilePreviewWindowPayload {
  mode?: "file" | "history";
  workspaceRef: WorkspaceRef;
  targetLabel?: string;
}

// Resolve the containing Workbench directly, including shared floating windows.
export const WORKBENCH_FILE_OPEN_KEY: InjectionKey<
  (request: WorkbenchFileOpenRequest) => Promise<void>
> = Symbol("workbench-file-open");

/** Markdown files open in the Workbench document editor instead of the OS default app. */
export function opensInWorkbenchOnClick(filePath: string): boolean {
  return /\.(?:md|markdown)$/i.test(filePath.trim());
}

export async function openWorkbenchFileTab(request: WorkbenchFileOpenRequest): Promise<void> {
  if (!hasTauriWindowRuntime()) throw new Error("Workbench window is unavailable.");
  const currentLabel = getCurrentWindow().label;
  const targetLabel = isWorkbenchWindowLabel(currentLabel) ? currentLabel : "main";
  await emitTo<WorkbenchFileOpenRequest>(targetLabel, WORKBENCH_FILE_OPEN_EVENT, {
    ...request,
    targetLabel,
  });
}
