import { ipcInvoke } from "./ipc";
import type {
  ModelDefaults,
  CustomEndpoint,
  CustomProvider,
  CodexModelConfig,
  ModelCatalogResponse,
  ModelOption,
} from "../types";

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

export function getCustomProviders(): Promise<CustomProvider[]> {
  return ipcInvoke<CustomProvider[]>("get_custom_providers");
}

export function saveCustomProviders(providers: CustomProvider[]): Promise<void> {
  return ipcInvoke("save_custom_providers", { providers });
}

export function testCustomEndpoint(endpoint: CustomEndpoint): Promise<string> {
  return ipcInvoke<string>("test_custom_endpoint", { endpoint });
}

export function getModelCatalog(): Promise<ModelCatalogResponse> {
  return ipcInvoke<ModelCatalogResponse>("get_model_catalog");
}

export function refreshModelCatalog(): Promise<ModelCatalogResponse> {
  return ipcInvoke<ModelCatalogResponse>("refresh_model_catalog");
}
