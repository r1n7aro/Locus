import { reactive, watch } from "vue";
import {
  DISPLAY_FIELD_KEYS,
  DISPLAY_GROUP_FIELDS,
  DISPLAY_GROUP_PREFIXES,
} from "./settingsLegacy";
import { saveSetting, saveSettings, settingValue } from "./useSettingsStore";
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
export type ViewOpenTarget = "tab" | "window";
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
/** Session tabs each workbench editor group keeps mounted; 0 keeps every tab mounted. */
export const DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP = 6;
export const MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS = [1, 2, 3, 6, 10, 0] as const;
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

export function normalizeMountedSessionEditorsPerGroup(value: unknown): number {
  // Number(null) and Number("") are 0, which would silently mean "unlimited".
  const parsed = typeof value === "number" || (typeof value === "string" && value.trim() !== "")
    ? Number(value)
    : Number.NaN;
  return MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS.includes(
    parsed as (typeof MOUNTED_SESSION_EDITORS_PER_GROUP_OPTIONS)[number],
  )
    ? parsed
    : DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP;
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
  /** Show unread results in the workspace tree. */
  showSessionUnreadIndicators: boolean;
  /** Move completed sessions to the start of their contiguous sibling block. */
  autoPromoteCompletedSessions: boolean;
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
  /** Experimental worktree selection before the first chat message. */
  worktreeEnabled: boolean;
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
  /**
   * Session tabs each workbench editor group keeps mounted (active, recent and
   * running ones); hidden tabs beyond it unload and reload when shown. 0 = all.
   */
  mountedSessionEditorsPerGroup: number;
  /** Collapse completed tool call batches in chat transcript */
  compactToolCalls: boolean;
  /** Hide completed thinking blocks in chat transcript */
  hideThinkingBlocks: boolean;
  /** Show View packages in the lower section of the session list */
  showViewsInSessionPanel: boolean;
  /** Default target for user-initiated View opens. */
  viewOpenTarget: ViewOpenTarget;
  viewOpenOverrides: Record<string, ViewOpenTarget>;
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
  showSessionUnreadIndicators: true,
  autoPromoteCompletedSessions: true,
  fileExplorerHiddenDirectories: [...DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES],
  unityFileExplorerHiddenDirectories: [...DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES],
  showPluginsTab: true,
  showAgentTab: true,
  showAgentSelector: true,
  worktreeEnabled: false,
  todoAutoOpen: true,
  changesAutoOpen: false,
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
  mountedSessionEditorsPerGroup: DEFAULT_MOUNTED_SESSION_EDITORS_PER_GROUP,
  compactToolCalls: true,
  hideThinkingBlocks: true,
  showViewsInSessionPanel: false,
  viewOpenTarget: "tab",
  viewOpenOverrides: {},
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

type ScalarDisplayField = Exclude<
  keyof DisplaySettings,
  "fonts" | "workspaceSectionVisibility" | "knowledgeFolderVisibility"
>;

/** Setting key behind each scalar field (see `settingsLegacy.ts`). */
const FIELD_KEYS: Record<ScalarDisplayField, string> = DISPLAY_FIELD_KEYS;
const FONT_SLOTS = DISPLAY_GROUP_FIELDS.fonts;
const SECTION_KINDS = DISPLAY_GROUP_FIELDS.workspaceSectionVisibility;
const KNOWLEDGE_FOLDER_KINDS = DISPLAY_GROUP_FIELDS.knowledgeFolderVisibility;

const ENUM_FIELDS: Partial<Record<ScalarDisplayField, readonly string[]>> = {
  workspaceDisplayMode: ["single", "multi"],
  viewOpenTarget: ["tab", "window"],
  chatDiffReviewTarget: ["inline", "window"],
  gitDiffReviewTarget: ["inline", "window"],
  planApprovalTarget: ["card", "window"],
  memoryFileOpenTarget: ["window", "knowledge"],
  soundAlertMode: ["soft", "bright", "urgent"],
  soundAlertSource: ["builtin", "custom"],
};

function normalizeViewOpenOverrides(value: unknown): Record<string, ViewOpenTarget> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value as Record<string, unknown>)
    .filter(([, target]) => target === "tab" || target === "window")) as Record<string, ViewOpenTarget>;
}

