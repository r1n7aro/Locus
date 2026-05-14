import { reactive } from "vue";

export type FontSlot = "ui" | "prose" | "monoInline" | "monoBlock" | "monoEditor";
export type DiffReviewTarget = "inline" | "window";
export type ChatDiffReviewTarget = DiffReviewTarget;
export type GitDiffReviewTarget = DiffReviewTarget;

export interface DisplaySettings {
  /** Auto-open TODO panel when todos arrive */
  todoAutoOpen: boolean;
  /** Auto-open file changes panel when changes arrive */
  changesAutoOpen: boolean;
  /** Auto-close file changes panel when a new round starts */
  changesAutoClose: boolean;
  /** Default target for reviewing chat file diffs */
  chatDiffReviewTarget: DiffReviewTarget;
  /** Default target for reviewing Git file diffs */
  gitDiffReviewTarget: DiffReviewTarget;
  /** Right-align user messages in the session transcript */
  rightAlignUserMessages: boolean;
  /** Collapse completed tool call batches in chat transcript */
  compactToolCalls: boolean;
  /** Hide completed thinking blocks in chat transcript */
  hideThinkingBlocks: boolean;
  /** Merge Git tree status letters into colored file icons */
  mergeGitTreeStatusIcon: boolean;
  /** Hide Git command suggestions in Git terminal */
  hideGitCommandSuggestions: boolean;
  /** Enable desktop notifications when the app is not focused */
  systemNotificationsEnabled: boolean;
  /** Notify when a chat run completes */
  notifyOnChatDone: boolean;
  /** Notify when the agent asks the user a question */
  notifyOnAskUser: boolean;
  /** Notify when a chat run fails */
  notifyOnChatError: boolean;
  /** Notify when tool approval is required */
  notifyOnToolConfirm: boolean;
  /** Per-slot font-family overrides (empty string = use default) */
  fonts: Record<FontSlot, string>;
}

const STORAGE_KEY = "locus-display-settings";

const defaultFonts: Record<FontSlot, string> = {
  ui: "",
  prose: "",
  monoInline: "",
  monoBlock: "",
  monoEditor: "",
};

const defaults: DisplaySettings = {
  todoAutoOpen: true,
  changesAutoOpen: true,
  changesAutoClose: true,
  chatDiffReviewTarget: "inline",
  gitDiffReviewTarget: "inline",
  rightAlignUserMessages: true,
  compactToolCalls: true,
  hideThinkingBlocks: true,
  mergeGitTreeStatusIcon: true,
  hideGitCommandSuggestions: false,
  systemNotificationsEnabled: true,
  notifyOnChatDone: true,
  notifyOnAskUser: true,
  notifyOnChatError: true,
  notifyOnToolConfirm: true,
  fonts: { ...defaultFonts },
};

function load(): DisplaySettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return { ...defaults, ...parsed, fonts: { ...defaultFonts, ...parsed.fonts } };
    }
  } catch { /* ignore */ }
  return { ...defaults, fonts: { ...defaultFonts } };
}

function save(s: DisplaySettings) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(s)); } catch { /* ignore */ }
}

const state = reactive<DisplaySettings>(load());

export function useDisplaySettings() {
  function set<K extends keyof DisplaySettings>(key: K, value: DisplaySettings[K]) {
    state[key] = value;
    save({ ...state });
  }

  function setFont(slot: FontSlot, value: string) {
    state.fonts[slot] = value;
    save({ ...state, fonts: { ...state.fonts } });
    applyFonts(state.fonts);
  }

  return { state, set, setFont };
}

/* ---- Font CSS-variable application ---- */

const slotToCssVar: Record<FontSlot, string> = {
  ui: "--font-ui",
  prose: "--font-prose",
  monoInline: "--font-mono-inline",
  monoBlock: "--font-mono-block",
  monoEditor: "--font-mono-editor",
};

const slotToFallbackVar: Record<FontSlot, string> = {
  ui: "var(--font-stack-sans)",
  prose: "var(--font-stack-sans)",
  monoInline: "var(--font-stack-mono)",
  monoBlock: "var(--font-stack-mono)",
  monoEditor: "var(--font-stack-mono)",
};

/** Slots not exposed to UI but that should follow an exposed slot */
const aliasSlots: { cssVar: string; follows: FontSlot; fallback: string }[] = [
  { cssVar: "--font-mono-identifier", follows: "monoInline", fallback: "var(--font-stack-mono)" },
  { cssVar: "--font-mono-display",    follows: "monoEditor", fallback: "var(--font-stack-mono)" },
];

function applyFonts(fonts: Record<FontSlot, string>) {
  const root = document.documentElement;
  for (const slot of Object.keys(slotToCssVar) as FontSlot[]) {
    const custom = fonts[slot]?.trim();
    const cssVar = slotToCssVar[slot];
    if (custom) {
      root.style.setProperty(cssVar, `${custom}, ${slotToFallbackVar[slot]}`);
    } else {
      root.style.setProperty(cssVar, slotToFallbackVar[slot]);
    }
  }
  for (const alias of aliasSlots) {
    const custom = fonts[alias.follows]?.trim();
    if (custom) {
      root.style.setProperty(alias.cssVar, `${custom}, ${alias.fallback}`);
    } else {
      root.style.setProperty(alias.cssVar, alias.fallback);
    }
  }
}

/** Call once from App.vue to apply saved font overrides on startup */
export function initFonts() {
  applyFonts(state.fonts);
}
