// Feedback shared by the settings pages and onboarding: the transient success
// banner, the last validation error, and a signal for provider sign-in
// changes that hosts answer by reloading auth and model state.
import { ref } from "vue";
import { defineStore } from "pinia";

export const useSettingsFeedbackStore = defineStore("settingsFeedback", () => {
  const success = ref("");
  const error = ref("");
  /** Increments whenever a provider credential is added, changed or removed. */
  const authRevision = ref(0);
  let successTimer: ReturnType<typeof setTimeout> | null = null;

  function flashSuccess(message: string, durationMs = 2000): void {
    success.value = message;
    if (successTimer) clearTimeout(successTimer);
    successTimer = setTimeout(() => {
      success.value = "";
      successTimer = null;
    }, durationMs);
  }

  function clearSuccess(): void {
    if (successTimer) clearTimeout(successTimer);
    successTimer = null;
    success.value = "";
  }

  function setError(message: string): void {
    error.value = message;
  }

  function clearError(): void {
    error.value = "";
  }

  function authChanged(): void {
    authRevision.value += 1;
  }

  function reset(): void {
    clearSuccess();
    clearError();
  }

  return {
    success,
    error,
    authRevision,
    flashSuccess,
    clearSuccess,
    setError,
    clearError,
    authChanged,
    reset,
  };
});
