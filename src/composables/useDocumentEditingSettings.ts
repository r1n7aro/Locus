import { reactive, readonly, watch } from "vue";
import { saveSetting, settingValue } from "./useSettingsStore";

const PUNCTUATION_PAIRING_KEY = "documents.punctuationPairing";
const COMMENT_AUTHOR_KEY = "documents.commentAuthor";
const MAX_COMMENT_AUTHOR_LENGTH = 64;

export function normalizeCommentAuthor(value: unknown): string {
  return typeof value === "string" ? value.replace(/\s+/g, " ").trim().slice(0, MAX_COMMENT_AUTHOR_LENGTH) : "";
}

export function normalizeDocumentEditingSettings(value: unknown) {
  const record = value && typeof value === "object" ? value as Record<string, unknown> : {};
  return {
    punctuationPairing: record.punctuationPairing === true,
    /** Name written on document comments. */
    commentAuthor: normalizeCommentAuthor(record.commentAuthor),
  };
}

function project() {
  return normalizeDocumentEditingSettings({
    punctuationPairing: settingValue(PUNCTUATION_PAIRING_KEY),
    commentAuthor: settingValue(COMMENT_AUTHOR_KEY),
  });
}

const state = reactive(project());

// Document and settings windows (and the agent) change it through settings.
watch(project, (next) => Object.assign(state, next), { flush: "sync" });

export function useDocumentEditingSettings() {
  return {
    state: readonly(state),
    setPunctuationPairing(enabled: boolean) {
      saveSetting(PUNCTUATION_PAIRING_KEY, enabled);
    },
    setCommentAuthor(name: string) {
      saveSetting(COMMENT_AUTHOR_KEY, normalizeCommentAuthor(name));
    },
  };
}
