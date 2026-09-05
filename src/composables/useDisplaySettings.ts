import { reactive } from "vue";
import type {
  NotificationSoundMode,
  NotificationSoundSource,
} from "../services/notificationSounds";

export type FontSlot = "ui" | "prose" | "monoInline" | "monoBlock" | "monoEditor";
export type DiffReviewTarget = "inline" | "window";
export type ChatDiffReviewTarget = DiffReviewTarget;
export type GitDiffReviewTarget = DiffReviewTarget;
export type PlanApprovalTarget = "card" | "window";
export type MemoryFileOpenTarget = "window" | "knowledge";
export type WorkspaceDisplayMode = "single" | "multi";
export type WorkspaceSectionVisibilityKind = "knowledge" | "collab" | "assets" | "views";
export type KnowledgeFolderKind = "plan" | "memory" | "design" | "skill" | "reference";
export type AssetRefClickAction =
  | "unitySelect"
  | "fileBrowser"
  | "unityInspector"
  | "locusInspector";

const LEGACY_LOCUS_INSPECTOR_ACTIONS = new Set([
  "locusInspectorAuto",
  "locusInspectorEmbedded",
  "locusInspectorWindow",
]);

export function normalizeAssetRefClickAction(
  value: unknown,
  fallback: AssetRefClickAction,
): AssetRefClickAction {
  if (LEGACY_LOCUS_INSPECTOR_ACTIONS.has(String(value))) return "locusInspector";
  if (
    value === "unitySelect"
    || value === "fileBrowser"
    || value === "unityInspector"
    || value === "locusInspector"
  ) return value;
  return fallback;
}

export const DEFAULT_SESSION_MESSAGE_PAGE_SIZE = 120;
export const SESSION_MESSAGE_PAGE_SIZE_OPTIONS = [80, 120, 160, 240, 400] as const;
export const DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES = [
  ".git",
  ".vs",
  ".vscode",
  ".idea",
  "node_modules",
  "__pycache__",
  ".next",
  "dist",
  "build",
  "obj",
] as const;
export const DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES = [
  "Library",
  "Temp",
  "Logs",
  "UserSettings",
  "MemoryCaptures",
] as const;

export function normalizeHiddenDirectoryNames(
  value: unknown,
  fallback: readonly string[] = [],
): string[] {
  const source = Array.isArray(value) ? value : fallback;
  const normalized: string[] = [];
  const seen = new Set<string>();
  for (const item of source) {
    if (typeof item !== "string") continue;
    const name = item.trim().replace(/[\\/]+$/g, "");
    if (!name || name === "." || name === ".." || name.includes("/") || name.includes("\\")) {
      continue;
    }
    const key = name.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    normalized.push(name);
  }
  return normalized;
}

export function normalizeSessionMessagePageSize(value: unknown): number {
  const parsed = Number(value);
  return SESSION_MESSAGE_PAGE_SIZE_OPTIONS.includes(
    parsed as (typeof SESSION_MESSAGE_PAGE_SIZE_OPTIONS)[number],
  )
    ? parsed
    : DEFAULT_SESSION_MESSAGE_PAGE_SIZE;
}

