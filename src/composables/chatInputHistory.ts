export type ChatInputHistoryDirection = "previous" | "next";

export interface ChatInputHistoryState<T extends object> {
  index: number | null;
  preservedValue: T | null;
}

export interface ChatInputHistoryStep<T extends object> {
  state: ChatInputHistoryState<T>;
  value: T;
}

export interface ChatInputHistoryKeyEvent {
  key: string;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  isComposing?: boolean;
}

export interface ChatInputHistorySelection {
  value: string;
  selectionStart: number | null;
  selectionEnd: number | null;
  isNavigating: boolean;
}

export function createChatInputHistoryState<T extends object>(): ChatInputHistoryState<T> {
  return {
    index: null,
    preservedValue: null,
  };
}

export function navigateChatInputHistory<T extends object>(
  history: readonly T[],
  state: ChatInputHistoryState<T>,
  direction: ChatInputHistoryDirection,
  currentValue: T,
): ChatInputHistoryStep<T> | null {
  if (history.length === 0) return null;

  if (direction === "previous") {
    const index = state.index == null
      ? history.length - 1
      : Math.max(0, Math.min(state.index - 1, history.length - 1));
    return {
      state: {
        index,
        preservedValue: state.index == null ? currentValue : state.preservedValue,
      },
      value: history[index]!,
    };
  }

  if (state.index == null) return null;

  if (state.index < history.length - 1) {
    const index = state.index + 1;
    return {
      state: {
        index,
        preservedValue: state.preservedValue,
      },
      value: history[index]!,
    };
  }

  return {
    state: createChatInputHistoryState<T>(),
    value: state.preservedValue ?? currentValue,
  };
}

export function shouldNavigateChatInputHistory(
  event: ChatInputHistoryKeyEvent,
  selection: ChatInputHistorySelection,
): boolean {
  if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return false;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.isComposing) return false;

  if (event.key === "ArrowDown" && !selection.isNavigating) return false;
  if (selection.selectionStart == null || selection.selectionEnd == null) return false;
  if (selection.selectionStart !== selection.selectionEnd) return false;

  const firstLineEnd = selection.value.indexOf("\n");
  if (firstLineEnd < 0) return true;
  if (event.key === "ArrowUp") return selection.selectionStart <= firstLineEnd;

  const lastLineStart = selection.value.lastIndexOf("\n") + 1;
  return selection.selectionStart >= lastLineStart;
}
