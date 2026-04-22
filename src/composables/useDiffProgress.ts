import { ref, computed, onUnmounted } from "vue";
import { listenDiffProgress, type DiffProgressEvent } from "../services/diff";
import { t } from "../i18n";

const phaseKeys: Record<DiffProgressEvent["phase"], string> = {
  fetchContent: "diff.progress.fetchContent",
  textDiff: "diff.progress.textDiff",
  parseYaml: "diff.progress.parseYaml",
  buildSemantic: "diff.progress.buildSemantic",
  done: "",
  error: "",
};

export function useDiffProgress() {
  const phase = ref<DiffProgressEvent["phase"] | null>(null);
  const current = ref(0);
  const total = ref(1);
  const elapsedMs = ref(0);
  const error = ref<string | null>(null);
  const active = ref(false);
  const phaseDurations = ref<Record<string, number> | null>(null);

  const phaseLabel = computed(() => {
    if (!phase.value || phase.value === "done" || phase.value === "error") {
      return t("chat.changes.loading");
    }
    const key = phaseKeys[phase.value];
    return key ? t(key) : t("chat.changes.loading");
  });

  const progress = computed(() => {
    if (!active.value || total.value === 0) return 0;
    if (phase.value === "done") return 1;
    // Show progress as (current + 0.5) / total so bar moves during each phase
    return Math.min((current.value + 0.5) / total.value, 0.99);
  });

  let unlisten: (() => void) | null = null;

  listenDiffProgress((evt) => {
    if (!active.value) return;
    phase.value = evt.phase;
    current.value = evt.current;
    total.value = evt.total;
    elapsedMs.value = evt.elapsedMs;
    error.value = evt.error ?? null;
    if (evt.phase === "done" || evt.phase === "error") {
      active.value = false;
      if (evt.phaseDurations) {
        phaseDurations.value = evt.phaseDurations;
        const parts = Object.entries(evt.phaseDurations)
          .map(([k, v]) => `${k}=${v}ms`)
          .join(", ");
        console.log(
          `[diff-perf] ${evt.requestKey} total=${evt.elapsedMs}ms [${parts}]`,
        );
      }
    }
  }).then((fn) => {
    unlisten = fn;
  });

  onUnmounted(() => {
    unlisten?.();
  });

  function reset() {
    phase.value = null;
    current.value = 0;
    total.value = 1;
    elapsedMs.value = 0;
    error.value = null;
    active.value = true;
    phaseDurations.value = null;
  }

  return {
    phase,
    phaseLabel,
    progress,
    error,
    active,
    phaseDurations,
    reset,
  };
}
