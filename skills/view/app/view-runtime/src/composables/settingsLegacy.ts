// Preferences that older builds kept in localStorage, mapped to setting keys.
// Read once to seed the settings store before the first backend snapshot
// and to import them into `settings.json`; Locus never writes these keys again.
import {
  SHORTCUT_ACTIONS,
  SHORTCUT_SETTING_KEYS,
  createDefaultShortcutSettings,
  normalizeShortcut,
  sameShortcut,
  shortcutRequiresModifier,
  shortcutToSetting,
  type ShortcutDefinition,
} from "./keyboardShortcutCore";

export const LEGACY_STORAGE_KEYS = {
  locale: "locus-locale",
  theme: "locus-theme-preference",
  unityEmbedTheme: "locus-unity-embed-theme-preference",
  display: "locus-display-settings",
  chatInput: "locus-chat-input-settings",
  shortcuts: "locus-keyboard-shortcuts",
  newSession: "locus-new-session-settings",
  documentEditing: "locus-document-editing-settings",
  globalSearch: "locus-global-search-settings",
  updateChannel: "locus-app-update-channel",
} as const;

/** `locus-display-settings` field → setting key, for scalar fields. */
export const DISPLAY_FIELD_KEYS = {
  showWelcomeSubtitle: "appearance.chrome.showWelcomeSubtitle",
  showPluginsTab: "appearance.chrome.showPluginsTab",
  showAgentTab: "appearance.chrome.showAgentTab",
  rightAlignUserMessages: "chat.messages.rightAlignUser",
  showTurnNavigationRail: "chat.messages.turnNavigationRail",
  compactToolCalls: "chat.messages.compactToolCalls",
  hideThinkingBlocks: "chat.messages.hideThinkingBlocks",
  showAgentSelector: "chat.composer.showAgentSelector",
  sessionMessagePageSize: "chat.history.pageSize",
  mountedSessionEditorsPerGroup: "workbench.tabs.mountedSessionEditorsPerGroup",
  todoAutoOpen: "workbench.panels.todoAutoOpen",
  changesAutoOpen: "workbench.panels.changesAutoOpen",
  changesAutoClose: "workbench.panels.changesAutoClose",
  fileChangePopoverEnabled: "workbench.panels.fileChangePopover",
  chatDiffReviewTarget: "workbench.open.chatDiffReview",
  gitDiffReviewTarget: "workbench.open.gitDiffReview",
  planApprovalTarget: "workbench.open.planApproval",
  memoryFileOpenTarget: "workbench.open.memoryFile",
  viewOpenTarget: "workbench.open.view",
  viewOpenOverrides: "workbench.open.viewOverrides",
  assetRefClickAction: "workbench.open.assetRefClick",
  unityEmbedAssetRefClickAction: "workbench.open.unityEmbedAssetRefClick",
  showViewsInSessionPanel: "workbench.views.showInSessionPanel",
  mergeGitTreeStatusIcon: "workbench.git.mergeTreeStatusIcon",
  hideGitCommandSuggestions: "workbench.git.hideCommandSuggestions",
  systemNotificationsEnabled: "notifications.system.enabled",
  notifyOnChatDone: "notifications.system.onChatDone",
  notifyOnSubagentDone: "notifications.system.onSubagentDone",
  notifyOnAskUser: "notifications.system.onAskUser",
  notifyOnChatError: "notifications.system.onChatError",
  notifyOnToolConfirm: "notifications.system.onToolConfirm",
  soundAlertsEnabled: "notifications.sound.enabled",
  soundAlertMode: "notifications.sound.mode",
  soundAlertSource: "notifications.sound.source",
  soundAlertCustomFilePath: "notifications.sound.customFile",
  soundAlertVolume: "notifications.sound.volume",
  soundOnChatDone: "notifications.sound.onChatDone",
  soundOnSubagentDone: "notifications.sound.onSubagentDone",
  soundOnAskUser: "notifications.sound.onAskUser",
  soundOnChatError: "notifications.sound.onChatError",
  soundOnToolConfirm: "notifications.sound.onToolConfirm",
  cacheInvalidationWarningsEnabled: "notifications.hints.cacheInvalidationWarnings",
  workspaceDisplayMode: "workspace.tree.displayMode",
  autoPlaceNewPlanDesignKnowledgeDocuments: "workspace.tree.autoPlaceNewPlanDesignDocuments",
  showSessionUnreadIndicators: "workspace.tree.showUnreadIndicators",
  autoPromoteCompletedSessions: "workspace.tree.autoPromoteCompletedSessions",
  fileExplorerHiddenDirectories: "workspace.explorer.hiddenDirectories",
  unityFileExplorerHiddenDirectories: "workspace.explorer.unityHiddenDirectories",
  worktreeEnabled: "workspace.worktree.enabled",
} as const;

