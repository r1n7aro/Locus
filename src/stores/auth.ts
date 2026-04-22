import { ref } from "vue";
import { defineStore } from "pinia";
import * as authService from "../services/auth";
import { filterVisibleProviders } from "../config/providerVisibility";
import { normalizeAppError } from "../services/errors";
import type { AppErrorPayload } from "../types";

export interface AuthStatusLoadFailure {
  target: "providers" | "codex";
  error: AppErrorPayload;
}

export const useAuthStore = defineStore("auth", () => {
  const isAuthenticated = ref(false);
  const hasApiKey = ref(false);
  const anthropicSdkAvailable = ref(false);
  const codexAuthenticated = ref(false);
  const authChecked = ref(false);

  /** Lightweight auth check — only getAuthStatus, no provider details. */
  async function checkAuthLight(markChecked = true) {
    try {
      const status = await authService.getAuthStatus();
      console.log("[Auth] checkAuthLight result:", JSON.stringify(status));
      isAuthenticated.value = status.authenticated;
      hasApiKey.value = status.hasApiKey;
    } catch (e) {
      console.error("get_auth_status failed:", e);
      isAuthenticated.value = false;
    }
    if (markChecked) {
      authChecked.value = true;
    }
  }

  /** Full auth check including provider details and codex status. */
  async function checkAuth(): Promise<AuthStatusLoadFailure[]> {
    authChecked.value = false;
    try {
      await checkAuthLight(false);
      return await loadProviderStatus();
    } finally {
      authChecked.value = true;
    }
  }

  async function loadProviderStatus(): Promise<AuthStatusLoadFailure[]> {
    const failures: AuthStatusLoadFailure[] = [];
    try {
      const providers = filterVisibleProviders(await authService.getProviders());
      const or = providers.find((p) => p.id === "openrouter");
      hasApiKey.value = !!or?.hasKey;
      const an = providers.find((p) => p.id === "anthropic");
      isAuthenticated.value = !!an?.hasKey;
      const sdk = providers.find((p) => p.id === "anthropic_sdk");
      anthropicSdkAvailable.value = !!sdk?.hasKey;
    } catch (e) {
      console.error("get_providers failed:", e);
      failures.push({
        target: "providers",
        error: normalizeAppError(e),
      });
    }
    try {
      const cs = await authService.codexStatus();
      console.log("[Auth] codexStatus:", JSON.stringify(cs));
      codexAuthenticated.value = cs.authenticated;
    } catch (e) {
      console.error("[Auth] codexStatus failed:", e);
      failures.push({
        target: "codex",
        error: normalizeAppError(e),
      });
    }
    return failures;
  }

  return {
    isAuthenticated,
    hasApiKey,
    anthropicSdkAvailable,
    codexAuthenticated,
    authChecked,
    checkAuthLight,
    checkAuth,
    loadProviderStatus,
  };
});
