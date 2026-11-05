// Settings navigation as data: groups → pages → sections. The sidebar, the
// settings search and `uiStore.openSettingsCategory(page, section)` all read
// this table. Section `keys` are setting keys or dotted prefixes from the
// settings table (`src-tauri/src/settings/definitions.rs`); they feed the
// search with each setting's label and description.
// Design: `design/设置系统/设置页面与存储整理.md` (页面结构).
import {
  Bell,
  Bot,
  Bug,
  ChartColumn,
  FolderTree,
  Globe,
  Info,
  Keyboard,
  KeyRound,
  LayoutGrid,
  Lock,
  MessageSquare,
  Network,
  Palette,
  PanelsTopLeft,
  ScrollText,
  Server,
  Settings,
  Sigma,
  Smartphone,
  type IconNode,
} from "lucide";
import { BLENDER_PROJECT_ICON, UNITY_PROJECT_ICON } from "../icons/projectIcons";

export interface SettingsSectionDef {
  id: string;
  /** i18n key of the section title. */
  titleKey: string;
  /** Setting keys or dotted prefixes shown in this section. */
  keys?: readonly string[];
  /** The whole section is experimental (individual settings carry their own flag). */
  experimental?: boolean;
}

export interface SettingsPageDef {
  id: string;
  group: SettingsGroupId;
  labelKey: string;
  icon: IconNode;
  sections: readonly SettingsSectionDef[];
}

export const SETTINGS_GROUPS = [
  { id: "models", labelKey: "settings.group.models" },
  { id: "agent", labelKey: "settings.group.agent" },
  { id: "workspace", labelKey: "settings.group.workspace" },
  { id: "integrations", labelKey: "settings.group.integrations" },
  { id: "interface", labelKey: "settings.group.interface" },
  { id: "connections", labelKey: "settings.group.connections" },
  { id: "system", labelKey: "settings.group.system" },
] as const;

export type SettingsGroupId = typeof SETTINGS_GROUPS[number]["id"];

