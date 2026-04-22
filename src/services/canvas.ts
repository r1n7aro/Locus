import { ipcInvoke } from "./ipc";

export function canvasGetSpec(specId: string): Promise<string> {
  return ipcInvoke<string>("canvas_get_spec", { specId });
}

export function canvasSetSpec(specId: string, spec: string): Promise<void> {
  return ipcInvoke("canvas_set_spec", { specId, spec });
}

export function canvasUpdateField(
  projectPath: string,
  scenePath: string | null,
  update: Record<string, unknown>,
  value: unknown,
  valueType: string,
): Promise<unknown> {
  return ipcInvoke("canvas_update_field", { projectPath, scenePath, update, value, valueType });
}

export interface CanvasRefreshQuery {
  id: string;
  gameObjectPath: string;
  componentType: string;
  propertyPath: string;
}

export interface CanvasRefreshResultEntry {
  id: string;
  exists: boolean;
  value: unknown;
}

export interface CanvasRefreshResult {
  results: CanvasRefreshResultEntry[];
}

export function canvasRefresh(
  projectPath: string,
  scenePath: string | null,
  queries: CanvasRefreshQuery[],
): Promise<CanvasRefreshResult> {
  return ipcInvoke<CanvasRefreshResult>("canvas_refresh", { projectPath, scenePath, queries });
}

export function canvasSave(projectPath: string, name: string, data: string): Promise<void> {
  return ipcInvoke("canvas_save", { projectPath, name, data });
}

export function canvasLoad(projectPath: string, name: string): Promise<string> {
  return ipcInvoke<string>("canvas_load", { projectPath, name });
}

export function canvasList(projectPath: string): Promise<string[]> {
  return ipcInvoke<string[]>("canvas_list", { projectPath });
}

export function canvasDelete(projectPath: string, name: string): Promise<void> {
  return ipcInvoke("canvas_delete", { projectPath, name });
}

export function canvasExecuteUpdate(
  projectPath: string,
  scenePath: string | null,
  fieldId: string,
  value: unknown,
  oldValue: unknown,
  options?: Record<string, unknown>,
): Promise<unknown> {
  return ipcInvoke("canvas_execute_update", { projectPath, scenePath, fieldId, value, oldValue, options });
}
