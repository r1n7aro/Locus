import { reactive } from "vue";
import { detectShortcutPlatform, type ShortcutPlatform } from "./useKeyboardShortcuts";

export type ChatSubmitMode = "enter-send" | "mod-enter-send";
export type RunningSendMode = "after-run" | "insert";

export interface ChatInputSettings {
  submitMode: ChatSubmitMode;
  runningSendMode: RunningSendMode;
}

type EnterEventLike = Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "metaKey"> & {
  isComposing?: boolean;
};

const STORAGE_KEY = "locus-chat-input-settings";

const defaults: ChatInputSettings = {
  submitMode: "enter-send",
  runningSendMode: "after-run",
};

function normalizeSubmitMode(value: unknown): ChatSubmitMode {
  return value === "mod-enter-send" ? "mod-enter-send" : "enter-send";
}

function normalizeRunningSendMode(value: unknown): RunningSendMode {
  return value === "insert" ? "insert" : "after-run";
}

function load(): ChatInputSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<ChatInputSettings>;
      return {
        submitMode: normalizeSubmitMode(parsed.submitMode),
        runningSendMode: normalizeRunningSendMode(parsed.runningSendMode),
      };
    }
  } catch {
    // ignore persistence failures
  }
  return { ...defaults };
}

function save(settings: ChatInputSettings) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // ignore persistence failures
  }
}

const state = reactive<ChatInputSettings>(load());

function isEnterKey(event: EnterEventLike): boolean {
  return event.key === "Enter" && !event.isComposing;
}

export function getChatSubmitModifierLabel(
  platform: ShortcutPlatform = detectShortcutPlatform(),
): string {
  return platform === "mac" ? "Cmd" : "Ctrl";
}

export function shouldSubmitOnEnter(
  event: EnterEventLike,
  mode: ChatSubmitMode,
): boolean {
  if (!isEnterKey(event) || event.shiftKey) return false;
  const hasPrimaryModifier = event.ctrlKey || event.metaKey;
  return mode === "enter-send" ? !hasPrimaryModifier : hasPrimaryModifier;
}

export function shouldSelectPopupOnEnter(
  event: EnterEventLike,
  mode: ChatSubmitMode,
): boolean {
  if (mode !== "enter-send") return false;
  if (!isEnterKey(event) || event.shiftKey) return false;
  return !event.ctrlKey && !event.metaKey;
}

export function useChatInputSettings() {
  function setSubmitMode(mode: ChatSubmitMode) {
    state.submitMode = normalizeSubmitMode(mode);
    save({ ...state });
  }

  function setRunningSendMode(mode: RunningSendMode) {
    state.runningSendMode = normalizeRunningSendMode(mode);
    save({ ...state });
  }

  return {
    state,
    setSubmitMode,
    setRunningSendMode,
  };
}