function readField<K extends ScalarDisplayField>(field: K, raw: unknown): DisplaySettings[K] {
  const fallback = defaults[field];
  let value: unknown;
  switch (field) {
    case "sessionMessagePageSize":
      value = normalizeSessionMessagePageSize(raw);
      break;
    case "mountedSessionEditorsPerGroup":
      value = normalizeMountedSessionEditorsPerGroup(raw);
      break;
    case "assetRefClickAction":
    case "unityEmbedAssetRefClickAction":
      value = normalizeAssetRefClickAction(raw, fallback as AssetRefClickAction);
      break;
    case "fileExplorerHiddenDirectories":
      value = normalizeHiddenDirectoryNames(raw, DEFAULT_FILE_EXPLORER_HIDDEN_DIRECTORIES);
      break;
    case "unityFileExplorerHiddenDirectories":
      value = normalizeHiddenDirectoryNames(raw, DEFAULT_UNITY_FILE_EXPLORER_HIDDEN_DIRECTORIES);
      break;
    case "viewOpenOverrides":
      value = normalizeViewOpenOverrides(raw);
      break;
    case "soundAlertVolume":
      value = typeof raw === "number" && Number.isFinite(raw)
        ? Math.min(100, Math.max(0, Math.round(raw)))
        : fallback;
      break;
    default: {
      const allowed = ENUM_FIELDS[field];
      if (allowed) value = allowed.includes(raw as string) ? raw : fallback;
      else value = typeof raw === typeof fallback ? raw : fallback;
    }
  }
  return value as DisplaySettings[K];
}

function project(): DisplaySettings {
  const next = {} as DisplaySettings;
  for (const field of Object.keys(FIELD_KEYS) as ScalarDisplayField[]) {
    (next as unknown as Record<string, unknown>)[field] = readField(field, settingValue(FIELD_KEYS[field]));
  }
  next.fonts = Object.fromEntries(FONT_SLOTS.map((slot) => {
    const raw = settingValue(DISPLAY_GROUP_PREFIXES.fonts + slot);
    return [slot, typeof raw === "string" ? raw : ""];
  })) as Record<FontSlot, string>;
  next.workspaceSectionVisibility = Object.fromEntries(SECTION_KINDS.map((kind) => {
    const raw = settingValue(DISPLAY_GROUP_PREFIXES.workspaceSectionVisibility + kind);
    return [kind, typeof raw === "boolean" ? raw : defaultWorkspaceSectionVisibility[kind]];
  })) as Record<WorkspaceSectionVisibilityKind, boolean>;
  next.knowledgeFolderVisibility = Object.fromEntries(KNOWLEDGE_FOLDER_KINDS.map((kind) => {
    const raw = settingValue(DISPLAY_GROUP_PREFIXES.knowledgeFolderVisibility + kind);
    return [kind, typeof raw === "boolean" ? raw : defaultKnowledgeFolderVisibility[kind]];
  })) as Record<KnowledgeFolderKind, boolean>;
  return next;
}

const state = reactive<DisplaySettings>(project());

/** Every window follows `settings.json`: changes from this window, other
 *  windows and the agent all arrive through the settings snapshot. Only
 *  fields whose value moved are replaced, so unrelated watchers stay quiet. */
watch(project, (next) => {
  const target = state as unknown as Record<string, unknown>;
  let fontsChanged = false;
  for (const [field, value] of Object.entries(next)) {
    if (JSON.stringify(target[field]) === JSON.stringify(value)) continue;
    target[field] = value;
    if (field === "fonts") fontsChanged = true;
  }
  if (fontsChanged && typeof document !== "undefined") applyFonts(state.fonts);
}, { flush: "sync" });

function groupWrites<G extends keyof typeof DISPLAY_GROUP_PREFIXES>(
  group: G,
  value: Record<string, unknown>,
): { key: string; value: unknown }[] {
  return DISPLAY_GROUP_FIELDS[group]
    .filter((field) => value[field] !== undefined)
    .map((field) => ({ key: DISPLAY_GROUP_PREFIXES[group] + field, value: value[field] }));
}

export function useDisplaySettings() {
  function set<K extends keyof DisplaySettings>(key: K, value: DisplaySettings[K]) {
    if (key === "fonts" || key === "workspaceSectionVisibility" || key === "knowledgeFolderVisibility") {
      saveSettings(groupWrites(
        key as keyof typeof DISPLAY_GROUP_PREFIXES,
        value as unknown as Record<string, unknown>,
      ));
      return;
    }
    const field = key as ScalarDisplayField;
    const normalized = field === "fileExplorerHiddenDirectories" || field === "unityFileExplorerHiddenDirectories"
      ? normalizeHiddenDirectoryNames(value)
      : readField(field, value);
    saveSetting(FIELD_KEYS[field], normalized);
  }

  function setFont(slot: FontSlot, value: string) {
    saveSetting(DISPLAY_GROUP_PREFIXES.fonts + slot, value);
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
