import { reactive } from "vue";
import type { KnowledgeAccessMode } from "../types";

const STORAGE_KEY = "locus-knowledge-access-mode";

const defaults = {
  mode: "full" as KnowledgeAccessMode,
};

function normalizeMode(value: unknown): KnowledgeAccessMode {
  if (value === "disabled" || value === "read_only") return value;
  return "full";
}

function load(): { mode: KnowledgeAccessMode } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as { mode?: unknown };
      return { mode: normalizeMode(parsed.mode) };
    }
  } catch {
    // ignore persistence failures
  }
  return { ...defaults };
}

function save(mode: KnowledgeAccessMode) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ mode }));
  } catch {
    // ignore persistence failures
  }
}

const state = reactive(load());

export function useKnowledgeAccessMode() {
  function setMode(mode: KnowledgeAccessMode) {
    state.mode = normalizeMode(mode);
    save(state.mode);
  }

  return {
    state,
    setMode,
  };
}