export interface DisplaySettings {
  /** Show the welcome subtitle above the chat input */
  showWelcomeSubtitle: boolean;
  /** Project tree projection used by the Development workbench */
  workspaceDisplayMode: WorkspaceDisplayMode;
  /** Fixed Development-tree sections visible for each ProjectContext. */
  workspaceSectionVisibility: Record<WorkspaceSectionVisibilityKind, boolean>;
  /** Knowledge roots projected into the Development tree */
  knowledgeFolderVisibility: Record<KnowledgeFolderKind, boolean>;
  /** Add newly created Plan and Design documents below the Knowledge tree entry. */
  autoPlaceNewPlanDesignKnowledgeDocuments: boolean;
  /** Directory names hidden from the Files page in every workspace. */
  fileExplorerHiddenDirectories: string[];
  /** Additional directory names hidden when the workspace is a Unity project. */
  unityFileExplorerHiddenDirectories: string[];
  /** Show Plugins tab in the top navigation */
  showPluginsTab: boolean;
  /** Show Agent tab in the top navigation */
  showAgentTab: boolean;
  /** Show the Agent column in chat model selectors */
  showAgentSelector: boolean;
  /** Show the Git sidebar in the Collaboration workspace */
  showCollabSidebar: boolean;
  /** Auto-open TODO panel when todos arrive */
  todoAutoOpen: boolean;
  /** Auto-open file changes panel when changes arrive */
  changesAutoOpen: boolean;
  /** Auto-close file changes panel when a new round starts */
  changesAutoClose: boolean;
  /** Enable hover preview popovers for file changes */
  fileChangePopoverEnabled: boolean;
  /** Default target for reviewing chat file diffs */
  chatDiffReviewTarget: DiffReviewTarget;
  /** Default target for reviewing Git file diffs */
  gitDiffReviewTarget: DiffReviewTarget;
  /** Default surface for the exit_plan_mode approval (inline card or standalone window) */
  planApprovalTarget: PlanApprovalTarget;
  /** Default target when opening a Memory document reference */
  memoryFileOpenTarget: MemoryFileOpenTarget;
  /** Default action when clicking a Unity asset reference in chat messages */
  assetRefClickAction: AssetRefClickAction;
  /** Click action override for chat running inside the Unity embed window */
  unityEmbedAssetRefClickAction: AssetRefClickAction;
  /** Right-align user messages in the session transcript */
  rightAlignUserMessages: boolean;
  /** Show the user-turn navigation rail when the transcript has enough left gutter */
  showTurnNavigationRail: boolean;
  /** Number of messages requested for initial session history and each older page */
  sessionMessagePageSize: number;
  /** Collapse completed tool call batches in chat transcript */
  compactToolCalls: boolean;
  /** Hide completed thinking blocks in chat transcript */
  hideThinkingBlocks: boolean;
  /** Show View packages in the lower section of the session list */
  showViewsInSessionPanel: boolean;
  /** Show the frontend log bar at the bottom of View windows */
  showViewLogBar: boolean;
  /** Merge Git tree status letters into colored file icons */
  mergeGitTreeStatusIcon: boolean;
  /** Hide Git command suggestions in Git terminal */
  hideGitCommandSuggestions: boolean;
  /** Enable desktop notifications when the app is not focused */
  systemNotificationsEnabled: boolean;
  /** Notify when a chat run completes */
  notifyOnChatDone: boolean;
  /** Notify when a subagent run completes */
  notifyOnSubagentDone: boolean;
  /** Notify when the agent asks the user a question */
  notifyOnAskUser: boolean;
  /** Notify when a chat run fails */
  notifyOnChatError: boolean;
  /** Notify when tool approval is required */
  notifyOnToolConfirm: boolean;
  /** Show an in-app warning when a request was expected to reuse the prompt cache but did not */
  cacheInvalidationWarningsEnabled: boolean;
  /** Enable sound alerts for key chat events */
  soundAlertsEnabled: boolean;
  /** Sound profile used for sound alerts */
  soundAlertMode: NotificationSoundMode;
  /** Sound source used for sound alerts */
  soundAlertSource: NotificationSoundSource;
  /** Custom sound file path used when soundAlertSource is custom */
  soundAlertCustomFilePath: string;
  /** Sound alert volume, stored as a percentage from 0 to 100 */
  soundAlertVolume: number;
  /** Play a sound when a chat run completes */
  soundOnChatDone: boolean;
  /** Play a sound when a subagent run completes */
  soundOnSubagentDone: boolean;
  /** Play a sound when the agent asks the user a question */
  soundOnAskUser: boolean;
  /** Play a sound when a chat run fails */
  soundOnChatError: boolean;
  /** Play a sound when tool approval is required */
  soundOnToolConfirm: boolean;
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

const defaultKnowledgeFolderVisibility: Record<KnowledgeFolderKind, boolean> = {
  plan: true,
  memory: true,
  design: true,
  skill: true,
  reference: true,
};

const defaultWorkspaceSectionVisibility: Record<WorkspaceSectionVisibilityKind, boolean> = {
  knowledge: true,
  collab: true,
  assets: true,
  views: true,
};

const defaults: DisplaySettings = {
  showWelcomeSubtitle: true,
  workspaceDisplayMode: "single",
  workspaceSectionVisibility: { ...defaultWorkspaceSectionVisibility },
  knowledgeFolderVisibility: { ...defaultKnowledgeFolderVisibility },
  autoPlaceNewPlanDesignKnowledgeDocuments: true,
  fileExplorerHiddenDirectories: [...DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES],
  unityFileExplorerHiddenDirectories: [...DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES],
  showPluginsTab: true,
  showAgentTab: true,
  showAgentSelector: true,
  showCollabSidebar: false,
  todoAutoOpen: true,
  changesAutoOpen: true,
  changesAutoClose: true,
  fileChangePopoverEnabled: true,
  chatDiffReviewTarget: "window",
  gitDiffReviewTarget: "window",
  planApprovalTarget: "card",
  memoryFileOpenTarget: "knowledge",
  assetRefClickAction: "locusInspector",
  // Inside the Unity embed window the editor's own Inspector is one click
  // away, so asset/GameObject refs open there by default.
  unityEmbedAssetRefClickAction: "unityInspector",
  rightAlignUserMessages: true,
  showTurnNavigationRail: true,
  sessionMessagePageSize: DEFAULT_SESSION_MESSAGE_PAGE_SIZE,
  compactToolCalls: true,
  hideThinkingBlocks: true,
  showViewsInSessionPanel: false,
  showViewLogBar: false,
  mergeGitTreeStatusIcon: true,
  hideGitCommandSuggestions: false,
  systemNotificationsEnabled: true,
  notifyOnChatDone: true,
  notifyOnSubagentDone: false,
  notifyOnAskUser: true,
  notifyOnChatError: true,
  notifyOnToolConfirm: true,
  cacheInvalidationWarningsEnabled: false,
  soundAlertsEnabled: false,
  soundAlertMode: "bright",
  soundAlertSource: "builtin",
  soundAlertCustomFilePath: "",
  soundAlertVolume: 50,
  soundOnChatDone: true,
  soundOnSubagentDone: false,
  soundOnAskUser: true,
  soundOnChatError: true,
  soundOnToolConfirm: true,
  fonts: { ...defaultFonts },
};

function load(): DisplaySettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        ...defaults,
        ...parsed,
        workspaceDisplayMode: parsed.workspaceDisplayMode === "multi" ? "multi" : "single",
        workspaceSectionVisibility: {
          ...defaultWorkspaceSectionVisibility,
          ...parsed.workspaceSectionVisibility,
        },
        knowledgeFolderVisibility: {
          ...defaultKnowledgeFolderVisibility,
          ...parsed.knowledgeFolderVisibility,
        },
        fileExplorerHiddenDirectories: normalizeHiddenDirectoryNames(
          parsed.fileExplorerHiddenDirectories,
          DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES,
        ),
        unityFileExplorerHiddenDirectories: normalizeHiddenDirectoryNames(
          parsed.unityFileExplorerHiddenDirectories,
          DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES,
        ),
        sessionMessagePageSize: normalizeSessionMessagePageSize(parsed.sessionMessagePageSize),
        assetRefClickAction: normalizeAssetRefClickAction(
          parsed.assetRefClickAction,
          defaults.assetRefClickAction,
        ),
        unityEmbedAssetRefClickAction: normalizeAssetRefClickAction(
          parsed.unityEmbedAssetRefClickAction,
          defaults.unityEmbedAssetRefClickAction,
        ),
        fonts: { ...defaultFonts, ...parsed.fonts },
      };
    }
  } catch { /* ignore */ }
  return {
    ...defaults,
    knowledgeFolderVisibility: { ...defaultKnowledgeFolderVisibility },
    workspaceSectionVisibility: { ...defaultWorkspaceSectionVisibility },
    fileExplorerHiddenDirectories: [...DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES],
    unityFileExplorerHiddenDirectories: [...DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES],
    fonts: { ...defaultFonts },
  };
}

