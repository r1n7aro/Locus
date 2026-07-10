import { ipcInvoke } from "./ipc";
import type { ModelDefaults, CustomEndpoint, CodexModelConfig, ModelOption } from "../types";

export function getModelDefaults(): Promise<ModelDefaults> {
  return ipcInvoke<ModelDefaults>("get_model_defaults");
}

export function saveModelDefaults(defaults: ModelDefaults): Promise<void> {
  return ipcInvoke("save_model_defaults", { defaults });
}

export function getCodexModelConfig(): Promise<CodexModelConfig> {
  return ipcInvoke<CodexModelConfig>("get_codex_model_config");
}

export function getCodexAvailableModels(): Promise<ModelOption[]> {
  return ipcInvoke<ModelOption[]>("get_codex_available_models");
}

export function saveCodexModelConfig(config: CodexModelConfig): Promise<void> {
  return ipcInvoke("save_codex_model_config", { config });
}

export function getLastModel(): Promise<string> {
  return ipcInvoke<string>("get_last_model");
}

export function saveLastModel(modelId: string): Promise<void> {
  return ipcInvoke("save_last_model", { modelId });
}

export function getLastEffort(): Promise<string> {
  return ipcInvoke<string>("get_last_effort");
}

export function saveLastEffort(effort: string): Promise<void> {
  return ipcInvoke("save_last_effort", { effort });
}

export function getCodexFastMode(): Promise<boolean> {
  return ipcInvoke<boolean>("get_codex_fast_mode");
}

export function saveCodexFastMode(enabled: boolean): Promise<void> {
  return ipcInvoke("save_codex_fast_mode", { enabled });
}

export function getCustomEndpoints(): Promise<CustomEndpoint[]> {
  return ipcInvoke<CustomEndpoint[]>("get_custom_endpoints");
}

export function saveCustomEndpoints(endpoints: CustomEndpoint[]): Promise<void> {
  return ipcInvoke("save_custom_endpoints", { endpoints });
}

export function testCustomEndpoint(endpoint: CustomEndpoint): Promise<string> {
  return ipcInvoke<string>("test_custom_endpoint", { endpoint });
}
