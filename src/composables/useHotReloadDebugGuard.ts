import { computed, ref } from "vue";
import {
  unityHotReloadPreflight,
  unityHotReloadSetCodeOptimization,
  unityHotReloadSetPlayModeReload,
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

  // Manual "Reload Domain on entering Play Mode" toggle, read off the SAME
  // preflight probe as Code Optimization. null = unknown (editor down / old
  // plugin). Flipping it does NOT recompile — it just edits EditorSettings.
  const domainReloadOnPlay = ref<boolean | null>(null);
  const settingPlayModeReload = ref(false);
  const playModeReloadError = ref("");

  // Only a positively-read "release" shows the hint; unknown (editor down /
  // old plugin) stays quiet, exactly as the execution path does.
  const isRelease = computed(() => codeOptimization.value === "release");

  async function refreshOptimization() {
    try {
      const preflight = await unityHotReloadPreflight();
      codeOptimization.value = preflight.codeOptimization;
      domainReloadOnPlay.value = preflight.domainReloadOnPlay;
    } catch {
      codeOptimization.value = null;
      domainReloadOnPlay.value = null;
    }
  }

  /** Set whether entering Play Mode reloads the domain. On failure re-read the
   * real EditorSettings state (the editor stays authoritative). */
  async function setPlayModeReload(domainReload: boolean) {
    if (settingPlayModeReload.value) return;
    settingPlayModeReload.value = true;
    playModeReloadError.value = "";
    try {
      const result = await unityHotReloadSetPlayModeReload(domainReload);
      domainReloadOnPlay.value = result.domainReloadOnPlay;
    } catch (error) {
      playModeReloadError.value = normalizeAppError(error).message;
      void refreshOptimization();
    } finally {
      settingPlayModeReload.value = false;
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
    domainReloadOnPlay,
    settingPlayModeReload,
    playModeReloadError,
    setPlayModeReload,
  };
}