/** Nested `locus-display-settings` objects → setting key prefix per entry. */
export const DISPLAY_GROUP_PREFIXES = {
  fonts: "appearance.fonts.",
  workspaceSectionVisibility: "workspace.tree.sections.",
  knowledgeFolderVisibility: "workspace.tree.knowledgeFolders.",
} as const;

export const DISPLAY_GROUP_FIELDS = {
  fonts: ["ui", "prose", "monoInline", "monoBlock", "monoEditor"],
  workspaceSectionVisibility: ["knowledge", "collab", "assets", "views"],
  knowledgeFolderVisibility: ["plan", "memory", "design", "skill", "reference"],
} as const;

export const GLOBAL_SEARCH_FIELD_KEYS = {
  enabled: "workspace.search.enabled",
  includeArchived: "workspace.search.includeArchived",
  knowledgeTitle: "workspace.search.sources.knowledgeTitle",
  knowledgeContent: "workspace.search.sources.knowledgeContent",
  sessionTitle: "workspace.search.sources.sessionTitle",
  sessionContent: "workspace.search.sources.sessionContent",
  fileName: "workspace.search.sources.fileName",
  viewName: "workspace.search.sources.viewName",
} as const;

type Storage = Pick<globalThis.Storage, "getItem">;

function defaultStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function readText(storage: Storage, key: string): string | null {
  try {
    return storage.getItem(key);
  } catch {
    return null;
  }
}

