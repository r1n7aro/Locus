import { ref } from "vue";

export type MarkdownEditorViewMode = "rendered" | "native";

const STORAGE_KEY = "locus:markdownEditorViewMode";

function loadMarkdownEditorViewMode(): MarkdownEditorViewMode {
  if (typeof localStorage === "undefined") return "rendered";
  try {
    return localStorage.getItem(STORAGE_KEY) === "native" ? "native" : "rendered";
  } catch {
    return "rendered";
  }
}

const sharedMarkdownEditorViewMode = ref<MarkdownEditorViewMode>(loadMarkdownEditorViewMode());

export function useMarkdownEditorViewMode() {
  function setMarkdownEditorViewMode(value: MarkdownEditorViewMode) {
    sharedMarkdownEditorViewMode.value = value;
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch {
      // ignore persistence failures
    }
  }

  return {
    markdownEditorViewMode: sharedMarkdownEditorViewMode,
    setMarkdownEditorViewMode,
  };
}
