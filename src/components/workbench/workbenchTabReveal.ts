import type { WorkbenchEditorInput } from "../../types/workbench";
import { isExternalWorkspaceFilePath } from "./workbenchExternalFile";

/** Tabs whose resource has a row in the workspace tree or one of its side lists. */
export function canRevealEditorInWorkspace(editor: WorkbenchEditorInput): boolean {
  const resource = editor.resource;
  switch (resource.kind) {
    case "session":
    case "knowledge":
    case "localFile":
      return true;
    case "asset":
    case "workspaceFile":
      return !isExternalWorkspaceFilePath(resource.path);
    default:
      return false;
  }
}

/** File tabs that the system file manager can select. */
export function canOpenEditorInFileExplorer(editor: WorkbenchEditorInput): boolean {
  switch (editor.resource.kind) {
    case "asset":
    case "workspaceFile":
    case "knowledge":
      return true;
    case "localFile":
      return !!editor.sourcePath;
    default:
      return false;
  }
}