export const SETTINGS_PAGES = [
  {
    id: "providers",
    group: "models",
    labelKey: "settings.tab.providers",
    icon: KeyRound,
    sections: [
      { id: "available", titleKey: "settings.models.available" },
      { id: "anthropic", titleKey: "settings.anthropic.title", keys: ["providers.anthropic."] },
      {
        id: "claudeCode",
        titleKey: "settings.claudeCode.title",
        keys: ["providers.claudeCode."],
        experimental: true,
      },
      { id: "codex", titleKey: "settings.codex.title", keys: ["providers.codex."] },
      { id: "openrouter", titleKey: "settings.provider.title" },
      { id: "custom", titleKey: "settings.custom.title" },
    ],
  },
  {
    id: "modelDefaults",
    group: "models",
    labelKey: "settings.tab.models",
    icon: LayoutGrid,
    sections: [{ id: "defaults", titleKey: "settings.tab.models", keys: ["models.defaults."] }],
  },
  {
    id: "modelUsage",
    group: "models",
    labelKey: "settings.tab.modelUsage",
    icon: ChartColumn,
    sections: [],
  },
  {
    id: "agentBehavior",
    group: "agent",
    labelKey: "settings.tab.agentBehavior",
    icon: Bot,
    sections: [
      { id: "newSession", titleKey: "settings.general.defaultAgent", keys: ["agent.newSession."] },
      { id: "subagents", titleKey: "settings.general.subagentLimits", keys: ["agent.subagent."] },
      { id: "requests", titleKey: "settings.section.requests", keys: ["agent.llm."] },
      { id: "toolLoading", titleKey: "settings.dynamicToolLoading.title", keys: ["agent.tools."] },
      { id: "sessionUndo", titleKey: "settings.general.sessionUndo", keys: ["agent.sessionUndo"] },
    ],
  },
  {
    id: "permissions",
    group: "agent",
    labelKey: "settings.tab.permissions",
    icon: Lock,
    sections: [{ id: "permissions", titleKey: "settings.tab.permissions", keys: ["permissions."] }],
  },
  {
    id: "mcp",
    group: "agent",
    labelKey: "settings.tab.mcp",
    icon: Network,
    sections: [],
  },
  {
    id: "workspace",
    group: "workspace",
    labelKey: "settings.tab.workspace",
    icon: FolderTree,
    sections: [
      { id: "project", titleKey: "settings.project.titleNone", keys: ["project.knowledgeEnabled", "project.markdownImages."] },
      { id: "tree", titleKey: "settings.section.workspaceTree", keys: ["workspace.tree."] },
      { id: "files", titleKey: "settings.display.fileExplorerTitle", keys: ["workspace.explorer."] },
      { id: "search", titleKey: "settings.tab.globalSearch", keys: ["workspace.search."] },
      { id: "worktree", titleKey: "settings.section.worktree", keys: ["workspace.worktree."], experimental: true },
      { id: "resources", titleKey: "settings.resources.title", keys: ["workspace.resources."] },
    ],
  },
  {
    id: "unity",
    group: "integrations",
    labelKey: "settings.tab.unity",
    icon: UNITY_PROJECT_ICON,
    sections: [
      { id: "enable", titleKey: "settings.services.unityEnabled", keys: ["unity.enabled"] },
      {
        id: "connection",
        titleKey: "settings.section.unityConnection",
        keys: ["unity.externalEditor.", "unity.embed.", "unity.connection."],
      },
      { id: "codeAnalysis", titleKey: "settings.codeAnalysis.master", keys: ["unity.codeAnalysis."] },
      {
        id: "compile",
        titleKey: "settings.tab.hotReload",
        keys: ["unity.compile.sidecar", "unity.compile.nonPublicAccess", "unity.hotReload.enabled"],
      },
      { id: "editors", titleKey: "settings.resources.editors.title" },
      { id: "testing", titleKey: "settings.tab.testing", keys: ["project.unityTestToolsEnabled"] },
      {
        id: "multiAgent",
        titleKey: "settings.experimental.unityMultiAgentEditor",
        keys: ["unity.multiAgentEditor"],
        experimental: true,
      },
      { id: "diagnostics", titleKey: "settings.section.unityDiagnostics" },
    ],
  },
  {
    id: "blender",
    group: "integrations",
    labelKey: "settings.tab.blender",
    icon: BLENDER_PROJECT_ICON,
    sections: [{ id: "blender", titleKey: "settings.tab.blender", keys: ["blender."] }],
  },
  {
    id: "latex",
    group: "integrations",
    labelKey: "settings.tab.latex",
    icon: Sigma,
    sections: [
      { id: "toolchain", titleKey: "settings.latex.toolchain" },
      {
        id: "preview",
        titleKey: "settings.latex.previewSection",
        keys: ["latex.engine", "latex.preview.", "latex.toolchain."],
      },
    ],
  },
  {
    id: "appearance",
    group: "interface",
    labelKey: "settings.tab.appearance",
    icon: Palette,
    sections: [
      { id: "language", titleKey: "settings.general.language", keys: ["appearance.language"] },
      { id: "theme", titleKey: "settings.display.themeTitle", keys: ["appearance.theme."] },
      { id: "fonts", titleKey: "settings.display.fontTitle", keys: ["appearance.fonts."] },
      { id: "chrome", titleKey: "settings.display.mainChromeTitle", keys: ["appearance.chrome."] },
    ],
  },
  {
    id: "chat",
    group: "interface",
    labelKey: "settings.tab.chat",
    icon: MessageSquare,
    sections: [
      { id: "messages", titleKey: "settings.section.messages", keys: ["chat.messages.", "chat.composer."] },
      { id: "history", titleKey: "settings.display.sessionHistoryTitle", keys: ["chat.history."] },
    ],
  },
  {
    id: "workbench",
    group: "interface",
    labelKey: "settings.tab.workbench",
    icon: PanelsTopLeft,
    sections: [
      { id: "tabs", titleKey: "settings.display.sessionTabsTitle", keys: ["workbench.tabs."] },
      { id: "panels", titleKey: "settings.display.panelBehaviorTitle", keys: ["workbench.panels."] },
      { id: "open", titleKey: "settings.section.openTargets", keys: ["workbench.open.", "workbench.views."] },
      { id: "git", titleKey: "settings.display.gitViewTitle", keys: ["workbench.git."] },
      { id: "documents", titleKey: "settings.section.documentEditing", keys: ["documents."] },
    ],
  },
  {
    id: "notifications",
    group: "interface",
    labelKey: "settings.tab.notifications",
    icon: Bell,
    sections: [
      { id: "system", titleKey: "settings.notifications.systemTitle", keys: ["notifications.system."] },
      { id: "sound", titleKey: "settings.notifications.soundTitle", keys: ["notifications.sound."] },
      { id: "hints", titleKey: "settings.notifications.cacheTitle", keys: ["notifications.hints."] },
    ],
  },
  {
    id: "shortcuts",
    group: "interface",
    labelKey: "settings.tab.shortcuts",
    icon: Keyboard,
    sections: [
      { id: "shortcuts", titleKey: "settings.shortcuts.title", keys: ["chat.input.", "shortcuts."] },
    ],
  },
  {
    id: "proxy",
    group: "connections",
    labelKey: "settings.tab.proxy",
    icon: Globe,
    sections: [{ id: "proxy", titleKey: "settings.tab.proxy", keys: ["network.proxy."] }],
  },
  {
    id: "mcpServer",
    group: "connections",
    labelKey: "settings.tab.mcpServer",
    icon: Server,
    sections: [{ id: "mcpServer", titleKey: "settings.tab.mcpServer", keys: ["mcpServer."] }],
  },
  {
    id: "remoteAccess",
    group: "connections",
    labelKey: "settings.tab.remoteAccess",
    icon: Smartphone,
    sections: [
      { id: "remoteAccess", titleKey: "settings.tab.remoteAccess", keys: ["remoteAccess."], experimental: true },
    ],
  },
  {
    id: "system",
    group: "system",
    labelKey: "settings.tab.system",
    icon: Settings,
    sections: [
      { id: "closeBehavior", titleKey: "settings.general.closeBehavior", keys: ["system.closeBehavior"] },
      { id: "storage", titleKey: "settings.general.storage" },
      { id: "tempFiles", titleKey: "settings.general.tempFiles" },
      { id: "git", titleKey: "settings.general.gitRuntime", keys: ["runtimes.git."] },
      { id: "python", titleKey: "settings.general.pythonRuntime", keys: ["runtimes.python."] },
      { id: "reset", titleKey: "settings.general.resetOnboarding" },
    ],
  },
  {
    id: "debug",
    group: "system",
    labelKey: "settings.tab.debug",
    icon: Bug,
    sections: [
      { id: "debugMode", titleKey: "settings.general.debugMode", keys: ["system.debug"] },
      { id: "toolFailureLog", titleKey: "settings.general.toolFailureLog", keys: ["system.toolFailureLog"] },
    ],
  },
  {
    id: "logs",
    group: "system",
    labelKey: "settings.tab.logs",
    icon: ScrollText,
    sections: [],
  },
  {
    id: "about",
    group: "system",
    labelKey: "settings.tab.about",
    icon: Info,
    sections: [{ id: "updates", titleKey: "settings.about.updateChannel", keys: ["updates."] }],
  },
] as const satisfies readonly SettingsPageDef[];

