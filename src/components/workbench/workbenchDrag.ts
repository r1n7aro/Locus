import type { WorkbenchEditorDragData } from "../../types/workbench";

export const WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE = "locus/workbench-editor-tab";

export interface WorkbenchEditorTabInternalDragData extends WorkbenchEditorDragData {
  title: string;
}
