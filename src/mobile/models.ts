import { getCustomProviderCatalog } from "../services/model";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";

let current: Promise<void> | null = null;

/** What the desktop bootstrap loads for the model selector. Custom providers
 * come from their key-free catalog: a device cannot read the API keys. */
async function fetchModelCatalog(): Promise<void> {
  const modelStore = useModelStore();
  const authStore = useAuthStore();
  await Promise.all([
    modelStore.loadDebugMode(),
    modelStore.loadModelDefaults(),
    modelStore.loadLastModel(),
    modelStore.loadAgentModelPreferences(),
    modelStore.loadCodexFastMode(),
    modelStore.loadCodexModelConfig(),
    getCustomProviderCatalog().then(
      (providers) => modelStore.applyCustomProviders(providers),
      (error: unknown) => console.warn("[remote] get_custom_provider_catalog:", error),
    ),
    authStore.checkAuth(),
  ]);
  await modelStore.loadCodexAvailableModels();
  modelStore.resolveSelectedModel(true);
  await modelStore.loadLastEffort();
}

/** Loads the model catalog once; `refresh` reloads it (after connecting or a
 * resync, since loads made while offline leave it empty). */
export function loadModelCatalog(options: { refresh?: boolean } = {}): Promise<void> {
  if (current && !options.refresh) return current;
  const request = fetchModelCatalog();
  current = request;
  request.catch((error: unknown) => {
    console.warn("[remote] model catalog failed to load", error);
    if (current === request) current = null;
  });
  return request;
}