function readObject(storage: Storage, key: string): Record<string, unknown> | null {
  const raw = readText(storage, key);
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function legacyLocale(value: string | null): string | null {
  const normalized = value?.trim().toLowerCase().replace(/_/g, "-") ?? "";
  if (normalized === "zh" || normalized.startsWith("zh-")) return "zh";
  if (normalized === "en" || normalized.startsWith("en-")) return "en";
  return null;
}

function legacyTheme(value: string | null): string | null {
  return value === "system" || value === "light" || value === "dark" ? value : null;
}

/**
 * Setting values found in the legacy localStorage keys. Only keys that were
 * actually stored are returned; values are passed through as stored and
 * normalized by the backend import and by each setting's reader.
 */
export function readLegacyFrontendSettings(
  storage: Storage | null = defaultStorage(),
): Record<string, unknown> {
  const values: Record<string, unknown> = {};
  if (!storage) return values;

  const locale = legacyLocale(readText(storage, LEGACY_STORAGE_KEYS.locale));
  if (locale) values["appearance.language"] = locale;
  const theme = legacyTheme(readText(storage, LEGACY_STORAGE_KEYS.theme));
  if (theme) values["appearance.theme.main"] = theme;
  const unityTheme = legacyTheme(readText(storage, LEGACY_STORAGE_KEYS.unityEmbedTheme));
  if (unityTheme) values["appearance.theme.unityEmbed"] = unityTheme;

  const display = readObject(storage, LEGACY_STORAGE_KEYS.display);
  if (display) {
    for (const [field, key] of Object.entries(DISPLAY_FIELD_KEYS)) {
      if (display[field] !== undefined && display[field] !== null) values[key] = display[field];
    }
    for (const [group, prefix] of Object.entries(DISPLAY_GROUP_PREFIXES)) {
      const entries = display[group];
      if (!entries || typeof entries !== "object") continue;
      for (const field of DISPLAY_GROUP_FIELDS[group as keyof typeof DISPLAY_GROUP_FIELDS]) {
        const value = (entries as Record<string, unknown>)[field];
        if (value !== undefined && value !== null) values[prefix + field] = value;
      }
    }
  }

  const chatInput = readObject(storage, LEGACY_STORAGE_KEYS.chatInput);
  if (chatInput) {
    if (typeof chatInput.submitMode === "string") values["chat.input.submitMode"] = chatInput.submitMode;
    if (typeof chatInput.runningSendMode === "string") {
      values["chat.input.runningSendMode"] = chatInput.runningSendMode;
    }
  }

  const shortcuts = readObject(storage, LEGACY_STORAGE_KEYS.shortcuts);
  if (shortcuts) {
    const defaults = createDefaultShortcutSettings();
    for (const action of SHORTCUT_ACTIONS) {
      const stored = shortcuts[action] as Partial<ShortcutDefinition> | undefined;
      if (!stored) continue;
      const shortcut = normalizeShortcut(stored, defaults[action], shortcutRequiresModifier(action));
      // The platform default stays empty so it keeps following the platform.
      if (!sameShortcut(shortcut, defaults[action])) {
        values[SHORTCUT_SETTING_KEYS[action]] = shortcutToSetting(shortcut);
      }
    }
  }

  const newSession = readObject(storage, LEGACY_STORAGE_KEYS.newSession);
  if (newSession) {
    const agentId = typeof newSession.defaultAgentId === "string" ? newSession.defaultAgentId.trim() : "";
    const mode = newSession.agentMode;
    if (mode === "remember" || mode === "workspace" || mode === "fixed") {
      values["agent.newSession.agentMode"] = mode === "fixed" && !agentId ? "remember" : mode;
    }
    if (agentId) values["agent.newSession.defaultAgentId"] = agentId;
  }

  const documentEditing = readObject(storage, LEGACY_STORAGE_KEYS.documentEditing);
  if (documentEditing) {
    if (typeof documentEditing.punctuationPairing === "boolean") {
      values["documents.punctuationPairing"] = documentEditing.punctuationPairing;
    }
    if (typeof documentEditing.commentAuthor === "string") {
      values["documents.commentAuthor"] = documentEditing.commentAuthor;
    }
  }

  const globalSearch = readObject(storage, LEGACY_STORAGE_KEYS.globalSearch);
  if (globalSearch) {
    for (const [field, key] of Object.entries(GLOBAL_SEARCH_FIELD_KEYS)) {
      if (typeof globalSearch[field] === "boolean") values[key] = globalSearch[field];
    }
  }

  const channel = readText(storage, LEGACY_STORAGE_KEYS.updateChannel);
  if (channel === "stable" || channel === "experimental") values["updates.channel"] = channel;

  return values;
}

/** The pre-migration global default Agent; seeds projects without remembered choices. */
export function readLegacyNewSessionAgentId(storage: Storage | null = defaultStorage()): string {
  if (!storage) return "";
  const record = readObject(storage, LEGACY_STORAGE_KEYS.newSession);
  if (!record) return "";
  const mode = record.agentMode;
  if (mode === "remember" || mode === "workspace" || mode === "fixed") {
    return typeof record.legacyAgentId === "string" ? record.legacyAgentId.trim() : "";
  }
  // Older builds stored only an Agent id that overrode every manual switch.
  return typeof record.defaultAgentId === "string" ? record.defaultAgentId.trim() : "";
}
