import { onScopeDispose, ref } from "vue";

interface CopyFeedbackOptions {
  durationMs?: number;
}

const DEFAULT_DURATION_MS = 1500;

export function useCopyFeedback(options: CopyFeedbackOptions = {}) {
  const copied = ref(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function reset() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    copied.value = false;
  }

  function markCopied() {
    copied.value = true;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      copied.value = false;
      timer = null;
    }, options.durationMs ?? DEFAULT_DURATION_MS);
  }

  async function copyText(text: string): Promise<boolean> {
    if (!text) return false;
    try {
      await navigator.clipboard.writeText(text);
      markCopied();
      return true;
    } catch {
      return false;
    }
  }

  onScopeDispose(reset);

  return {
    copied,
    copyText,
    reset,
  };
}