export type SettingsPageId = typeof SETTINGS_PAGES[number]["id"];

export const DEFAULT_SETTINGS_PAGE: SettingsPageId = "providers";

export function isSettingsPageId(value: unknown): value is SettingsPageId {
  return SETTINGS_PAGES.some((page) => page.id === value);
}

export function settingsPage(id: SettingsPageId): SettingsPageDef {
  return SETTINGS_PAGES.find((page) => page.id === id)!;
}

export function settingsPagesInGroup(group: SettingsGroupId): readonly SettingsPageDef[] {
  return SETTINGS_PAGES.filter((page) => page.group === group);
}

/** Whether `key` belongs to a section listing `patterns` (keys or `prefix.`). */
export function sectionIncludesKey(patterns: readonly string[] | undefined, key: string): boolean {
  return (patterns ?? []).some((pattern) => pattern.endsWith(".") ? key.startsWith(pattern) : key === pattern);
}

export interface SettingsSearchEntry {
  pageId: SettingsPageId;
  sectionId: string | null;
  /** Translated texts searched for the query. */
  texts: string[];
  /** Setting keys the entry covers (also searchable). */
  keys: string[];
  experimental: boolean;
}

export interface SettingsSearchResult {
  pageId: SettingsPageId;
  sectionId: string | null;
  /** Label of the matching setting, when a single setting matched. */
  settingLabel: string | null;
}

