import { computed, ref } from "vue";
import {
  unityHotReloadPreflight,
  unityHotReloadSetCodeOptimization,
} from "../services/csharpLsp";
import { normalizeAppError } from "../services/errors";

/**
 * Release-first hot reload (shared by the icon above the chat input and the
 * Settings switch). Enabling NO LONGER blocks on the editor's Code
 * Optimization: hot reload works in Release, where Mono inlines only some
 * small methods — those converge via an automatic recompile instead of the
 * live detour (see the Rust coordinator / Unity LocusBridge.HotReload).
 *
 * We still read the connected editor's optimization so the UI can show an
 * OPTIONAL, dismissible "switch to Debug" hint (Debug avoids the occasional
 * convergence recompile), mirroring the reference plugin's suggestion rather
 * than the old hard gate.
 *
 * `enable` is the caller's own "turn it on" routine; it runs unconditionally.
 */
export function useHotReloadDebugGuard(enable: () => Promise<void>) {
  const codeOptimization = ref<string | null>(null);
  const switching = ref(false);
  const switchError = ref("");

  // Only a positively-read "release" shows the hint; unknown (editor down /
  // old plugin) stays quiet, exactly as the execution path does.
  const isRelease = computed(() => codeOptimization.value === "release");

  async function refreshOptimization() {
    try {
      codeOptimization.value = (await unityHotReloadPreflight()).codeOptimization;
    } catch {
      codeOptimization.value = null;
    }
  }

  async function enableHotReload() {
    await enable();
    // Refresh the hint in the background once it's on (non-blocking).
    void refreshOptimization();
  }

  /** Switch the connected editor to an explicit Code Optimization level.
   * Triggers a Unity recompile; on failure we re-read the real state. */
  async function setOptimization(level: "debug" | "release") {
    if (switching.value) return;
    switching.value = true;
    switchError.value = "";
    try {
      const result = await unityHotReloadSetCodeOptimization(level);
      codeOptimization.value = result.codeOptimization;
    } catch (error) {
      switchError.value = normalizeAppError(error).message;
      // The switch may have partially landed (e.g. a recompile interrupted the
      // probe) — re-read so the UI reflects the editor's actual level.
      void refreshOptimization();
    } finally {
      switching.value = false;
    }
  }

  // Back-compat: the Settings switch still offers a one-shot "switch to Debug".
  async function switchToDebug() {
    await setOptimization("debug");
  }

  return {
    codeOptimization,
    isRelease,
    switching,
    switchError,
    refreshOptimization,
    enableHotReload,
    setOptimization,
    switchToDebug,
  };
}