function save(s: DisplaySettings) {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(s)); } catch { /* ignore */ }
}

const state = reactive<DisplaySettings>(load());

/** Keep every window's copy in sync: View host/content windows are separate
 *  webviews sharing the same origin, so a settings change in the main window
 *  arrives here as a storage event (same mechanism as useTheme). */
function handleStorageChange(event: StorageEvent) {
  if (event.key !== null && event.key !== STORAGE_KEY) return;
  const next = load();
  Object.assign(state, next, {
    knowledgeFolderVisibility: { ...next.knowledgeFolderVisibility },
    workspaceSectionVisibility: { ...next.workspaceSectionVisibility },
    fileExplorerHiddenDirectories: [...next.fileExplorerHiddenDirectories],
    unityFileExplorerHiddenDirectories: [...next.unityFileExplorerHiddenDirectories],
    fonts: { ...next.fonts },
  });
  applyFonts(state.fonts);
}

if (typeof window !== "undefined") {
  window.addEventListener("storage", handleStorageChange);
}

export function useDisplaySettings() {
  function set<K extends keyof DisplaySettings>(key: K, value: DisplaySettings[K]) {
    state[key] = (
      key === "sessionMessagePageSize"
        ? normalizeSessionMessagePageSize(value)
        : key === "assetRefClickAction"
          ? normalizeAssetRefClickAction(value, defaults.assetRefClickAction)
        : key === "unityEmbedAssetRefClickAction"
          ? normalizeAssetRefClickAction(value, defaults.unityEmbedAssetRefClickAction)
        : key === "fileExplorerHiddenDirectories" || key === "unityFileExplorerHiddenDirectories"
          ? normalizeHiddenDirectoryNames(value)
        : value
    ) as DisplaySettings[K];
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