/**
 * Builds the search index. `settingKeys` comes from the settings schema
 * (empty when it is unavailable); `experimentalKeys` marks experimental
 * settings so that searching the experimental badge lists them.
 */
export function buildSettingsSearchIndex(
  translate: (key: string) => string,
  settingKeys: readonly string[],
  experimentalKeys: ReadonlySet<string>,
): SettingsSearchEntry[] {
  const entries: SettingsSearchEntry[] = [];
  for (const page of SETTINGS_PAGES as readonly SettingsPageDef[]) {
    const pageId = page.id as SettingsPageId;
    entries.push({ pageId, sectionId: null, texts: [translate(page.labelKey)], keys: [], experimental: false });
    for (const section of page.sections) {
      const keys = settingKeys.filter((key) => sectionIncludesKey(section.keys, key));
      entries.push({
        pageId,
        sectionId: section.id,
        texts: [translate(section.titleKey)],
        keys,
        experimental: Boolean(section.experimental) || keys.some((key) => experimentalKeys.has(key)),
      });
    }
  }
  return entries;
}

function normalized(text: string): string {
  return text.toLocaleLowerCase().replace(/\s+/g, " ").trim();
}

/**
 * Matches a query against page and section titles, setting labels,
 * descriptions and keys. The experimental badge text (or "experimental")
 * lists every experimental section.
 */
export function searchSettings(
  index: readonly SettingsSearchEntry[],
  query: string,
  translate: (key: string) => string,
): SettingsSearchResult[] {
  const needle = normalized(query);
  if (!needle) return [];
  const experimentalWords = [normalized(translate("settings.badge.experimental")), "experimental"];
  const wantsExperimental = experimentalWords.some((word) =>
    (needle.length >= 2 && word.startsWith(needle)) || needle.includes(word));
  const results: SettingsSearchResult[] = [];
  const seen = new Set<string>();
  const add = (result: SettingsSearchResult) => {
    const id = `${result.pageId}/${result.sectionId ?? ""}/${result.settingLabel ?? ""}`;
    if (seen.has(id)) return;
    seen.add(id);
    results.push(result);
  };
  for (const entry of index) {
    if (wantsExperimental && entry.experimental) {
      add({ pageId: entry.pageId, sectionId: entry.sectionId, settingLabel: null });
      continue;
    }
    if (entry.texts.some((text) => normalized(text).includes(needle))) {
      add({ pageId: entry.pageId, sectionId: entry.sectionId, settingLabel: null });
      continue;
    }
    for (const key of entry.keys) {
      const label = translate(`settings.item.${key}.label`);
      const description = translate(`settings.item.${key}.desc`);
      if ([label, description, key].some((text) => normalized(text).includes(needle))) {
        add({ pageId: entry.pageId, sectionId: entry.sectionId, settingLabel: label });
      }
    }
  }
  return results;
}
