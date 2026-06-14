import { ref } from "vue";
import {
  unityHotReloadPreflight,
  unityHotReloadSetCodeOptimizationDebug,
} from "../services/csharpLsp";
import { normalizeAppError } from "../services/errors";

/**
 * Enable-time gate shared by both hot-reload toggles (the icon above the chat
 * input and the Settings switch). Hot patches only take effect when the Unity
 * editor's Code Optimization is Debug — Release inlines call sites past the
 * MonoMod redirect, so patches silently do nothing.
 *
 * Before turning the feature on we probe the connected editor. On a positive
 * "release" we surface a modal; confirming switches the editor to Debug and
 * then runs the caller's `enable` step. When the editor is unreachable or the
 * value can't be read, we enable directly — the execution-time probe still
 * guards real hot reloads, so there is nothing to block on yet.
 *
 * `enable` is the caller's own "turn it on" routine (it owns the
 * `unityHotReloadSetEnabled(true)` call and its component's status/error
 * state), so each call site keeps its existing behaviour and only the gate is
 * shared.
 */
export function useHotReloadDebugGuard(enable: () => Promise<void>) {
  const promptVisible = ref(false);
  const adjusting = ref(false);
  const adjustError = ref("");

  async function guardedEnable() {
    let codeOptimization: string | null = null;
    try {
      codeOptimization = (await unityHotReloadPreflight()).codeOptimization;
    } catch {
      // Can't tell (editor down, command failed) → don't block; the
      // execution-time probe gates the actual hot reload.
      codeOptimization = null;
    }
    if (codeOptimization === "release") {
      adjustError.value = "";
      promptVisible.value = true;
      return;
    }
    await enable();
  }

  async function confirmAdjust() {
    if (adjusting.value) return;
    adjusting.value = true;
    adjustError.value = "";
    try {
      await unityHotReloadSetCodeOptimizationDebug();
      promptVisible.value = false;
      await enable();
    } catch (error) {
      adjustError.value = normalizeAppError(error).message;
    } finally {
      adjusting.value = false;
    }
  }

  function cancelAdjust() {
    if (adjusting.value) return;
    promptVisible.value = false;
    adjustError.value = "";
  }

  return {
    promptVisible,
    adjusting,
    adjustError,
    guardedEnable,
    confirmAdjust,
    cancelAdjust,
  };
}
