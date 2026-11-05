import { reactive, watch } from "vue";
import { saveSetting, settingValue } from "./useSettingsStore";
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

const defaults: ChatInputSettings = {
  submitMode: "enter-send",
  runningSendMode: "insert",
};

function normalizeSubmitMode(value: unknown): ChatSubmitMode {
  return value === "mod-enter-send" ? "mod-enter-send" : "enter-send";
}

function normalizeRunningSendMode(value: unknown): RunningSendMode {
  return value === "after-run" ? "after-run" : defaults.runningSendMode;
}

const SUBMIT_MODE_KEY = "chat.input.submitMode";
const RUNNING_SEND_MODE_KEY = "chat.input.runningSendMode";

function project(): ChatInputSettings {
  return {
    submitMode: normalizeSubmitMode(settingValue(SUBMIT_MODE_KEY)),
    runningSendMode: normalizeRunningSendMode(settingValue(RUNNING_SEND_MODE_KEY)),
  };
}

const state = reactive<ChatInputSettings>(project());
watch(project, (next) => {
  if (state.submitMode !== next.submitMode) state.submitMode = next.submitMode;
  if (state.runningSendMode !== next.runningSendMode) state.runningSendMode = next.runningSendMode;
}, { flush: "sync" });

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
    saveSetting(SUBMIT_MODE_KEY, normalizeSubmitMode(mode));
  }

  function setRunningSendMode(mode: RunningSendMode) {
    saveSetting(RUNNING_SEND_MODE_KEY, normalizeRunningSendMode(mode));
  }

  return {
    state,
    setSubmitMode,
    setRunningSendMode,
  };
}
