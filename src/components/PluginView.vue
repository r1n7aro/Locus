<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Copy,
  ExternalLink,
  FolderOpen,
  HelpCircle,
  KeyRound,
  Link,
  LoaderCircle,
  LogOut,
  Package,
  PackageCheck,
  Plus,
  RefreshCw,
  Save,
  Search,
  Settings2,
  Star,
  Store,
  Trash2,
  UserCheck,
  X,
} from "lucide";
import type { IconNode } from "lucide";
import { locale, t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { useResizablePanel } from "../composables/useResizablePanel";
import { useCopyFeedback } from "../composables/useCopyFeedback";
import {
  DEFAULT_PLUGIN_REGISTRY_BRANCH,
  DEFAULT_PLUGIN_REGISTRY_PATH,
  DEFAULT_PLUGIN_REGISTRY_SOURCE,
  normalizePluginRegistryPath,
  normalizePluginRegistrySources,
  parsePluginRegistryRepoInput,
  pluginRegistrySourceBaseUrl,
  pluginRegistrySourceRepoLabel,
  type PluginRegistrySource,
} from "../services/pluginRegistrySources";
import {
  pluginListInstalled,
  pluginGithubAuthLogout,
  pluginGithubAuthStatus,
  pluginGithubOAuthPoll,
  pluginGithubOAuthStart,
  pluginGithubRepoSetStarred,
  pluginGithubRepoStarStatus,
  pluginInstallFromRegistry,
  pluginInstallFromSource,
  pluginRegistryFetchDescription,
  pluginRegistryFetchManifest,
  pluginRegistryFetchPlugin,
  pluginRegistryFetchSearchIndex,
  pluginRegistryFetchShard,
  pluginUninstall,
  type InstalledPluginSummary,
  type PluginGithubAuthStatus,
  type PluginGithubRepoStarStatus,
  type PluginInstallScope,
  type PluginRegistryCacheMode,
  type PluginRegistryDescriptionSource,
  type PluginRegistryEntry,
  type PluginRegistryManifest,
  type PluginRegistryStat,
  type PluginRegistrySummary,
} from "../services/plugin";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import { useNotificationStore } from "../stores/notification";
import BaseButton from "./ui/BaseButton.vue";
import BaseContextMenu from "./ui/BaseContextMenu.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "./icons/locusViewIcons";
import MarkdownRenderer from "./MarkdownRenderer.vue";

const props = defineProps<{
  workingDir: string;
}>();

const PLUGIN_REGISTRY_SOURCES_STORAGE_KEY = "locus:plugin-hub:registries";
type PluginListMode = "installed" | "registry";
const REGISTRY_BUCKET_BATCH_SIZE = 8;
const REGISTRY_VISIBLE_PAGE_SIZE = 30;
const REGISTRY_SCROLL_LOAD_OFFSET = 160;
const REGISTRY_INSTALL_SCOPE: PluginInstallScope = "app";

interface PluginRegistrySourceDraft {
  id: string;
  name: string;
  repoInput: string;
  branch: string;
  path: string;
}

interface PluginRegistrySourceRef {
  registryKey: string;
  registrySourceId: string;
  registrySourceName: string;
  registrySourceRepo: string;
  registrySourceBranch: string;
  registrySourcePath: string;
  registrySourceLabel: string;
}

interface PluginListContextMenuState {
  x: number;
  y: number;
}

interface RegistryLoadOptions {
  cacheMode?: PluginRegistryCacheMode | null;
}

interface RegistryRefreshOptions extends RegistryLoadOptions {
  preserveExisting?: boolean;
  silent?: boolean;
}

type RegistryPluginSummary = PluginRegistrySummary & PluginRegistrySourceRef;
type RegistryPluginEntry = PluginRegistryEntry & PluginRegistrySourceRef;
type DirectImportMode = "local" | "link";

const notificationStore = useNotificationStore();
const pluginLayoutRef = ref<HTMLElement | null>(null);
const installedPlugins = ref<InstalledPluginSummary[]>([]);
const loading = ref(false);
const uninstallKey = ref("");
const uninstallConfirmKey = ref("");
const loadError = ref("");
const selectedPluginKey = ref("");
const registryBaseUrls = ref<Record<string, string>>({});
const registryManifests = ref<Record<string, PluginRegistryManifest>>({});
const registrySummaries = ref<RegistryPluginSummary[]>([]);
const registryDetails = ref<Record<string, RegistryPluginEntry>>({});
const registryDescriptions = ref<Record<string, string>>({});
const registryLoadedBuckets = ref<Record<string, Set<string>>>({});
const registrySearchIndexLoaded = ref<Record<string, boolean>>({});
const registryLoading = ref(false);
const registryLoadingMore = ref(false);
const registrySearchIndexLoading = ref(false);
const registryInstalledEntriesLoading = ref(false);
const registryInstallKey = ref("");
const registryDetailLoadingId = ref("");
const registryDescriptionLoadingId = ref("");
const registryDescriptionError = ref("");
const registryError = ref("");
const pluginSearch = ref("");
const registryVisibleCount = ref(REGISTRY_VISIBLE_PAGE_SIZE);
const selectedRegistryKey = ref("");
const pluginListMode = ref<PluginListMode>("registry");
const registrySources = ref<PluginRegistrySource[]>([{ ...DEFAULT_PLUGIN_REGISTRY_SOURCE }]);
const registrySourceDrafts = ref<PluginRegistrySourceDraft[]>([]);
const registryConfigOpen = ref(false);
const registryConfigError = ref("");
const pluginHelpOpen = ref(false);
const githubAuthStatus = ref<PluginGithubAuthStatus>({ authenticated: false, account: "" });
const githubAuthSaving = ref(false);
const githubAuthError = ref("");
const githubStarStatusByRepo = ref<Record<string, PluginGithubRepoStarStatus>>({});
const githubStarLoadingRepo = ref("");
const githubStarSavingRepo = ref("");
const githubStarError = ref("");
type GithubOAuthStep = "idle" | "opening" | "waiting" | "success" | "error";
const githubOAuthOpen = ref(false);
const githubOAuthStep = ref<GithubOAuthStep>("idle");
const githubOAuthUserCode = ref("");
const githubOAuthUrl = ref("");
const githubOAuthDeviceCode = ref("");
const githubOAuthInterval = ref(5);
const githubOAuthError = ref("");
const { copied: githubOAuthCodeCopied, copyText: copyGithubOAuthText, reset: resetGithubOAuthCopyState } = useCopyFeedback();
const pluginListContextMenu = ref<PluginListContextMenuState | null>(null);
const directImportOpen = ref(false);
const directImportMode = ref<DirectImportMode>("link");
const directImportSource = ref("");
const directImportScope = ref<PluginInstallScope>("app");
const directImportInstalling = ref(false);
const directImportError = ref("");
let unlistenPluginsChanged: UnlistenFn | null = null;
let unlistenKnowledgeChanged: UnlistenFn | null = null;
let unlistenViewTreeChanged: UnlistenFn | null = null;
let githubOAuthPollTimer: ReturnType<typeof setTimeout> | null = null;
let githubOAuthPollInFlight = false;

const {
  size: installedPaneWidth,
  isDragging: resizingInstalledPane,
  onMouseDown: onInstalledPaneResizeMouseDown,
} = useResizablePanel(pluginLayoutRef, {
  storageKey: "locus:plugins:installed-pane-width",
  defaultSize: 420,
  minSize: 280,
  maxSize: (container) => Math.max(320, Math.min(720, container.clientWidth * 0.7)),
});

const installedSorted = computed(() =>
  [...installedPlugins.value].sort((left, right) =>
    left.scope.localeCompare(right.scope)
    || left.id.localeCompare(right.id),
  ),
);
const hasWorkspace = computed(() => !!props.workingDir.trim());
const selectedInstalledPlugin = computed(() =>
  installedSorted.value.find((plugin) => `${plugin.scope}:${plugin.id}` === selectedPluginKey.value)
  ?? null,
);
const filteredInstalledPlugins = computed(() => {
  const query = normalizeSearch(pluginSearch.value);
  if (!query) return installedSorted.value;
  return installedSorted.value.filter((plugin) => {
    const registryDisplay = installedRegistryDisplay(plugin);
    const haystack = normalizeSearch([
      registryDisplay?.name,
      localizedRegistrySummary(registryDisplay),
      ...localizedRegistryTextValues(registryDisplay?.summaryI18n),
      registryDisplay?.author,
      registryDisplay?.registrySourceName,
      registryDisplay?.registrySourceRepo,
      registryDisplay?.registrySourceBranch,
      ...(registryDisplay?.tags ?? []),
      plugin.name,
      plugin.id,
      plugin.version,
      plugin.scope,
    ].join(" "));
    return haystack.includes(query);
  });
});
const installedPluginById = computed(() => {
  const byId = new Map<string, InstalledPluginSummary[]>();
  for (const plugin of installedSorted.value) {
    const installed = byId.get(plugin.id) ?? [];
    installed.push(plugin);
    byId.set(plugin.id, installed);
  }
  for (const installed of byId.values()) {
    installed.sort((left, right) =>
      pluginScopeSortValue(left.scope) - pluginScopeSortValue(right.scope)
      || left.id.localeCompare(right.id),
    );
  }
  return byId;
});
const hasMoreRegistryBuckets = computed(() =>
  pendingRegistryBucketRefs(1).length > 0,
);
const sortedRegistrySummaries = computed(() =>
  [...registrySummaries.value].sort((left, right) =>
    Number(installedPluginById.value.has(right.id)) - Number(installedPluginById.value.has(left.id))
    || right.updatedAt.localeCompare(left.updatedAt)
    || left.name.localeCompare(right.name)
    || left.registrySourceLabel.localeCompare(right.registrySourceLabel)
    || left.id.localeCompare(right.id),
  ),
);
const filteredRegistrySummaries = computed(() => {
  const query = normalizeSearch(pluginSearch.value);
  if (!query) return sortedRegistrySummaries.value;
  return sortedRegistrySummaries.value.filter((plugin) => {
    const haystack = normalizeSearch([
      plugin.name,
      plugin.id,
      localizedRegistrySummary(plugin),
      ...localizedRegistryTextValues(plugin.summaryI18n),
      plugin.author,
      plugin.registrySourceName,
      plugin.registrySourceRepo,
      plugin.registrySourceBranch,
      ...(plugin.tags ?? []),
    ].join(" "));
    return haystack.includes(query);
  });
});
const registryNameSourceCounts = computed(() => {
  const counts = new Map<string, Set<string>>();
  for (const plugin of registrySummaries.value) {
    const nameKey = normalizeSearch(plugin.name || plugin.id);
    const sourceIds = counts.get(nameKey) ?? new Set<string>();
    sourceIds.add(plugin.registrySourceId);
    counts.set(nameKey, sourceIds);
  }
  return counts;
});
const visibleRegistrySummaries = computed(() =>
  filteredRegistrySummaries.value.slice(0, registryVisibleCount.value),
);
const hasMoreVisibleRegistrySummaries = computed(() =>
  filteredRegistrySummaries.value.length > registryVisibleCount.value,
);
const hasMoreRegistryListItems = computed(() =>
  hasMoreVisibleRegistrySummaries.value || hasMoreRegistryBuckets.value,
);
const selectedRegistrySummary = computed(() =>
  registrySummaries.value.find((plugin) => plugin.registryKey === selectedRegistryKey.value) ?? null,
);
const selectedRegistryDetail = computed(() =>
  selectedRegistryKey.value ? registryDetails.value[selectedRegistryKey.value] ?? null : null,
);
const selectedRegistryDisplay = computed<RegistryPluginEntry | RegistryPluginSummary | null>(() =>
  selectedRegistryDetail.value ?? selectedRegistrySummary.value,
);
const selectedRegistryGithubRepo = computed(() =>
  registryGithubRepoKey(selectedRegistryDetail.value),
);
const selectedRegistryStarStatus = computed(() =>
  selectedRegistryGithubRepo.value ? githubStarStatusByRepo.value[selectedRegistryGithubRepo.value] ?? null : null,
);
const selectedRegistryStarBusy = computed(() =>
  selectedRegistryGithubRepo.value === githubStarLoadingRepo.value
  || selectedRegistryGithubRepo.value === githubStarSavingRepo.value,
);
const selectedRegistryDescriptionContent = computed(() => {
  const display = selectedRegistryDisplay.value;
  if (!display) return "";
  const detailed = registryDescriptions.value[registryDescriptionCacheKey(display)]?.trim();
  if (detailed) return detailed;
  return localizedRegistryDescription(selectedRegistryDetail.value)
    || localizedRegistrySummary(display)
    || display.id;
});
const pluginListModeToggleIcon = computed(() =>
  pluginListMode.value === "registry" ? PackageCheck : Store,
);
const pluginListModeToggleTitle = computed(() =>
  pluginListMode.value === "registry" ? t("plugin.installed.title") : t("plugin.hub.registry"),
);
function errorMessage(error: unknown): string {
  return normalizeAppError(error).message;
}

function normalizeSearch(value: string): string {
  return value.trim().toLowerCase();
}

function registryLocaleCandidates(): string[] {
  return locale.value === "zh" ? ["zh", "en"] : ["en", "zh"];
}

function localizedRegistryText(
  values: Record<string, string> | null | undefined,
  fallback = "",
): string {
  const normalizedValues = values ?? {};
  for (const candidate of registryLocaleCandidates()) {
    const value = normalizedValues[candidate]?.trim();
    if (value) return value;
  }
  for (const value of Object.values(normalizedValues)) {
    const trimmed = value.trim();
    if (trimmed) return trimmed;
  }
  return fallback.trim();
}

function localizedRegistryTextValues(values: Record<string, string> | null | undefined): string[] {
  return Object.values(values ?? {})
    .map((value) => value.trim())
    .filter(Boolean);
}

function localizedRegistryDescriptionSource(
  plugin: PluginRegistryEntry,
): PluginRegistryDescriptionSource | null {
  const values = plugin.descriptionSourceI18n ?? {};
  for (const candidate of registryLocaleCandidates()) {
    const source = values[candidate];
    if (registryDescriptionSourceHasValue(source)) return source;
  }
  for (const source of Object.values(values)) {
    if (registryDescriptionSourceHasValue(source)) return source;
  }
  return registryDescriptionSourceHasValue(plugin.descriptionSource) ? plugin.descriptionSource ?? null : null;
}

function localizedRegistrySummary(plugin: PluginRegistrySummary | PluginRegistryEntry | null): string {
  if (!plugin) return "";
  return localizedRegistryText(plugin.summaryI18n, plugin.summary || plugin.id);
}

function localizedRegistryDescription(plugin: PluginRegistryEntry | null): string {
  if (!plugin) return "";
  return localizedRegistryText(plugin.descriptionI18n, plugin.description || localizedRegistrySummary(plugin));
}

function registryDescriptionCacheKey(plugin: RegistryPluginEntry | RegistryPluginSummary): string {
  return `${plugin.registryKey}:${locale.value}`;
}

function registryDescriptionSourceHasValue(
  source: PluginRegistryDescriptionSource | null | undefined,
): boolean {
  return !!(
    source?.url?.trim()
    || source?.repo?.trim()
    || source?.path?.trim()
  );
}

function selectedPluginIdentity(plugin: InstalledPluginSummary): string {
  return `${plugin.scope}:${plugin.id}`;
}

function selectInstalledPlugin(plugin: InstalledPluginSummary) {
  const key = selectedPluginIdentity(plugin);
  pluginListMode.value = "installed";
  selectedPluginKey.value = key;
}

function togglePluginListMode() {
  const nextMode: PluginListMode = pluginListMode.value === "registry" ? "installed" : "registry";
  pluginListMode.value = nextMode;
  if (nextMode === "installed") {
    registryConfigOpen.value = false;
    return;
  }
  resetRegistryVisibleCount();
}

function openPluginListContextMenu(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
  pluginListContextMenu.value = { x: event.clientX, y: event.clientY };
}

function closePluginListContextMenu() {
  pluginListContextMenu.value = null;
}

function openPluginHelp() {
  pluginHelpOpen.value = true;
  registryConfigOpen.value = false;
  directImportOpen.value = false;
}

function closePluginHelp() {
  pluginHelpOpen.value = false;
}

async function refreshPluginListFromMenu() {
  closePluginListContextMenu();
  await refreshAll();
  await refreshRegistry({ cacheMode: "networkPreferred" });
}

function openDirectImport(mode: DirectImportMode) {
  closePluginListContextMenu();
  registryConfigOpen.value = false;
  pluginHelpOpen.value = false;
  directImportMode.value = mode;
  directImportSource.value = "";
  directImportScope.value = "app";
  directImportError.value = "";
  directImportOpen.value = true;
}

function closeDirectImport() {
  if (directImportInstalling.value) return;
  directImportOpen.value = false;
  directImportError.value = "";
}

async function chooseDirectImportFile() {
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Plugin", extensions: ["zip"] }],
  });
  if (typeof selected === "string") {
    directImportSource.value = selected;
  }
}

async function chooseDirectImportFolder() {
  const selected = await open({
    multiple: false,
    directory: true,
  });
  if (typeof selected === "string") {
    directImportSource.value = selected;
  }
}

function setDirectImportScope(scope: PluginInstallScope) {
  if (scope === "project" && !hasWorkspace.value) return;
  directImportScope.value = scope;
}

async function installDirectPlugin() {
  if (directImportInstalling.value) return;
  const input = directImportSource.value.trim();
  if (!input) {
    directImportError.value = t("plugin.import.emptySource");
    return;
  }
  directImportInstalling.value = true;
  directImportError.value = "";
  try {
    const installed = await pluginInstallFromSource({
      type: directImportMode.value === "local" ? "local" : "auto",
      input,
    }, directImportScope.value);
    notificationStore.addNotice(
      "success",
      t("plugin.notice.installed", installed.name || installed.id),
      { operation: "pluginDirectInstall" },
    );
    directImportOpen.value = false;
    await refreshAll();
    void loadInstalledRegistryEntries();
  } catch (error) {
    directImportError.value = errorMessage(error);
    notificationStore.addNotice("error", directImportError.value, { operation: "pluginDirectInstall" });
  } finally {
    directImportInstalling.value = false;
  }
}

function toggleRegistryConfig() {
  registryConfigOpen.value = !registryConfigOpen.value;
  if (registryConfigOpen.value) {
    directImportOpen.value = false;
    pluginHelpOpen.value = false;
    syncRegistryDraftsFromSources();
    void refreshGithubAuthStatus();
  }
}

async function refreshGithubAuthStatus() {
  try {
    githubAuthStatus.value = await pluginGithubAuthStatus();
    githubAuthError.value = "";
  } catch (error) {
    githubAuthError.value = errorMessage(error);
  }
}

function stopGithubOAuthPoll() {
  if (githubOAuthPollTimer) {
    clearTimeout(githubOAuthPollTimer);
    githubOAuthPollTimer = null;
  }
  githubOAuthPollInFlight = false;
}

function scheduleGithubOAuthPoll(delayMs = githubOAuthInterval.value * 1000) {
  if (githubOAuthPollTimer) clearTimeout(githubOAuthPollTimer);
  githubOAuthPollTimer = setTimeout(() => {
    githubOAuthPollTimer = null;
    void pollGithubOAuth();
  }, delayMs);
}

async function openGithubOAuth() {
  githubOAuthOpen.value = true;
  registryConfigOpen.value = false;
  directImportOpen.value = false;
  pluginHelpOpen.value = false;
  githubOAuthError.value = "";
  await refreshGithubAuthStatus();
  if (githubAuthStatus.value.authenticated) {
    githubOAuthStep.value = "success";
    stopGithubOAuthPoll();
    return;
  }
  void startGithubOAuth();
}

async function startGithubOAuth() {
  if (githubOAuthStep.value === "opening" || githubOAuthStep.value === "waiting") return;
  stopGithubOAuthPoll();
  resetGithubOAuthCopyState();
  githubOAuthError.value = "";
  githubOAuthStep.value = "opening";
  try {
    const info = await pluginGithubOAuthStart();
    if (info.auth) {
      githubAuthStatus.value = info.auth;
      githubOAuthStep.value = "success";
      githubOAuthError.value = "";
      notificationStore.addNotice("success", t("plugin.hub.githubLoginSaved"), { operation: "pluginGithubAuth" });
      return;
    }

    githubOAuthUserCode.value = info.userCode ?? "";
    githubOAuthUrl.value = info.verificationUri ?? "";
    githubOAuthDeviceCode.value = info.deviceCode ?? "";
    githubOAuthInterval.value = Math.max(info.interval || 5, 5);
    githubOAuthStep.value = "waiting";

    if (githubOAuthUserCode.value && githubOAuthUrl.value) {
      void openUrl(githubOAuthUrl.value).catch(() => undefined);
    }
    scheduleGithubOAuthPoll();
  } catch (error) {
    githubOAuthStep.value = "error";
    githubOAuthError.value = errorMessage(error);
  }
}

async function pollGithubOAuth() {
  if (githubOAuthPollInFlight || githubOAuthStep.value !== "waiting") return;
  githubOAuthPollInFlight = true;
  try {
    const result = await pluginGithubOAuthPoll(githubOAuthDeviceCode.value);
    if (result.status === "success" && result.auth) {
      stopGithubOAuthPoll();
      githubAuthStatus.value = result.auth;
      githubOAuthStep.value = "success";
      githubOAuthError.value = "";
      notificationStore.addNotice("success", t("plugin.hub.githubLoginSaved"), { operation: "pluginGithubAuth" });
      return;
    }
    if (result.status === "failed") {
      stopGithubOAuthPoll();
      githubOAuthStep.value = "error";
      githubOAuthError.value = result.message || t("plugin.hub.githubOAuthFailed");
      return;
    }
    if (result.message) {
      githubOAuthError.value = result.message;
      githubOAuthInterval.value += 5;
    }
    if (githubOAuthStep.value === "waiting") scheduleGithubOAuthPoll();
  } catch (error) {
    console.warn("Failed to poll GitHub OAuth:", error);
    if (githubOAuthStep.value === "waiting") scheduleGithubOAuthPoll();
  } finally {
    githubOAuthPollInFlight = false;
  }
}

function closeGithubOAuth() {
  stopGithubOAuthPoll();
  resetGithubOAuthCopyState();
  githubOAuthOpen.value = false;
  githubOAuthStep.value = "idle";
  githubOAuthError.value = "";
}

async function copyGithubOAuthCode() {
  if (!githubOAuthUserCode.value) return;
  await copyGithubOAuthText(githubOAuthUserCode.value);
}

async function logoutGithubAuth() {
  if (githubAuthSaving.value) return;
  stopGithubOAuthPoll();
  githubAuthSaving.value = true;
  githubAuthError.value = "";
  try {
    githubAuthStatus.value = await pluginGithubAuthLogout();
    githubOAuthStep.value = "idle";
    notificationStore.addNotice("success", t("plugin.hub.githubLoggedOut"), { operation: "pluginGithubAuth" });
  } catch (error) {
    githubAuthError.value = errorMessage(error);
    notificationStore.addNotice("error", githubAuthError.value, { operation: "pluginGithubAuth" });
  } finally {
    githubAuthSaving.value = false;
  }
}

function loadRegistrySourceSettings() {
  let nextSources: PluginRegistrySource[] = [{ ...DEFAULT_PLUGIN_REGISTRY_SOURCE }];
  let shouldPersist = false;
  try {
    const rawSources = localStorage.getItem(PLUGIN_REGISTRY_SOURCES_STORAGE_KEY);
    if (rawSources) {
      nextSources = normalizePluginRegistrySources(JSON.parse(rawSources));
      shouldPersist = rawSources !== JSON.stringify(nextSources);
    }
  } catch (error) {
    console.warn("Failed to load plugin registry sources:", error);
  }
  registrySources.value = nextSources;
  if (shouldPersist) persistRegistrySourceSettings();
  syncRegistryDraftsFromSources();
}

function persistRegistrySourceSettings() {
  try {
    localStorage.setItem(PLUGIN_REGISTRY_SOURCES_STORAGE_KEY, JSON.stringify(registrySources.value));
  } catch (error) {
    console.warn("Failed to save plugin registry sources:", error);
  }
}

function registrySourceToDraft(source: PluginRegistrySource): PluginRegistrySourceDraft {
  return {
    id: source.id,
    name: source.name,
    repoInput: pluginRegistrySourceRepoLabel(source),
    branch: source.branch || DEFAULT_PLUGIN_REGISTRY_BRANCH,
    path: source.path || DEFAULT_PLUGIN_REGISTRY_PATH,
  };
}

function syncRegistryDraftsFromSources() {
  registrySourceDrafts.value = registrySources.value.map(registrySourceToDraft);
  registryConfigError.value = "";
}

function uniqueRegistrySourceId(baseId: string, seenIds: Set<string>): string {
  const normalized = baseId.trim() || `registry-${Date.now().toString(36)}`;
  let nextId = normalized;
  let index = 2;
  while (seenIds.has(nextId)) {
    nextId = `${normalized}-${index}`;
    index += 1;
  }
  seenIds.add(nextId);
  return nextId;
}

function addRegistrySource() {
  registrySourceDrafts.value = [
    ...registrySourceDrafts.value,
    {
      id: `registry-${Date.now().toString(36)}`,
      name: t("plugin.hub.registryNewName"),
      repoInput: "",
      branch: DEFAULT_PLUGIN_REGISTRY_BRANCH,
      path: DEFAULT_PLUGIN_REGISTRY_PATH,
    },
  ];
  registryConfigOpen.value = true;
}

function removeRegistrySourceDraft(index: number) {
  if (registrySourceDrafts.value.length <= 1) return;
  registrySourceDrafts.value = registrySourceDrafts.value.filter((_, itemIndex) => itemIndex !== index);
}

function saveRegistrySources() {
  const seenIds = new Set<string>();
  const nextSources: PluginRegistrySource[] = [];
  for (const draft of registrySourceDrafts.value) {
    const repo = parsePluginRegistryRepoInput(draft.repoInput);
    if (!repo) {
      registryConfigError.value = t("plugin.hub.registryInvalidRepo");
      return;
    }
    const path = normalizePluginRegistryPath(repo.path || draft.path || DEFAULT_PLUGIN_REGISTRY_PATH);
    if (!path) {
      registryConfigError.value = t("plugin.hub.registryInvalidPath");
      return;
    }
    nextSources.push({
      id: uniqueRegistrySourceId(draft.id, seenIds),
      name: draft.name.trim() || `${repo.owner}/${repo.repo}`,
      owner: repo.owner,
      repo: repo.repo,
      url: repo.url,
      branch: repo.branch || draft.branch.trim() || DEFAULT_PLUGIN_REGISTRY_BRANCH,
      path,
    });
  }
  registrySources.value = normalizePluginRegistrySources(nextSources);
  registryConfigError.value = "";
  persistRegistrySourceSettings();
  syncRegistryDraftsFromSources();
  registryConfigOpen.value = false;
  void refreshRegistry();
}

function registrySourceLabel(source: PluginRegistrySource): string {
  const repo = pluginRegistrySourceRepoLabel(source);
  const branch = source.branch || DEFAULT_PLUGIN_REGISTRY_BRANCH;
  return `${source.name || repo} @ ${branch}`;
}

function registryItemKey(sourceId: string, pluginId: string): string {
  return `${sourceId}::${pluginId}`;
}

function withRegistrySource<T extends PluginRegistrySummary>(
  plugin: T,
  source: PluginRegistrySource,
): T & PluginRegistrySourceRef {
  return {
    ...plugin,
    registryKey: registryItemKey(source.id, plugin.id),
    registrySourceId: source.id,
    registrySourceName: source.name || pluginRegistrySourceRepoLabel(source),
    registrySourceRepo: pluginRegistrySourceRepoLabel(source),
    registrySourceBranch: source.branch || DEFAULT_PLUGIN_REGISTRY_BRANCH,
    registrySourcePath: source.path || DEFAULT_PLUGIN_REGISTRY_PATH,
    registrySourceLabel: registrySourceLabel(source),
  };
}

function registrySourceById(sourceId: string): PluginRegistrySource | null {
  return registrySources.value.find((source) => source.id === sourceId) ?? null;
}

function shouldShowRegistrySource(plugin: RegistryPluginSummary | RegistryPluginEntry): boolean {
  const nameKey = normalizeSearch(plugin.name || plugin.id);
  return (registryNameSourceCounts.value.get(nameKey)?.size ?? 0) > 1;
}

function registryLoadedBucketSet(sourceId: string): Set<string> {
  return registryLoadedBuckets.value[sourceId] ?? new Set<string>();
}

function setRegistryLoadedBucketSet(sourceId: string, buckets: Set<string>) {
  registryLoadedBuckets.value = {
    ...registryLoadedBuckets.value,
    [sourceId]: buckets,
  };
}

function mergeRegistrySummaries(nextPlugins: RegistryPluginSummary[]) {
  const byKey = new Map(registrySummaries.value.map((plugin) => [plugin.registryKey, plugin]));
  for (const plugin of nextPlugins) {
    byKey.set(plugin.registryKey, plugin);
  }
  registrySummaries.value = Array.from(byKey.values());
  if (selectedRegistryKey.value && !byKey.has(selectedRegistryKey.value)) {
    selectedRegistryKey.value = "";
  }
}

function registryEntryToSummary(entry: RegistryPluginEntry): RegistryPluginSummary {
  return {
    id: entry.id,
    name: entry.name,
    summary: entry.summary,
    summaryI18n: entry.summaryI18n,
    author: entry.author,
    tags: entry.tags,
    latestVersion: entry.latestVersion,
    updatedAt: entry.updatedAt,
    icon: entry.icon,
    stats: entry.stats,
    compatibility: entry.compatibility,
    registryKey: entry.registryKey,
    registrySourceId: entry.registrySourceId,
    registrySourceName: entry.registrySourceName,
    registrySourceRepo: entry.registrySourceRepo,
    registrySourceBranch: entry.registrySourceBranch,
    registrySourcePath: entry.registrySourcePath,
    registrySourceLabel: entry.registrySourceLabel,
  };
}

async function loadInstalledRegistryEntries() {
  if (registryInstalledEntriesLoading.value) return;
  const installedIds = Array.from(installedPluginById.value.keys());
  if (installedIds.length === 0) return;
  const requests = registrySources.value.flatMap((source) => {
    const manifest = registryManifests.value[source.id];
    const registryBaseUrl = registryBaseUrls.value[source.id];
    if (!manifest || !registryBaseUrl) return [];
    return installedIds
      .filter((pluginId) => !registrySummaries.value.some((plugin) => plugin.registryKey === registryItemKey(source.id, pluginId)))
      .map((pluginId) =>
        pluginRegistryFetchPlugin({
          registryBaseUrl,
          entryBasePath: manifest.entryBasePath,
          pluginId,
        }).then((entry) => withRegistrySource(entry, source)),
      );
  });
  if (requests.length === 0) return;
  registryInstalledEntriesLoading.value = true;
  try {
    for (let index = 0; index < requests.length; index += 4) {
      const settled = await Promise.allSettled(requests.slice(index, index + 4));
      const entries = settled
        .filter((result): result is PromiseFulfilledResult<RegistryPluginEntry> => result.status === "fulfilled")
        .map((result) => result.value);
      if (entries.length === 0) continue;
      registryDetails.value = entries.reduce<Record<string, RegistryPluginEntry>>((nextDetails, entry) => {
        nextDetails[entry.registryKey] = entry;
        return nextDetails;
      }, { ...registryDetails.value });
      mergeRegistrySummaries(entries.map(registryEntryToSummary));
    }
  } finally {
    registryInstalledEntriesLoading.value = false;
  }
}

interface RegistryBucketRef {
  source: PluginRegistrySource;
  bucket: string;
}

function pendingRegistryBucketRefs(count: number): RegistryBucketRef[] {
  const pendingBySource = registrySources.value
    .map((source) => {
      const manifest = registryManifests.value[source.id];
      if (!manifest) return null;
      const loaded = registryLoadedBucketSet(source.id);
      return {
        source,
        buckets: manifest.availableBuckets.filter((bucket) => !loaded.has(bucket)),
      };
    })
    .filter((value): value is { source: PluginRegistrySource; buckets: string[] } => !!value && value.buckets.length > 0);
  const refs: RegistryBucketRef[] = [];
  while (refs.length < count && pendingBySource.some((item) => item.buckets.length > 0)) {
    for (const item of pendingBySource) {
      const bucket = item.buckets.shift();
      if (!bucket) continue;
      refs.push({ source: item.source, bucket });
      if (refs.length >= count) break;
    }
  }
  return refs;
}

async function loadRegistryBucket(source: PluginRegistrySource, bucket: string, options: RegistryLoadOptions = {}) {
  const manifest = registryManifests.value[source.id];
  if (!manifest || registryLoadedBucketSet(source.id).has(bucket)) return;
  const shard = await pluginRegistryFetchShard({
    registryBaseUrl: registryBaseUrls.value[source.id],
    summaryBasePath: manifest.summaryBasePath,
    bucket,
    cacheMode: options.cacheMode,
  });
  mergeRegistrySummaries((shard.plugins ?? []).map((plugin) => withRegistrySource(plugin, source)));
  setRegistryLoadedBucketSet(source.id, new Set([...registryLoadedBucketSet(source.id), bucket]));
}

async function loadRegistryBuckets(refs: RegistryBucketRef[], options: RegistryLoadOptions = {}) {
  for (let index = 0; index < refs.length; index += 4) {
    await Promise.all(refs.slice(index, index + 4).map((ref) =>
      loadRegistryBucket(ref.source, ref.bucket, options),
    ));
  }
}

async function loadMoreRegistryBuckets(count = REGISTRY_BUCKET_BATCH_SIZE, options: RegistryLoadOptions = {}) {
  if (registryLoadingMore.value) return;
  const nextBuckets = pendingRegistryBucketRefs(count);
  if (nextBuckets.length === 0) return;
  registryLoadingMore.value = true;
  registryError.value = "";
  try {
    await loadRegistryBuckets(nextBuckets, options);
  } catch (error) {
    registryError.value = errorMessage(error);
  } finally {
    registryLoadingMore.value = false;
  }
}

async function loadRegistrySearchIndex(source: PluginRegistrySource, options: RegistryLoadOptions = {}) {
  if (registrySearchIndexLoaded.value[source.id]) return;
  const manifest = registryManifests.value[source.id];
  const registryBaseUrl = registryBaseUrls.value[source.id];
  if (!manifest || !registryBaseUrl) return;
  const index = await pluginRegistryFetchSearchIndex({
    registryBaseUrl,
    searchIndexPath: manifest.searchIndexPath,
    cacheMode: options.cacheMode,
  });
  mergeRegistrySummaries((index.plugins ?? []).map((plugin) => withRegistrySource(plugin, source)));
  registrySearchIndexLoaded.value = {
    ...registrySearchIndexLoaded.value,
    [source.id]: true,
  };
}

async function loadRegistrySearchIndexes(options: RegistryLoadOptions = {}) {
  if (registrySearchIndexLoading.value) return;
  const sources = registrySources.value.filter((source) => !registrySearchIndexLoaded.value[source.id]);
  if (sources.length === 0) return;
  registrySearchIndexLoading.value = true;
  try {
    for (let index = 0; index < sources.length; index += 4) {
      await Promise.allSettled(sources.slice(index, index + 4).map((source) =>
        loadRegistrySearchIndex(source, options),
      ));
    }
  } finally {
    registrySearchIndexLoading.value = false;
  }
}

function resetRegistryVisibleCount() {
  registryVisibleCount.value = REGISTRY_VISIBLE_PAGE_SIZE;
}

async function loadNextRegistryPage() {
  if (hasMoreVisibleRegistrySummaries.value) {
    registryVisibleCount.value += REGISTRY_VISIBLE_PAGE_SIZE;
    return;
  }
  if (!hasMoreRegistryBuckets.value || registryLoadingMore.value) return;
  await loadMoreRegistryBuckets();
  registryVisibleCount.value += REGISTRY_VISIBLE_PAGE_SIZE;
}

function handleRegistryListScroll(event: Event) {
  const list = event.currentTarget as HTMLElement | null;
  if (!list) return;
  const remaining = list.scrollHeight - list.scrollTop - list.clientHeight;
  if (remaining > REGISTRY_SCROLL_LOAD_OFFSET) return;
  void loadNextRegistryPage();
}

async function refreshRegistry(options: RegistryRefreshOptions = {}) {
  const preserveExisting = options.preserveExisting === true;
  const showLoading = !options.silent || registrySummaries.value.length === 0;
  if (showLoading) {
    registryLoading.value = true;
  }
  registryError.value = "";
  if (preserveExisting) {
    registryLoadedBuckets.value = {};
    registrySearchIndexLoaded.value = {};
  } else {
    selectedRegistryKey.value = "";
    registrySummaries.value = [];
    registryDetails.value = {};
    registryDescriptions.value = {};
    registryDescriptionError.value = "";
    registryDescriptionLoadingId.value = "";
    registryLoadedBuckets.value = {};
    registrySearchIndexLoaded.value = {};
    registryBaseUrls.value = {};
    registryManifests.value = {};
    resetRegistryVisibleCount();
  }
  try {
    const settled = await Promise.allSettled(registrySources.value.map((source) =>
      pluginRegistryFetchManifest(pluginRegistrySourceBaseUrl(source), options.cacheMode)
        .then((result) => ({ source, result })),
    ));
    const nextBaseUrls: Record<string, string> = {};
    const nextManifests: Record<string, PluginRegistryManifest> = {};
    const errors: string[] = [];
    for (const result of settled) {
      if (result.status === "fulfilled") {
        nextBaseUrls[result.value.source.id] = result.value.result.baseUrl;
        nextManifests[result.value.source.id] = result.value.result.manifest;
      } else {
        errors.push(errorMessage(result.reason));
      }
    }
    registryBaseUrls.value = nextBaseUrls;
    registryManifests.value = nextManifests;
    if (Object.keys(nextManifests).length === 0 && errors.length > 0) {
      if (!preserveExisting || registrySummaries.value.length === 0) {
        registryError.value = errors.join("\n");
      }
      return;
    }
    await loadMoreRegistryBuckets(REGISTRY_BUCKET_BATCH_SIZE, { cacheMode: options.cacheMode });
    void loadInstalledRegistryEntries();
  } catch (error) {
    if (!preserveExisting || registrySummaries.value.length === 0) {
      registryError.value = errorMessage(error);
    }
  } finally {
    if (showLoading) {
      registryLoading.value = false;
    }
  }
}

async function ensureRegistryDetail(plugin: RegistryPluginSummary | RegistryPluginEntry): Promise<RegistryPluginEntry | null> {
  const existing = registryDetails.value[plugin.registryKey];
  if (existing) return existing;
  const source = registrySourceById(plugin.registrySourceId);
  const manifest = registryManifests.value[plugin.registrySourceId];
  if (!source || !manifest) return null;
  registryDetailLoadingId.value = plugin.registryKey;
  registryError.value = "";
  try {
    const detail = await pluginRegistryFetchPlugin({
      registryBaseUrl: registryBaseUrls.value[source.id],
      entryBasePath: manifest.entryBasePath,
      pluginId: plugin.id,
    });
    const sourcedDetail = withRegistrySource(detail, source);
    registryDetails.value = {
      ...registryDetails.value,
      [sourcedDetail.registryKey]: sourcedDetail,
    };
    return sourcedDetail;
  } catch (error) {
    registryError.value = errorMessage(error);
    return null;
  } finally {
    registryDetailLoadingId.value = "";
  }
}

async function ensureRegistryDescription(plugin: RegistryPluginEntry) {
  const cacheKey = registryDescriptionCacheKey(plugin);
  const descriptionSource = localizedRegistryDescriptionSource(plugin);
  if (!registryDescriptionSourceHasValue(descriptionSource) || registryDescriptions.value[cacheKey]) return;
  const pluginKey = plugin.registryKey;
  registryDescriptionLoadingId.value = pluginKey;
  registryDescriptionError.value = "";
  try {
    const result = await pluginRegistryFetchDescription({
      repo: plugin.repo,
      descriptionSource,
    });
    registryDescriptions.value = {
      ...registryDescriptions.value,
      [cacheKey]: result.content,
    };
  } catch (error) {
    if (selectedRegistryKey.value === pluginKey) {
      registryDescriptionError.value = errorMessage(error);
    }
  } finally {
    if (registryDescriptionLoadingId.value === pluginKey) {
      registryDescriptionLoadingId.value = "";
    }
  }
}

async function selectRegistryPlugin(plugin: RegistryPluginSummary) {
  pluginListMode.value = "registry";
  selectedRegistryKey.value = plugin.registryKey;
  registryConfigOpen.value = false;
  registryDescriptionError.value = "";
  const detail = await ensureRegistryDetail(plugin);
  if (detail) {
    void ensureRegistryDescription(detail);
    void ensureSelectedRegistryGithubStarStatus();
  }
}

function registryCompatibilityLabel(plugin: RegistryPluginSummary | RegistryPluginEntry): string {
  if (plugin.compatibility?.projectIndependent === false) {
    return t("plugin.dependency.projectDependent");
  }
  return t("plugin.dependency.independent");
}

function registryVersionLabel(plugin: RegistryPluginSummary | RegistryPluginEntry | null): string {
  return plugin?.latestVersion?.trim() || "0.0.0";
}

interface ParsedPluginVersion {
  numbers: number[];
  prerelease: string[];
}

function parsePluginVersion(value: string | null | undefined): ParsedPluginVersion | null {
  const normalized = (value ?? "").trim().replace(/^v(?=\d)/i, "");
  if (!normalized) return null;
  const [withoutBuild] = normalized.split("+", 1);
  const [core, ...prereleaseParts] = withoutBuild.split("-");
  const numberParts = core.split(".");
  if (numberParts.length === 0 || numberParts.some((part) => !/^\d+$/.test(part))) return null;
  return {
    numbers: numberParts.map((part) => Number(part)),
    prerelease: prereleaseParts.join("-").split(".").filter(Boolean),
  };
}

function comparePluginPrerelease(left: string[], right: string[]): number {
  if (left.length === 0 && right.length === 0) return 0;
  if (left.length === 0) return 1;
  if (right.length === 0) return -1;
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = left[index];
    const rightPart = right[index];
    if (leftPart === undefined) return -1;
    if (rightPart === undefined) return 1;
    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) {
      const diff = Number(leftPart) - Number(rightPart);
      if (diff !== 0) return diff;
      continue;
    }
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    const diff = leftPart.localeCompare(rightPart);
    if (diff !== 0) return diff;
  }
  return 0;
}

function comparePluginVersions(left: string | null | undefined, right: string | null | undefined): number | null {
  const leftVersion = parsePluginVersion(left);
  const rightVersion = parsePluginVersion(right);
  if (!leftVersion || !rightVersion) return null;
  const length = Math.max(leftVersion.numbers.length, rightVersion.numbers.length, 3);
  for (let index = 0; index < length; index += 1) {
    const diff = (leftVersion.numbers[index] ?? 0) - (rightVersion.numbers[index] ?? 0);
    if (diff !== 0) return diff;
  }
  return comparePluginPrerelease(leftVersion.prerelease, rightVersion.prerelease);
}

function registryVersionIsNewer(currentVersion: string | null | undefined, latestVersion: string | null | undefined): boolean {
  const diff = comparePluginVersions(latestVersion, currentVersion);
  return diff !== null && diff > 0;
}

function registryIconUrl(plugin: RegistryPluginSummary | RegistryPluginEntry | null): string {
  const icon = plugin?.icon;
  const type = icon?.type?.trim().toLowerCase();
  const hasUrlIcon = type === "url" || (!type && !!icon?.url?.trim());
  const url = hasUrlIcon ? icon?.url?.trim() : "";
  if (!url || !/^https?:\/\//i.test(url)) return "";
  return url;
}

function registryIconNode(plugin: RegistryPluginSummary | RegistryPluginEntry | null): IconNode {
  const icon = plugin?.icon;
  const type = icon?.type?.trim().toLowerCase();
  const hasLocusIcon = type === "locus" || (!type && !!icon?.id?.trim());
  const iconId = hasLocusIcon ? icon?.id : "";
  return resolveLocusViewIcon(iconId || "Package");
}

function registryIconAlt(plugin: RegistryPluginSummary | RegistryPluginEntry): string {
  return `${plugin.name || plugin.id} icon`;
}

function hideBrokenRegistryIcon(event: Event) {
  const image = event.currentTarget as HTMLImageElement | null;
  if (image) {
    image.style.display = "none";
  }
}

function isGithubRepoPart(value: string | undefined): boolean {
  return !!value
    && value.length <= 100
    && !value.startsWith(".")
    && !value.endsWith(".")
    && /^[A-Za-z0-9_.-]+$/.test(value);
}

function registryGithubRepoKey(plugin: RegistryPluginEntry | null): string {
  const raw = plugin?.repo?.trim().replace(/\.git$/i, "") ?? "";
  if (!raw) return "";
  let path = raw;
  if (/^https?:\/\//i.test(raw)) {
    try {
      const url = new URL(raw);
      if (url.hostname.toLowerCase() !== "github.com") return "";
      path = url.pathname;
    } catch {
      return "";
    }
  }
  const [owner, repo] = path
    .trim()
    .replace(/^\/+|\/+$/g, "")
    .split("/")
    .filter(Boolean)
    .map((part) => part.replace(/\.git$/i, ""));
  if (!isGithubRepoPart(owner) || !isGithubRepoPart(repo)) return "";
  return `${owner}/${repo}`;
}

function registryRepoUrl(plugin: RegistryPluginEntry): string {
  const repo = plugin.repo.trim();
  if (!repo) return "";
  if (/^https?:\/\//i.test(repo)) return repo;
  return `https://github.com/${repo}`;
}

async function openRegistryRepo(plugin: RegistryPluginEntry) {
  const url = registryRepoUrl(plugin);
  if (!url) return;
  try {
    await openUrl(url);
  } catch (error) {
    notificationStore.addNotice("error", errorMessage(error), { operation: "pluginOpenRepo" });
  }
}

async function refreshAll() {
  loading.value = true;
  loadError.value = "";
  try {
    installedPlugins.value = await pluginListInstalled();
    void loadInstalledRegistryEntries();
    if (
      selectedPluginKey.value
      && !installedPlugins.value.some((plugin) => selectedPluginIdentity(plugin) === selectedPluginKey.value)
    ) {
      selectedPluginKey.value = "";
    }
  } catch (error) {
    const message = errorMessage(error);
    loadError.value = message;
    notificationStore.addNotice("error", message, { operation: "pluginRefresh" });
  } finally {
    loading.value = false;
  }
}

function pluginScopeLabel(scope: PluginInstallScope | string): string {
  return scope === "project" ? t("plugin.scope.project") : t("plugin.scope.app");
}

function pluginScopeSortValue(scope: PluginInstallScope | string): number {
  if (scope === "project") return 0;
  if (scope === "app") return 1;
  return 2;
}

function pluginScopeTag(scope: PluginInstallScope | string): string {
  return scope === "project" ? "PRJ" : "APP";
}

type InstalledRegistryDisplay = RegistryPluginEntry | RegistryPluginSummary;

function registryDisplaySortKey(plugin: InstalledRegistryDisplay): string {
  return [
    plugin.registrySourceLabel,
    plugin.registrySourceId,
    plugin.registryKey,
  ].join("\n");
}

function installedRegistryDisplays(plugin: InstalledPluginSummary): InstalledRegistryDisplay[] {
  const byKey = new Map<string, InstalledRegistryDisplay>();
  for (const entry of Object.values(registryDetails.value)) {
    if (entry.id === plugin.id) {
      byKey.set(entry.registryKey, entry);
    }
  }
  for (const summary of registrySummaries.value) {
    if (summary.id === plugin.id && !byKey.has(summary.registryKey)) {
      byKey.set(summary.registryKey, summary);
    }
  }
  return Array.from(byKey.values()).sort((left, right) =>
    registryDisplaySortKey(left).localeCompare(registryDisplaySortKey(right)),
  );
}

function installedRegistryDisplay(plugin: InstalledPluginSummary): RegistryPluginEntry | RegistryPluginSummary | null {
  return installedRegistryDisplays(plugin)[0] ?? null;
}

function installedListDisplayName(plugin: InstalledPluginSummary): string {
  const registryDisplay = installedRegistryDisplay(plugin);
  return registryDisplay?.name || plugin.name || plugin.id;
}

function installedListVersion(plugin: InstalledPluginSummary): string {
  const registryDisplay = installedRegistryDisplay(plugin);
  return plugin.version?.trim() || registryVersionLabel(registryDisplay);
}

function installedListSummary(plugin: InstalledPluginSummary): string {
  const registryDisplay = installedRegistryDisplay(plugin);
  return localizedRegistrySummary(registryDisplay) || plugin.id;
}

function installedListAuthor(plugin: InstalledPluginSummary): string {
  return installedRegistryDisplay(plugin)?.author?.trim() || "";
}

function installedListSourceLabel(plugin: InstalledPluginSummary): string {
  return installedRegistryDisplay(plugin)?.registrySourceLabel || "";
}

function shouldShowInstalledRegistrySource(plugin: InstalledPluginSummary): boolean {
  const registryDisplay = installedRegistryDisplay(plugin);
  return registryDisplay ? shouldShowRegistrySource(registryDisplay) : false;
}

function installedListIconUrl(plugin: InstalledPluginSummary): string {
  return registryIconUrl(installedRegistryDisplay(plugin));
}

function installedListIconNode(plugin: InstalledPluginSummary): IconNode {
  const registryDisplay = installedRegistryDisplay(plugin);
  return registryDisplay ? registryIconNode(registryDisplay) : Package;
}

function installedListIconAlt(plugin: InstalledPluginSummary): string {
  return `${installedListDisplayName(plugin)} icon`;
}

function installedRegistryPlugins(plugin: RegistryPluginEntry | RegistryPluginSummary): InstalledPluginSummary[] {
  return installedPluginById.value.get(plugin.id) ?? [];
}

function installedRegistryPlugin(plugin: RegistryPluginEntry | RegistryPluginSummary): InstalledPluginSummary | null {
  const installed = installedRegistryPlugins(plugin);
  return installed.length === 1 ? installed[0] : null;
}

function registryPluginIsInstalled(plugin: RegistryPluginEntry | RegistryPluginSummary): boolean {
  return installedPluginById.value.has(plugin.id);
}

function registryPluginHasUpdate(
  installed: InstalledPluginSummary | null,
  registryDisplay: RegistryPluginEntry | RegistryPluginSummary | null,
): boolean {
  if (!installed || !registryDisplay) return false;
  return registryVersionIsNewer(installed.version, registryVersionLabel(registryDisplay));
}

function registryPluginUpdateAvailable(plugin: RegistryPluginEntry | RegistryPluginSummary): boolean {
  return installedRegistryPlugins(plugin).some((installed) => registryPluginHasUpdate(installed, plugin));
}

function installedUpdateCandidate(plugin: InstalledPluginSummary): RegistryPluginEntry | RegistryPluginSummary | null {
  const candidates = installedRegistryDisplays(plugin).filter((registryDisplay) =>
    registryPluginHasUpdate(plugin, registryDisplay),
  );
  return candidates.length === 1 ? candidates[0] : null;
}

function installedLatestVersionLabel(plugin: InstalledPluginSummary): string {
  return registryVersionLabel(installedUpdateCandidate(plugin));
}

function shouldShowRegistryInstallDivider(plugin: RegistryPluginSummary, index: number): boolean {
  if (index <= 0 || registryPluginIsInstalled(plugin)) return false;
  const previous = visibleRegistrySummaries.value[index - 1];
  return !!previous && registryPluginIsInstalled(previous);
}

function installedRegistryScopeTag(plugin: RegistryPluginEntry | RegistryPluginSummary): string {
  return installedRegistryPlugins(plugin)
    .map((installed) => installed.scope)
    .sort((left, right) => pluginScopeSortValue(left) - pluginScopeSortValue(right))
    .map(pluginScopeTag)
    .join("/");
}

function installedRegistryPluginKey(plugin: RegistryPluginEntry | RegistryPluginSummary): string {
  const installed = installedRegistryPlugin(plugin);
  return installed ? selectedPluginIdentity(installed) : "";
}

function registryStatValue(stat: PluginRegistryStat): string {
  const value = stat.value;
  if (value === null || value === undefined) return "";
  if (typeof value === "number") {
    return new Intl.NumberFormat(undefined, {
      notation: "compact",
      maximumFractionDigits: 1,
    }).format(value);
  }
  return String(value).trim();
}

function registryStatTitle(stat: PluginRegistryStat): string {
  const label = stat.label?.trim() || stat.id?.trim() || "";
  const value = registryStatValue(stat);
  return label && value ? `${label}: ${value}` : label || value;
}

function registryStatIconNode(stat: PluginRegistryStat): IconNode {
  const icon = stat.icon;
  const type = icon?.type?.trim().toLowerCase();
  const iconId = type === "locus" || (!type && !!icon?.id?.trim()) ? icon?.id : "";
  if (iconId?.trim()) return resolveLocusViewIcon(iconId);
  const statId = normalizeSearch(`${stat.id ?? ""} ${stat.label ?? ""}`);
  return statId.includes("star") || statId.includes("github") ? Star : Store;
}

function registryStatIsGithubStar(stat: PluginRegistryStat): boolean {
  const statId = normalizeSearch(`${stat.id ?? ""} ${stat.label ?? ""}`);
  return statId.includes("star") || statId.includes("stargazer");
}

function upsertRegistryStarStat(
  stats: PluginRegistryStat[] | undefined,
  count: number,
): PluginRegistryStat[] {
  const nextStats = [...(stats ?? [])];
  const starIndex = nextStats.findIndex(registryStatIsGithubStar);
  const nextStat: PluginRegistryStat = {
    ...(starIndex >= 0 ? nextStats[starIndex] : { id: "stars", label: "Stars", icon: { type: "locus", id: "Star" } }),
    value: count,
  };
  if (starIndex >= 0) {
    nextStats[starIndex] = nextStat;
  } else {
    nextStats.unshift(nextStat);
  }
  return nextStats;
}

function updateRegistryStarCount(registryKey: string, count: number | null | undefined) {
  if (!registryKey || count === null || count === undefined) return;
  registrySummaries.value = registrySummaries.value.map((plugin) =>
    plugin.registryKey === registryKey
      ? { ...plugin, stats: upsertRegistryStarStat(plugin.stats, count) }
      : plugin,
  );
  const detail = registryDetails.value[registryKey];
  if (detail) {
    registryDetails.value = {
      ...registryDetails.value,
      [registryKey]: {
        ...detail,
        stats: upsertRegistryStarStat(detail.stats, count),
      },
    };
  }
}

function registryStarCount(plugin: RegistryPluginEntry | RegistryPluginSummary | null | undefined): number | null {
  const value = plugin?.stats?.find(registryStatIsGithubStar)?.value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  if (!/^\d+$/.test(normalized)) return null;
  return Number(normalized);
}

function updateRegistryStarCountByDelta(registryKey: string, delta: number) {
  if (!registryKey || delta === 0) return;
  const plugin = registrySummaries.value.find((entry) => entry.registryKey === registryKey)
    ?? registryDetails.value[registryKey]
    ?? null;
  const count = registryStarCount(plugin);
  if (count === null) return;
  updateRegistryStarCount(registryKey, Math.max(0, count + delta));
}

function syncGithubStarStatus(
  status: PluginGithubRepoStarStatus,
  registryKey = selectedRegistryKey.value,
  starCountDelta = 0,
) {
  githubStarStatusByRepo.value = {
    ...githubStarStatusByRepo.value,
    [status.repo]: status,
  };
  if (status.stargazersCount !== null && status.stargazersCount !== undefined) {
    updateRegistryStarCount(registryKey, status.stargazersCount);
  } else {
    updateRegistryStarCountByDelta(registryKey, starCountDelta);
  }
}

async function refreshSelectedRegistryGithubStarStatus(options: { notify?: boolean } = {}) {
  const repo = selectedRegistryGithubRepo.value;
  const registryKey = selectedRegistryKey.value;
  if (!githubAuthStatus.value.authenticated || !repo || githubStarLoadingRepo.value === repo) return;
  githubStarLoadingRepo.value = repo;
  githubStarError.value = "";
  try {
    const status = await pluginGithubRepoStarStatus(repo);
    syncGithubStarStatus(status, registryKey);
  } catch (error) {
    const message = errorMessage(error);
    if (selectedRegistryGithubRepo.value === repo) {
      githubStarError.value = message;
    }
    if (options.notify) {
      notificationStore.addNotice("error", message, { operation: "pluginGithubStarStatus" });
    }
  } finally {
    if (githubStarLoadingRepo.value === repo) {
      githubStarLoadingRepo.value = "";
    }
  }
}

async function ensureSelectedRegistryGithubStarStatus() {
  const repo = selectedRegistryGithubRepo.value;
  if (!githubAuthStatus.value.authenticated || !repo || githubStarStatusByRepo.value[repo]) return;
  await refreshSelectedRegistryGithubStarStatus();
}

async function toggleSelectedRegistryGithubStar() {
  const repo = selectedRegistryGithubRepo.value;
  const registryKey = selectedRegistryKey.value;
  const pluginName = selectedRegistryDisplay.value?.name || selectedRegistryDisplay.value?.id || repo;
  if (!githubAuthStatus.value.authenticated || !repo || githubStarSavingRepo.value === repo) return;
  if (!githubStarStatusByRepo.value[repo]) {
    await refreshSelectedRegistryGithubStarStatus({ notify: true });
  }
  const currentStatus = githubStarStatusByRepo.value[repo];
  if (!currentStatus) return;
  const nextStarred = !currentStatus.starred;
  githubStarSavingRepo.value = repo;
  githubStarError.value = "";
  try {
    const status = await pluginGithubRepoSetStarred(repo, nextStarred);
    const starCountDelta = status.starred === currentStatus.starred ? 0 : status.starred ? 1 : -1;
    syncGithubStarStatus(status, registryKey, starCountDelta);
    notificationStore.addNotice(
      "success",
      t(nextStarred ? "plugin.notice.starred" : "plugin.notice.unstarred", pluginName),
      { operation: "pluginGithubStar" },
    );
  } catch (error) {
    const message = errorMessage(error);
    githubStarError.value = message;
    notificationStore.addNotice("error", message, { operation: "pluginGithubStar" });
  } finally {
    if (githubStarSavingRepo.value === repo) {
      githubStarSavingRepo.value = "";
    }
  }
}

async function installRegistryPluginWithScopes(
  plugin: RegistryPluginSummary | RegistryPluginEntry,
  scopes: PluginInstallScope[],
  operation: "install" | "update",
) {
  if (registryInstallKey.value) return;
  const uniqueScopes = Array.from(new Set(scopes)).sort(
    (left, right) => pluginScopeSortValue(left) - pluginScopeSortValue(right),
  );
  if (uniqueScopes.length === 0) return;
  registryInstallKey.value = plugin.registryKey;
  try {
    const detail = await ensureRegistryDetail(plugin);
    if (!detail) return;
    let installed: InstalledPluginSummary | null = null;
    for (const scope of uniqueScopes) {
      installed = await pluginInstallFromRegistry({
        id: detail.id,
        latestVersion: detail.latestVersion,
        download: detail.download ?? {},
        downloadSource: detail.downloadSource
          ? { ...detail.downloadSource, repo: detail.downloadSource.repo || detail.repo }
          : {},
      }, scope);
    }
    const installedLabel = [
      installed?.name || detail.name || detail.id,
      uniqueScopes.length > 1 ? `(${uniqueScopes.map(pluginScopeTag).join("/")})` : "",
    ].filter(Boolean).join(" ");
    notificationStore.addNotice(
      "success",
      t(operation === "update" ? "plugin.notice.updated" : "plugin.notice.installed", installedLabel),
      { operation: operation === "update" ? "pluginRegistryUpdate" : "pluginRegistryInstall" },
    );
    await refreshAll();
    void loadInstalledRegistryEntries();
  } catch (error) {
    notificationStore.addNotice(
      "error",
      errorMessage(error),
      { operation: operation === "update" ? "pluginRegistryUpdate" : "pluginRegistryInstall" },
    );
  } finally {
    registryInstallKey.value = "";
  }
}

async function installRegistryPluginWithScope(
  plugin: RegistryPluginSummary | RegistryPluginEntry,
  scope: PluginInstallScope,
  operation: "install" | "update",
) {
  await installRegistryPluginWithScopes(plugin, [scope], operation);
}

async function installRegistryPlugin(plugin: RegistryPluginSummary | RegistryPluginEntry) {
  await installRegistryPluginWithScope(plugin, REGISTRY_INSTALL_SCOPE, "install");
}

async function updateRegistryPlugin(plugin: RegistryPluginSummary | RegistryPluginEntry) {
  const outdated = installedRegistryPlugins(plugin).filter((installed) =>
    registryPluginHasUpdate(installed, plugin),
  );
  if (!outdated.length) return;
  await installRegistryPluginWithScopes(plugin, outdated.map((installed) => installed.scope), "update");
}

async function updateInstalledPlugin(plugin: InstalledPluginSummary) {
  const candidate = installedUpdateCandidate(plugin);
  if (!candidate) return;
  await installRegistryPluginWithScope(candidate, plugin.scope, "update");
}

async function uninstallRegistryPlugin(plugin: RegistryPluginSummary | RegistryPluginEntry) {
  const installed = installedRegistryPlugin(plugin);
  if (!installed) return;
  await uninstallPlugin(installed);
}

function pluginDependencyCount(plugin: InstalledPluginSummary): number {
  return plugin.dependencies?.project?.length ?? 0;
}

function pluginDependencyLabel(plugin: InstalledPluginSummary): string {
  const count = pluginDependencyCount(plugin);
  if (count > 0) return t("plugin.dependency.count", count);
  if (plugin.compatibility?.projectIndependent === false) {
    return t("plugin.dependency.projectDependent");
  }
  return t("plugin.dependency.independent");
}

async function uninstallPlugin(plugin: InstalledPluginSummary) {
  const key = `${plugin.scope}:${plugin.id}`;
  if (uninstallConfirmKey.value !== key) {
    uninstallConfirmKey.value = key;
    return;
  }
  uninstallKey.value = key;
  try {
    await pluginUninstall(plugin.id, plugin.scope);
    notificationStore.addNotice(
      "success",
      t("plugin.notice.uninstalled", plugin.name || plugin.id),
      { operation: "pluginUninstall" },
    );
    await refreshAll();
  } catch (error) {
    notificationStore.addNotice("error", errorMessage(error), { operation: "pluginUninstall" });
  } finally {
    uninstallKey.value = "";
    uninstallConfirmKey.value = "";
  }
}

function formatComponentSummary(plugin: InstalledPluginSummary): string {
  const ruleCount = plugin.rules?.length ?? 0;
  return [
    plugin.agents.length ? t("plugin.component.agents", plugin.agents.length) : "",
    ruleCount ? t("plugin.component.rules", ruleCount) : "",
    plugin.skills.length ? t("plugin.component.skills", plugin.skills.length) : "",
    plugin.views.length ? t("plugin.component.views", plugin.views.length) : "",
  ].filter(Boolean).join(" / ") || t("common.none");
}

watch(() => props.workingDir, () => {
  void refreshAll();
});

watch(pluginSearch, (value) => {
  resetRegistryVisibleCount();
  if (!value.trim() || pluginListMode.value !== "registry") return;
  void loadRegistrySearchIndexes();
  if (filteredRegistrySummaries.value.length === 0 && hasMoreRegistryBuckets.value) {
    void loadNextRegistryPage();
  }
});

watch(locale, () => {
  registryDescriptionError.value = "";
  const detail = selectedRegistryDetail.value;
  if (detail) {
    void ensureRegistryDescription(detail);
  }
});

watch([selectedRegistryGithubRepo, () => githubAuthStatus.value.authenticated], ([repo, authenticated]) => {
  githubStarError.value = "";
  if (!authenticated) {
    githubStarStatusByRepo.value = {};
    return;
  }
  if (repo) {
    void ensureSelectedRegistryGithubStarStatus();
  }
});

onMounted(async () => {
  loadRegistrySourceSettings();
  void refreshGithubAuthStatus();
  await refreshAll();
  await refreshRegistry({ cacheMode: "cachePreferred", silent: true });
  void refreshRegistry({
    cacheMode: "networkPreferred",
    preserveExisting: true,
    silent: true,
  });
  if (!hasTauriWindowRuntime()) return;
  try {
    unlistenPluginsChanged = await listen("plugins-changed", () => {
      void refreshAll();
    });
    unlistenKnowledgeChanged = await listen("knowledge-changed", () => {
      void refreshAll();
    });
    unlistenViewTreeChanged = await listen("view-tree-changed", () => {
      void refreshAll();
    });
  } catch (error) {
    console.warn("Failed to listen for plugin view refresh events:", error);
  }
});

onUnmounted(() => {
  stopGithubOAuthPoll();
  unlistenPluginsChanged?.();
  unlistenKnowledgeChanged?.();
  unlistenViewTreeChanged?.();
});
</script>

<template>
  <div
    ref="pluginLayoutRef"
    class="plugin-view"
    :class="{ 'is-resizing-installed-pane': resizingInstalledPane }"
  >
    <aside
      class="plugin-pane plugin-list-pane"
      :style="{ width: `${installedPaneWidth}px` }"
      @contextmenu="openPluginListContextMenu"
    >
      <header class="plugin-pane-header plugin-list-header">
        <div class="plugin-list-titlebar">
          <div class="plugin-pane-title">
            <LucideIcon :icon="Package" :size="15" />
            <span>{{ t("app.tab.plugins") }}</span>
          </div>
          <div class="plugin-titlebar-actions">
            <BaseButton
              class="plugin-icon-button"
              :title="t('plugin.help.label')"
              :aria-label="t('plugin.help.label')"
              :aria-expanded="pluginHelpOpen"
              @click="openPluginHelp"
            >
              <LucideIcon :icon="HelpCircle" :size="13" />
            </BaseButton>
            <BaseButton
              class="plugin-github-login-button"
              :class="{ 'is-authenticated': githubAuthStatus.authenticated }"
              :title="githubAuthStatus.authenticated ? t('plugin.hub.githubAuthenticated', githubAuthStatus.account || 'GitHub') : t('plugin.hub.githubLogin')"
              @click="openGithubOAuth"
            >
              <LucideIcon :icon="githubAuthStatus.authenticated ? UserCheck : KeyRound" :size="13" />
              <span>{{ githubAuthStatus.authenticated ? t("plugin.hub.githubLoggedIn") : t("plugin.hub.githubLogin") }}</span>
            </BaseButton>
            <BaseButton
              class="plugin-icon-button"
              :title="t('plugin.hub.registryConfig')"
              @click="toggleRegistryConfig"
            >
              <LucideIcon :icon="Settings2" :size="13" />
            </BaseButton>
          </div>
        </div>
      </header>

      <div class="plugin-list-tools">
        <div class="plugin-list-tool-row">
          <div class="plugin-hub-search">
            <LucideIcon :icon="Search" :size="14" />
            <input
              v-model="pluginSearch"
              class="plugin-hub-search-input"
              :placeholder="t('plugin.hub.searchPlaceholder')"
            />
          </div>
          <BaseButton class="plugin-icon-button" :title="pluginListModeToggleTitle" @click="togglePluginListMode">
            <LucideIcon :icon="pluginListModeToggleIcon" :size="13" />
          </BaseButton>
        </div>
      </div>

      <div class="plugin-list-content">
        <template v-if="pluginListMode === 'registry'">
          <div v-if="registryError" class="plugin-error">{{ registryError }}</div>
          <div v-else-if="registryLoading && registrySummaries.length === 0" class="plugin-empty">
            {{ t("plugin.hub.loading") }}
          </div>
          <div v-else-if="!registryLoading && filteredRegistrySummaries.length === 0 && !hasMoreRegistryBuckets" class="plugin-empty">
            {{ pluginSearch.trim() ? t("plugin.hub.emptySearch") : t("plugin.hub.empty") }}
          </div>
          <div v-else class="plugin-list" @scroll.passive="handleRegistryListScroll">
            <template v-for="(plugin, index) in visibleRegistrySummaries" :key="plugin.registryKey">
              <div
                v-if="shouldShowRegistryInstallDivider(plugin, index)"
                class="plugin-registry-section-divider"
              >
                <span>{{ t("plugin.hub.uninstalled") }}</span>
              </div>
              <div
                class="plugin-list-item plugin-registry-list-item"
                :class="{ active: selectedRegistryKey === plugin.registryKey }"
              >
                <button
                  type="button"
                  class="plugin-list-select-button"
                  @click="selectRegistryPlugin(plugin)"
                >
                  <div class="plugin-hub-icon-frame plugin-list-icon">
                    <LucideIcon :icon="registryIconNode(plugin)" :size="20" />
                    <img
                      v-if="registryIconUrl(plugin)"
                      class="plugin-hub-icon-image"
                      :src="registryIconUrl(plugin)"
                      :alt="registryIconAlt(plugin)"
                      referrerpolicy="no-referrer"
                      @error="hideBrokenRegistryIcon"
                    />
                  </div>
                  <div class="plugin-list-main">
                    <div class="plugin-list-title">
                      <span class="plugin-name">{{ plugin.name || plugin.id }}</span>
                      <span class="plugin-version">{{ registryVersionLabel(plugin) }}</span>
                      <span v-if="installedRegistryScopeTag(plugin)" class="plugin-scope-tag">{{ installedRegistryScopeTag(plugin) }}</span>
                    </div>
                    <div class="plugin-list-summary">{{ localizedRegistrySummary(plugin) || plugin.id }}</div>
                    <div class="plugin-list-meta">
                      <span v-if="shouldShowRegistrySource(plugin)">{{ plugin.registrySourceLabel }}</span>
                      <span v-if="plugin.author">{{ plugin.author }}</span>
                    </div>
                  </div>
                </button>
                <div class="plugin-list-side">
                  <div v-if="plugin.stats?.length" class="plugin-list-stats">
                    <span
                      v-for="stat in plugin.stats"
                      :key="stat.id || stat.label || registryStatValue(stat)"
                      class="plugin-list-stat"
                      :title="registryStatTitle(stat)"
                    >
                      <LucideIcon :icon="registryStatIconNode(stat)" :size="12" />
                      <span>{{ registryStatValue(stat) }}</span>
                    </span>
                  </div>
                  <BaseButton
                    v-if="registryPluginUpdateAvailable(plugin)"
                    class="plugin-registry-action-button is-update-action"
                    :disabled="registryInstallKey === plugin.registryKey"
                    @click.stop="updateRegistryPlugin(plugin)"
                  >
                    {{ registryInstallKey === plugin.registryKey ? t("plugin.hub.updating") : t("plugin.hub.update") }}
                  </BaseButton>
                  <BaseButton
                    v-else-if="installedRegistryPlugin(plugin)"
                    class="plugin-registry-action-button is-uninstall-action"
                    :class="{ 'is-confirming': uninstallConfirmKey === installedRegistryPluginKey(plugin) }"
                    :disabled="uninstallKey === installedRegistryPluginKey(plugin)"
                    @click.stop="uninstallRegistryPlugin(plugin)"
                  >
                    {{ uninstallConfirmKey === installedRegistryPluginKey(plugin) ? t("common.confirm") : t("plugin.hub.uninstall") }}
                  </BaseButton>
                  <BaseButton
                    v-else-if="!registryPluginIsInstalled(plugin)"
                    class="plugin-registry-action-button is-install-action"
                    :disabled="registryInstallKey === plugin.registryKey"
                    @click.stop="installRegistryPlugin(plugin)"
                  >
                    {{ registryInstallKey === plugin.registryKey ? t("plugin.hub.installing") : t("plugin.hub.install") }}
                  </BaseButton>
                </div>
              </div>
            </template>
            <div v-if="hasMoreRegistryListItems || registryLoadingMore" class="plugin-list-footer">
              <BaseButton
                class="plugin-hub-load-more"
                :disabled="registryLoadingMore"
                @click="loadNextRegistryPage()"
              >
                {{ registryLoadingMore ? t("common.loading") : t("plugin.hub.loadMore") }}
              </BaseButton>
            </div>
          </div>
        </template>

        <template v-else>
          <div v-if="loadError" class="plugin-error">{{ loadError }}</div>
          <div v-else-if="loading && installedSorted.length === 0" class="plugin-empty">
            {{ t("common.loading") }}
          </div>
          <div v-else-if="installedSorted.length === 0" class="plugin-empty">
            {{ t("plugin.installed.empty") }}
          </div>
          <div v-else-if="filteredInstalledPlugins.length === 0" class="plugin-empty">
            {{ t("plugin.hub.emptySearch") }}
          </div>
          <div v-else class="plugin-list">
            <div
              v-for="plugin in filteredInstalledPlugins"
              :key="`${plugin.scope}:${plugin.id}`"
              class="plugin-list-item plugin-registry-list-item plugin-installed-list-item"
              :class="{ active: selectedPluginKey === `${plugin.scope}:${plugin.id}` }"
            >
              <button
                type="button"
                class="plugin-list-select-button"
                @click="selectInstalledPlugin(plugin)"
              >
                <div class="plugin-hub-icon-frame plugin-list-icon">
                  <LucideIcon :icon="installedListIconNode(plugin)" :size="20" />
                  <img
                    v-if="installedListIconUrl(plugin)"
                    class="plugin-hub-icon-image"
                    :src="installedListIconUrl(plugin)"
                    :alt="installedListIconAlt(plugin)"
                    referrerpolicy="no-referrer"
                    @error="hideBrokenRegistryIcon"
                  />
                </div>
                <div class="plugin-list-main">
                  <div class="plugin-list-title">
                    <span class="plugin-name">{{ installedListDisplayName(plugin) }}</span>
                    <span class="plugin-version">{{ installedListVersion(plugin) }}</span>
                    <span class="plugin-scope-tag">{{ pluginScopeTag(plugin.scope) }}</span>
                  </div>
                  <div class="plugin-list-summary">{{ installedListSummary(plugin) }}</div>
                  <div class="plugin-list-meta">
                    <span v-if="shouldShowInstalledRegistrySource(plugin)">{{ installedListSourceLabel(plugin) }}</span>
                    <span v-if="installedListAuthor(plugin)">{{ installedListAuthor(plugin) }}</span>
                  </div>
                </div>
              </button>
              <div class="plugin-list-side">
                <BaseButton
                  v-if="installedUpdateCandidate(plugin)"
                  class="plugin-registry-action-button is-update-action"
                  :disabled="registryInstallKey === installedUpdateCandidate(plugin)?.registryKey"
                  @click.stop="updateInstalledPlugin(plugin)"
                >
                  {{ registryInstallKey === installedUpdateCandidate(plugin)?.registryKey ? t("plugin.hub.updating") : t("plugin.hub.update") }}
                </BaseButton>
                <BaseButton
                  v-else
                  class="plugin-registry-action-button is-uninstall-action"
                  :class="{ 'is-confirming': uninstallConfirmKey === `${plugin.scope}:${plugin.id}` }"
                  :disabled="uninstallKey === `${plugin.scope}:${plugin.id}`"
                  @click.stop="uninstallPlugin(plugin)"
                >
                  {{ uninstallConfirmKey === `${plugin.scope}:${plugin.id}` ? t("common.confirm") : t("plugin.hub.uninstall") }}
                </BaseButton>
              </div>
            </div>
          </div>
        </template>
      </div>
    </aside>

    <div
      class="plugin-resize-handle"
      role="separator"
      aria-orientation="vertical"
      @mousedown="onInstalledPaneResizeMouseDown"
    />

    <main class="plugin-pane plugin-detail-pane">
      <div v-if="pluginListMode === 'registry' && selectedRegistryDisplay" class="plugin-detail-view">
        <header class="plugin-detail-hero">
          <div class="plugin-detail-hero-main">
            <div class="plugin-hub-icon-frame plugin-detail-hero-icon">
              <LucideIcon :icon="registryIconNode(selectedRegistryDisplay)" :size="32" />
              <img
                v-if="registryIconUrl(selectedRegistryDisplay)"
                class="plugin-hub-icon-image"
                :src="registryIconUrl(selectedRegistryDisplay)"
                :alt="registryIconAlt(selectedRegistryDisplay)"
                referrerpolicy="no-referrer"
                @error="hideBrokenRegistryIcon"
              />
            </div>
            <div class="plugin-detail-heading">
              <div class="plugin-detail-title">{{ selectedRegistryDisplay.name || selectedRegistryDisplay.id }}</div>
              <div class="plugin-detail-id">{{ selectedRegistryDisplay.id }}</div>
              <div class="plugin-detail-summary">{{ localizedRegistrySummary(selectedRegistryDisplay) || selectedRegistryDisplay.id }}</div>
            </div>
          </div>
          <div class="plugin-detail-actions">
            <BaseButton
              v-if="githubAuthStatus.authenticated && selectedRegistryGithubRepo"
              class="plugin-star-button"
              :class="{ 'is-starred': selectedRegistryStarStatus?.starred }"
              :disabled="selectedRegistryStarBusy"
              :title="githubStarError || (selectedRegistryStarStatus?.starred ? t('plugin.hub.unstar') : t('plugin.hub.star'))"
              @click="toggleSelectedRegistryGithubStar"
            >
              <LucideIcon
                :class="{ 'plugin-github-oauth-spin': selectedRegistryStarBusy }"
                :icon="selectedRegistryStarBusy ? LoaderCircle : Star"
                :size="13"
              />
              {{ selectedRegistryStarBusy ? t("common.loading") : selectedRegistryStarStatus?.starred ? t("plugin.hub.starred") : t("plugin.hub.star") }}
            </BaseButton>
            <BaseButton
              v-if="selectedRegistryDetail && registryRepoUrl(selectedRegistryDetail)"
              @click="openRegistryRepo(selectedRegistryDetail)"
            >
              <LucideIcon :icon="ExternalLink" :size="13" />
              {{ t("plugin.hub.openRepo") }}
            </BaseButton>
            <BaseButton
              v-if="registryPluginUpdateAvailable(selectedRegistryDisplay)"
              :disabled="registryInstallKey === selectedRegistryDisplay.registryKey"
              @click="updateRegistryPlugin(selectedRegistryDisplay)"
            >
              <LucideIcon :icon="RefreshCw" :size="13" />
              {{ registryInstallKey === selectedRegistryDisplay.registryKey ? t("plugin.hub.updating") : t("plugin.hub.update") }}
            </BaseButton>
            <BaseButton
              v-else-if="!registryPluginIsInstalled(selectedRegistryDisplay)"
              :disabled="registryInstallKey === selectedRegistryDisplay.registryKey"
              @click="installRegistryPlugin(selectedRegistryDisplay)"
            >
              <LucideIcon :icon="PackageCheck" :size="13" />
              {{ registryInstallKey === selectedRegistryDisplay.registryKey ? t("plugin.hub.installing") : t("plugin.hub.install") }}
            </BaseButton>
            <BaseButton
              v-if="installedRegistryPlugin(selectedRegistryDisplay)"
              variant="danger"
              :disabled="uninstallKey === installedRegistryPluginKey(selectedRegistryDisplay)"
              @click="uninstallRegistryPlugin(selectedRegistryDisplay)"
            >
              <LucideIcon :icon="Trash2" :size="13" />
              {{ uninstallConfirmKey === installedRegistryPluginKey(selectedRegistryDisplay) ? t("common.confirm") : t("plugin.hub.uninstall") }}
            </BaseButton>
          </div>
        </header>

        <section class="plugin-detail-meta">
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.author") }}</span>
            <strong>{{ selectedRegistryDisplay.author || "—" }}</strong>
          </div>
          <div v-if="shouldShowRegistrySource(selectedRegistryDisplay)" class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.registry") }}</span>
            <strong>{{ selectedRegistryDisplay.registrySourceLabel }}</strong>
          </div>
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.export.version") }}</span>
            <strong>{{ registryVersionLabel(selectedRegistryDisplay) }}</strong>
          </div>
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.compatibility") }}</span>
            <strong>{{ registryCompatibilityLabel(selectedRegistryDisplay) }}</strong>
          </div>
          <div v-if="selectedRegistryDetail?.license" class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.license") }}</span>
            <strong>{{ selectedRegistryDetail.license }}</strong>
          </div>
          <div v-if="selectedRegistryDisplay.compatibility?.minLocusVersion" class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.minVersion") }}</span>
            <strong>{{ t("plugin.hub.minLocusVersion", selectedRegistryDisplay.compatibility.minLocusVersion) }}</strong>
          </div>
          <div v-if="selectedRegistryDisplay.tags?.length" class="plugin-detail-meta-item is-wide">
            <span>{{ t("plugin.hub.tags") }}</span>
            <strong>{{ selectedRegistryDisplay.tags.join(" / ") }}</strong>
          </div>
        </section>

        <section class="plugin-detail-scroll">
          <div v-if="registryDescriptionLoadingId === selectedRegistryDisplay.registryKey" class="plugin-empty compact">
            {{ t("common.loading") }}
          </div>
          <div v-if="registryDescriptionError" class="plugin-error compact">
            {{ registryDescriptionError }}
          </div>
          <div v-if="registryDetailLoadingId === selectedRegistryDisplay.registryKey" class="plugin-empty compact">
            {{ t("common.loading") }}
          </div>
          <MarkdownRenderer :content="selectedRegistryDescriptionContent" />
        </section>
      </div>

      <div v-else-if="pluginListMode === 'installed' && selectedInstalledPlugin" class="plugin-detail-view">
        <header class="plugin-detail-hero">
          <div class="plugin-detail-hero-main">
            <div class="plugin-detail-hero-icon plugin-local-icon">
              <LucideIcon :icon="Package" :size="31" />
            </div>
            <div class="plugin-detail-heading">
              <div class="plugin-detail-title">{{ selectedInstalledPlugin.name || selectedInstalledPlugin.id }}</div>
              <div class="plugin-detail-id">{{ selectedInstalledPlugin.id }}</div>
            </div>
          </div>
          <div class="plugin-detail-actions">
            <BaseButton
              v-if="installedUpdateCandidate(selectedInstalledPlugin)"
              :disabled="registryInstallKey === installedUpdateCandidate(selectedInstalledPlugin)?.registryKey"
              @click="updateInstalledPlugin(selectedInstalledPlugin)"
            >
              <LucideIcon :icon="RefreshCw" :size="13" />
              {{ registryInstallKey === installedUpdateCandidate(selectedInstalledPlugin)?.registryKey ? t("plugin.hub.updating") : t("plugin.hub.update") }}
            </BaseButton>
            <BaseButton
              variant="danger"
              :disabled="uninstallKey === `${selectedInstalledPlugin.scope}:${selectedInstalledPlugin.id}`"
              @click="uninstallPlugin(selectedInstalledPlugin)"
            >
              <LucideIcon :icon="Trash2" :size="13" />
              {{ uninstallConfirmKey === `${selectedInstalledPlugin.scope}:${selectedInstalledPlugin.id}` ? t("common.confirm") : t("plugin.hub.uninstall") }}
            </BaseButton>
          </div>
        </header>

        <section class="plugin-detail-meta">
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.install.scope") }}</span>
            <strong>{{ pluginScopeLabel(selectedInstalledPlugin.scope) }}</strong>
          </div>
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.export.version") }}</span>
            <strong>{{ selectedInstalledPlugin.version || "0.0.0" }}</strong>
          </div>
          <div v-if="installedUpdateCandidate(selectedInstalledPlugin)" class="plugin-detail-meta-item">
            <span>{{ t("plugin.hub.latestAvailable") }}</span>
            <strong>{{ installedLatestVersionLabel(selectedInstalledPlugin) }}</strong>
          </div>
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.detail.components") }}</span>
            <strong>{{ formatComponentSummary(selectedInstalledPlugin) }}</strong>
          </div>
          <div class="plugin-detail-meta-item">
            <span>{{ t("plugin.dependency.title") }}</span>
            <strong>{{ pluginDependencyLabel(selectedInstalledPlugin) }}</strong>
          </div>
          <div class="plugin-detail-meta-item is-wide">
            <span>{{ t("plugin.detail.path") }}</span>
            <strong>{{ selectedInstalledPlugin.root }}</strong>
          </div>
        </section>

        <section class="plugin-detail-scroll">
          <div class="plugin-detail-section">
            <div class="plugin-detail-label">{{ t("plugin.dependency.title") }}</div>
            <div v-if="selectedInstalledPlugin.dependencies?.project?.length" class="plugin-dependency-list">
              <div
                v-for="dependency in selectedInstalledPlugin.dependencies.project"
                :key="`${dependency.kind}:${dependency.name}`"
                class="plugin-dependency-row"
              >
                <span>{{ dependency.kind || "custom" }}</span>
                <span>{{ dependency.name }}</span>
                <span v-if="dependency.version">{{ dependency.version }}</span>
              </div>
            </div>
            <div v-else class="plugin-empty compact">{{ t("plugin.dependency.empty") }}</div>
          </div>
        </section>
      </div>

      <div v-else class="plugin-detail-empty-state">
        {{ t("plugin.detail.empty") }}
      </div>
    </main>

    <BaseContextMenu
      v-if="pluginListContextMenu"
      class="plugin-list-context-menu"
      :x="pluginListContextMenu.x"
      :y="pluginListContextMenu.y"
      :aria-label="t('app.tab.plugins')"
      @close="closePluginListContextMenu"
    >
      <button class="plugin-list-context-menu-item" type="button" @click="refreshPluginListFromMenu">
        <LucideIcon :icon="RefreshCw" :size="13" />
        <span>{{ t("plugin.hub.refresh") }}</span>
      </button>
      <div class="ctx-sep"></div>
      <button class="plugin-list-context-menu-item" type="button" @click="openDirectImport('local')">
        <LucideIcon :icon="FolderOpen" :size="13" />
        <span>{{ t("plugin.import.local") }}</span>
      </button>
      <button class="plugin-list-context-menu-item" type="button" @click="openDirectImport('link')">
        <LucideIcon :icon="Link" :size="13" />
        <span>{{ t("plugin.import.link") }}</span>
      </button>
    </BaseContextMenu>

    <Transition name="plugin-hub-modal">
      <div
        v-if="pluginHelpOpen"
        class="plugin-hub-config-overlay"
        @click.self="closePluginHelp"
      >
        <section
          class="plugin-hub-config-modal plugin-help-modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="plugin-help-title"
          tabindex="-1"
          @keydown.esc.stop="closePluginHelp"
        >
          <header class="plugin-hub-config-header">
            <div class="plugin-hub-config-title">
              <span id="plugin-help-title">{{ t("plugin.help.title") }}</span>
              <span>{{ t("plugin.help.subtitle") }}</span>
            </div>
            <button
              type="button"
              class="plugin-hub-config-close"
              :aria-label="t('common.close')"
              @click="closePluginHelp"
            >
              <LucideIcon :icon="X" :size="15" />
            </button>
          </header>

          <div class="plugin-hub-config-body plugin-help-body">
            <section class="plugin-help-section">
              <div class="plugin-help-section-title">{{ t("plugin.help.featureTitle") }}</div>
              <p>{{ t("plugin.help.featureBody") }}</p>
            </section>
            <section class="plugin-help-section">
              <div class="plugin-help-section-title">{{ t("plugin.help.commandTitle") }}</div>
              <p>{{ t("plugin.help.commandBody") }}</p>
            </section>
            <section class="plugin-help-section">
              <div class="plugin-help-section-title">{{ t("plugin.help.skillTitle") }}</div>
              <p>{{ t("plugin.help.skillBody") }}</p>
            </section>
            <section class="plugin-help-section">
              <div class="plugin-help-section-title">{{ t("plugin.help.publishTitle") }}</div>
              <p>{{ t("plugin.help.publishBody") }}</p>
            </section>
          </div>

          <footer class="plugin-hub-config-footer">
            <div class="plugin-hub-config-footer-spacer" />
            <BaseButton @click="closePluginHelp">
              {{ t("common.close") }}
            </BaseButton>
          </footer>
        </section>
      </div>
    </Transition>

    <Transition name="plugin-hub-modal">
      <div
        v-if="registryConfigOpen"
        class="plugin-hub-config-overlay"
        @click.self="registryConfigOpen = false"
      >
        <section
          class="plugin-hub-config-modal"
          role="dialog"
          aria-modal="true"
          :aria-label="t('plugin.hub.registryConfig')"
        >
          <header class="plugin-hub-config-header">
            <div class="plugin-hub-config-title">
              <span>{{ t("plugin.hub.registryConfig") }}</span>
              <span>{{ t("plugin.hub.registryCount", registrySourceDrafts.length) }}</span>
            </div>
            <button
              type="button"
              class="plugin-hub-config-close"
              :aria-label="t('common.close')"
              @click="registryConfigOpen = false"
            >
              <LucideIcon :icon="X" :size="15" />
            </button>
          </header>

          <div class="plugin-hub-config-body">
            <div class="plugin-hub-registry-table">
              <div class="plugin-hub-registry-row is-header">
                <span>{{ t("plugin.hub.registryName") }}</span>
                <span>{{ t("plugin.hub.registryRepo") }}</span>
                <span>{{ t("plugin.hub.registryBranch") }}</span>
                <span>{{ t("plugin.hub.registryPath") }}</span>
                <span></span>
              </div>
              <div
                v-for="(source, index) in registrySourceDrafts"
                :key="source.id"
                class="plugin-hub-registry-row"
              >
                <input
                  v-model="source.name"
                  class="plugin-hub-registry-input"
                  :placeholder="t('plugin.hub.registryName')"
                  @keydown.enter.prevent="saveRegistrySources"
                />
                <input
                  v-model="source.repoInput"
                  class="plugin-hub-registry-input"
                  :placeholder="t('plugin.hub.registryAddressPlaceholder')"
                  @keydown.enter.prevent="saveRegistrySources"
                />
                <input
                  v-model="source.branch"
                  class="plugin-hub-registry-input"
                  :placeholder="DEFAULT_PLUGIN_REGISTRY_BRANCH"
                  @keydown.enter.prevent="saveRegistrySources"
                />
                <input
                  v-model="source.path"
                  class="plugin-hub-registry-input"
                  :placeholder="DEFAULT_PLUGIN_REGISTRY_PATH"
                  @keydown.enter.prevent="saveRegistrySources"
                />
                <BaseButton
                  class="plugin-icon-button"
                  variant="danger"
                  :disabled="registrySourceDrafts.length <= 1"
                  :title="t('plugin.hub.registryRemove')"
                  @click="removeRegistrySourceDraft(index)"
                >
                  <LucideIcon :icon="Trash2" :size="13" />
                </BaseButton>
              </div>
            </div>

            <div v-if="registryConfigError" class="plugin-error compact">{{ registryConfigError }}</div>
          </div>

          <footer class="plugin-hub-config-footer">
            <BaseButton @click="addRegistrySource">
              <LucideIcon :icon="Plus" :size="13" />
              {{ t("plugin.hub.registryAdd") }}
            </BaseButton>
            <div class="plugin-hub-config-footer-spacer" />
            <BaseButton @click="registryConfigOpen = false">
              {{ t("common.cancel") }}
            </BaseButton>
            <BaseButton @click="saveRegistrySources">
              <LucideIcon :icon="Save" :size="13" />
              {{ t("plugin.hub.registrySave") }}
            </BaseButton>
          </footer>
        </section>
      </div>
    </Transition>

    <Transition name="plugin-hub-modal">
      <div
        v-if="githubOAuthOpen"
        class="plugin-hub-config-overlay"
        @click.self="closeGithubOAuth"
      >
        <section
          class="plugin-hub-config-modal plugin-github-oauth-modal"
          role="dialog"
          aria-modal="true"
          :aria-label="t('plugin.hub.githubAuth')"
        >
          <header class="plugin-hub-config-header">
            <div class="plugin-hub-config-title">
              <span>{{ t("plugin.hub.githubAuth") }}</span>
              <span>
                {{
                  githubAuthStatus.authenticated
                    ? t("plugin.hub.githubAuthenticated", githubAuthStatus.account || "GitHub")
                    : t("plugin.hub.githubAuthHint")
                }}
              </span>
            </div>
            <button
              type="button"
              class="plugin-hub-config-close"
              :aria-label="t('common.close')"
              @click="closeGithubOAuth"
            >
              <LucideIcon :icon="X" :size="15" />
            </button>
          </header>

          <div class="plugin-hub-config-body plugin-github-oauth-body">
            <template v-if="githubAuthStatus.authenticated && githubOAuthStep !== 'opening' && githubOAuthStep !== 'waiting'">
              <div class="plugin-github-oauth-status">
                <LucideIcon :icon="UserCheck" :size="28" />
                <div>
                  <strong>{{ t("plugin.hub.githubAuthenticated", githubAuthStatus.account || "GitHub") }}</strong>
                  <span>{{ t("plugin.hub.githubAuthHint") }}</span>
                </div>
              </div>
            </template>

            <template v-else-if="githubOAuthStep === 'opening'">
              <div class="plugin-github-oauth-status">
                <LucideIcon class="plugin-github-oauth-spin" :icon="LoaderCircle" :size="24" />
                <div>
                  <strong>{{ t("plugin.hub.githubOAuthOpening") }}</strong>
                  <span>{{ t("plugin.hub.githubAuthHint") }}</span>
                </div>
              </div>
            </template>

            <template v-else-if="githubOAuthStep === 'waiting'">
              <div class="plugin-github-oauth-instruction">
                {{ githubOAuthUserCode ? t("plugin.hub.githubOAuthInstruction") : t("plugin.hub.githubCliInstruction") }}
              </div>
              <button
                v-if="githubOAuthUserCode"
                type="button"
                class="plugin-github-oauth-code"
                :class="{ copied: githubOAuthCodeCopied }"
                :title="githubOAuthCodeCopied ? t('common.copied') : t('common.clickToCopy')"
                @click="copyGithubOAuthCode"
              >
                <span>{{ githubOAuthUserCode }}</span>
                <span>
                  <LucideIcon :icon="Copy" :size="13" />
                  {{ githubOAuthCodeCopied ? t("common.copied") : t("common.clickToCopy") }}
                </span>
              </button>
              <BaseButton v-if="githubOAuthUrl" class="plugin-github-oauth-link" @click="openUrl(githubOAuthUrl)">
                <LucideIcon :icon="ExternalLink" :size="13" />
                {{ t("plugin.hub.githubOAuthOpen") }}
              </BaseButton>
              <div class="plugin-github-oauth-waiting">
                <LucideIcon class="plugin-github-oauth-spin" :icon="LoaderCircle" :size="14" />
                <span>{{ t("plugin.hub.githubOAuthWaiting") }}</span>
              </div>
            </template>

            <template v-else>
              <div class="plugin-github-oauth-status">
                <LucideIcon :icon="KeyRound" :size="24" />
                <div>
                  <strong>{{ t("plugin.hub.githubLogin") }}</strong>
                  <span>{{ t("plugin.hub.githubAuthHint") }}</span>
                </div>
              </div>
            </template>

            <div v-if="githubOAuthError || githubAuthError" class="plugin-error compact">
              {{ githubOAuthError || githubAuthError }}
            </div>
          </div>

          <footer class="plugin-hub-config-footer">
            <template v-if="githubAuthStatus.authenticated && githubOAuthStep !== 'opening' && githubOAuthStep !== 'waiting'">
              <BaseButton
                :disabled="githubAuthSaving"
                @click="logoutGithubAuth"
              >
                <LucideIcon :icon="LogOut" :size="13" />
                {{ githubAuthSaving ? t("common.loading") : t("plugin.hub.githubLogout") }}
              </BaseButton>
              <div class="plugin-hub-config-footer-spacer" />
              <BaseButton @click="closeGithubOAuth">
                {{ t("common.close") }}
              </BaseButton>
            </template>
            <template v-else>
              <div class="plugin-hub-config-footer-spacer" />
              <BaseButton v-if="githubOAuthStep === 'waiting' || githubOAuthStep === 'opening'" @click="closeGithubOAuth">
                {{ t("common.cancel") }}
              </BaseButton>
              <BaseButton v-else @click="closeGithubOAuth">
                {{ t("common.cancel") }}
              </BaseButton>
              <BaseButton
                v-if="githubOAuthStep === 'idle' || githubOAuthStep === 'error'"
                @click="startGithubOAuth"
              >
                <LucideIcon :icon="KeyRound" :size="13" />
                {{ githubOAuthStep === "error" ? t("plugin.hub.githubOAuthRetry") : t("plugin.hub.githubLogin") }}
              </BaseButton>
            </template>
          </footer>
        </section>
      </div>
    </Transition>

    <Transition name="plugin-hub-modal">
      <div
        v-if="directImportOpen"
        class="plugin-hub-config-overlay"
        @click.self="closeDirectImport"
      >
        <section
          class="plugin-hub-config-modal plugin-import-modal"
          role="dialog"
          aria-modal="true"
          :aria-label="directImportMode === 'local' ? t('plugin.import.local') : t('plugin.import.link')"
        >
          <header class="plugin-hub-config-header">
            <div class="plugin-hub-config-title">
              <span>{{ directImportMode === "local" ? t("plugin.import.local") : t("plugin.import.link") }}</span>
              <span>{{ t("plugin.install.scope") }}</span>
            </div>
            <button
              type="button"
              class="plugin-hub-config-close"
              :aria-label="t('common.close')"
              @click="closeDirectImport"
            >
              <LucideIcon :icon="X" :size="15" />
            </button>
          </header>

          <div class="plugin-hub-config-body">
            <label class="plugin-import-field">
              <span>{{ t("plugin.import.source") }}</span>
              <div class="plugin-import-source-row">
                <input
                  v-model="directImportSource"
                  class="plugin-hub-registry-input plugin-import-source-input"
                  :placeholder="directImportMode === 'local' ? t('plugin.import.localPlaceholder') : t('plugin.import.linkPlaceholder')"
                  @keydown.enter.prevent="installDirectPlugin"
                />
                <template v-if="directImportMode === 'local'">
                  <BaseButton :title="t('plugin.import.pickFile')" @click="chooseDirectImportFile">
                    <LucideIcon :icon="Package" :size="13" />
                    {{ t("plugin.import.pickFile") }}
                  </BaseButton>
                  <BaseButton :title="t('plugin.import.pickFolder')" @click="chooseDirectImportFolder">
                    <LucideIcon :icon="FolderOpen" :size="13" />
                    {{ t("plugin.import.pickFolder") }}
                  </BaseButton>
                </template>
              </div>
            </label>

            <div class="plugin-import-field">
              <span>{{ t("plugin.install.scope") }}</span>
              <div class="plugin-import-scope-row">
                <BaseButton
                  :class="{ active: directImportScope === 'app' }"
                  @click="setDirectImportScope('app')"
                >
                  {{ t("plugin.scope.app") }}
                </BaseButton>
                <BaseButton
                  :class="{ active: directImportScope === 'project' }"
                  :disabled="!hasWorkspace"
                  :title="hasWorkspace ? t('plugin.scope.project') : t('plugin.install.projectDisabled')"
                  @click="setDirectImportScope('project')"
                >
                  {{ t("plugin.scope.project") }}
                </BaseButton>
              </div>
            </div>

            <div v-if="directImportError" class="plugin-error compact">{{ directImportError }}</div>
          </div>

          <footer class="plugin-hub-config-footer">
            <div class="plugin-hub-config-footer-spacer" />
            <BaseButton :disabled="directImportInstalling" @click="closeDirectImport">
              {{ t("common.cancel") }}
            </BaseButton>
            <BaseButton :disabled="directImportInstalling" @click="installDirectPlugin">
              {{ directImportInstalling ? t("plugin.hub.installing") : t("plugin.install.action") }}
            </BaseButton>
          </footer>
        </section>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.plugin-view {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  background: var(--bg-color);
  color: var(--text-color);
  overflow: hidden;
}

.plugin-pane {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.plugin-installed-pane,
.plugin-list-pane {
  flex: 0 0 auto;
  border-right: none;
  background: var(--panel-bg);
}

.plugin-store-pane,
.plugin-detail-pane {
  flex: 1 1 0;
  border-right: none;
  background: var(--panel-bg);
}

.plugin-detail-pane {
  overflow: hidden;
}

.plugin-view.is-resizing-installed-pane {
  cursor: col-resize;
}

.plugin-resize-handle {
  position: relative;
  width: 5px;
  flex: 0 0 5px;
  cursor: col-resize;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.plugin-resize-handle::before {
  content: "";
  position: absolute;
  inset: 0 2px;
  background: transparent;
  transition: background 0.15s ease;
}

.plugin-resize-handle:hover::before,
.plugin-view.is-resizing-installed-pane .plugin-resize-handle::before {
  background: var(--accent-color);
}

.plugin-pane-header {
  min-height: 44px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  box-sizing: border-box;
}

.plugin-list-header {
  align-items: stretch;
  flex-direction: column;
}

.plugin-list-titlebar {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.plugin-pane-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.plugin-titlebar-actions {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 6px;
}

.plugin-github-login-button {
  height: 28px;
  min-width: 0;
  padding: 0 8px;
  gap: 6px;
  color: var(--text-secondary);
  font-size: 12px;
}

.plugin-github-login-button.is-authenticated {
  color: var(--text-color);
}

.plugin-header-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  min-width: 0;
}

.plugin-list-tools {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px;
  border-bottom: 1px solid var(--border-color);
}

.plugin-list-tool-row {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.plugin-icon-button {
  min-width: 28px;
  width: 28px;
  padding: 0;
  justify-content: center;
}

.plugin-list-content {
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.plugin-list {
  min-height: 0;
  overflow: auto;
  padding: 8px;
}

.plugin-store-body {
  min-height: 0;
  overflow: auto;
}

.plugin-list-item {
  width: 100%;
  box-sizing: border-box;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.plugin-list-item + .plugin-list-item {
  margin-top: 4px;
}

.plugin-list-item:hover {
  background: var(--hover-bg);
  border-color: var(--border-color);
}

.plugin-list-item.active {
  background: var(--active-bg);
  border-color: var(--border-strong);
}

.plugin-list-item:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 2px;
}

.plugin-registry-section-divider {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 7px 2px 5px;
  color: var(--text-tertiary);
  font-size: 11px;
  line-height: 1;
}

.plugin-registry-section-divider::before,
.plugin-registry-section-divider::after {
  content: "";
  height: 1px;
  flex: 1 1 auto;
  background: var(--border-color);
}

.plugin-registry-section-divider span {
  flex: 0 0 auto;
}

.plugin-registry-list-item {
  align-items: stretch;
  gap: 0;
  padding: 0;
  cursor: default;
}

.plugin-list-select-button {
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 9px 8px 9px 10px;
  border: 0;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.plugin-list-select-button:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -2px;
  border-radius: 5px;
}

.plugin-list-side {
  flex: 0 0 auto;
  min-width: 54px;
  padding: 7px 8px 7px 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  justify-content: space-between;
  gap: 5px;
}

.plugin-list-stats {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  min-height: 14px;
}

.plugin-list-stat {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1;
  white-space: nowrap;
}

.plugin-list-pane :deep(.plugin-registry-action-button) {
  min-height: 20px;
  height: 20px;
  padding: 0 7px;
  border-radius: 4px;
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
}

.plugin-list-pane :deep(.plugin-registry-action-button:hover:not(:disabled)) {
  background: color-mix(in srgb, var(--panel-bg) 58%, var(--hover-bg) 42%);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.plugin-list-pane :deep(.plugin-registry-action-button.is-install-action) {
  color: color-mix(in srgb, var(--accent-color) 64%, var(--text-color) 36%);
  border-color: color-mix(in srgb, var(--accent-border) 74%, var(--border-color) 26%);
  background: color-mix(in srgb, var(--accent-soft) 76%, transparent);
}

.plugin-list-pane :deep(.plugin-registry-action-button.is-update-action) {
  color: var(--accent-color);
  border-color: color-mix(in srgb, var(--accent-border) 82%, var(--border-color) 18%);
  background: color-mix(in srgb, var(--accent-soft) 84%, transparent);
}

.plugin-list-pane :deep(.plugin-registry-action-button.is-install-action:hover:not(:disabled)),
.plugin-list-pane :deep(.plugin-registry-action-button.is-update-action:hover:not(:disabled)) {
  color: var(--accent-color);
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color) 58%);
  background: var(--accent-soft);
}

.plugin-list-pane :deep(.plugin-registry-action-button.is-confirming) {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
}

.plugin-list-pane :deep(.plugin-registry-action-button.is-confirming:hover:not(:disabled)) {
  background: var(--status-danger-bg);
  border-color: var(--status-danger-fg);
  color: var(--status-danger-fg);
}

.plugin-list-main {
  min-width: 0;
  flex: 1;
}

.plugin-list-icon {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
  border-radius: 7px;
}

.plugin-local-icon {
  display: grid;
  place-items: center;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 84%, var(--panel-bg) 16%);
  color: var(--text-secondary);
}

.plugin-list-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.plugin-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 600;
}

.plugin-version,
.plugin-list-id,
.plugin-list-meta {
  font-size: 11px;
  color: var(--text-secondary);
}

.plugin-version {
  flex-shrink: 0;
}

.plugin-scope-tag {
  flex-shrink: 0;
  height: 16px;
  padding: 0 5px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  display: inline-flex;
  align-items: center;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--sidebar-bg) 22%);
  font-size: 10px;
  font-weight: 650;
  line-height: 1;
}

.plugin-list-id {
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.plugin-list-summary {
  margin-top: 4px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.35;
  white-space: nowrap;
  text-overflow: ellipsis;
  overflow: hidden;
}

.plugin-list-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 6px;
}

.plugin-list-footer {
  padding: 4px 0 0;
}

.plugin-empty,
.plugin-error {
  padding: 16px 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.plugin-empty.compact {
  padding: 10px;
}

.plugin-error.compact {
  padding: 0;
}

.plugin-error {
  color: var(--status-danger-fg);
}

.plugin-detail-view {
  min-width: 0;
  min-height: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.plugin-detail-hero {
  flex: 0 0 auto;
  min-width: 0;
  padding: 18px 20px 16px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
}

.plugin-detail-hero-main {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  gap: 14px;
}

.plugin-detail-hero-icon {
  width: 64px;
  height: 64px;
  flex: 0 0 auto;
  border-radius: 8px;
}

.plugin-detail-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.plugin-detail-summary {
  max-width: 780px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.plugin-detail-actions {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  flex-wrap: wrap;
}

.plugin-detail-actions :deep(.plugin-star-button.is-starred) {
  color: var(--accent-color);
  border-color: color-mix(in srgb, var(--accent-border) 82%, var(--border-color) 18%);
  background: color-mix(in srgb, var(--accent-soft) 84%, transparent);
}

.plugin-detail-actions :deep(.plugin-star-button.is-starred:hover:not(:disabled)) {
  color: var(--accent-color);
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color) 58%);
  background: var(--accent-soft);
}

.plugin-detail-meta {
  flex: 0 0 auto;
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 10px 16px;
  padding: 12px 20px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 98%, var(--sidebar-bg) 2%);
}

.plugin-detail-meta-item {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.plugin-detail-meta-item.is-wide {
  grid-column: span 2;
}

.plugin-detail-meta-item span {
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.plugin-detail-meta-item strong {
  min-width: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plugin-detail-scroll {
  min-height: 0;
  flex: 1;
  overflow: auto;
  padding: 18px 20px 28px;
}

.plugin-detail-empty-state {
  min-height: 0;
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  color: var(--text-secondary);
  font-size: 12px;
}

.plugin-detail-body,
.plugin-hub-body {
  min-height: 0;
  flex: 1;
}

.plugin-detail-body {
  overflow: auto;
  padding: 12px;
}

.plugin-detail-section {
  padding: 10px 0;
  border-bottom: 1px solid var(--border-color);
}

.plugin-detail-section:first-child {
  padding-top: 0;
}

.plugin-detail-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-width: 0;
}

.plugin-detail-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 14px;
  font-weight: 650;
  color: var(--text-color);
}

.plugin-detail-id {
  margin-top: 4px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  word-break: break-all;
}

.plugin-detail-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.plugin-detail-field {
  min-width: 0;
}

.plugin-detail-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
}

.plugin-detail-value {
  margin-top: 3px;
  font-size: 12px;
  color: var(--text-color);
  min-width: 0;
}

.plugin-detail-value.is-path {
  font-family: var(--font-mono-identifier);
  word-break: break-all;
}

.plugin-dependency-list {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.plugin-dependency-row {
  display: grid;
  grid-template-columns: minmax(80px, 0.8fr) minmax(120px, 1.4fr) minmax(64px, 0.6fr);
  gap: 8px;
  padding: 6px 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  color: var(--text-secondary);
  font-size: 11px;
}

.plugin-hub-body {
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.plugin-hub-toolbar {
  flex: 0 0 auto;
  min-height: 42px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  gap: 8px;
}

.plugin-hub-config-overlay {
  position: fixed;
  inset: 0;
  z-index: 2200;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 18px;
  background: rgba(0, 0, 0, 0.42);
}

.plugin-hub-config-modal {
  width: min(720px, calc(100vw - 48px));
  max-height: min(560px, calc(100vh - 48px));
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--panel-bg);
  box-shadow: 0 16px 44px rgba(0, 0, 0, 0.32);
}

.plugin-import-modal {
  width: min(640px, calc(100vw - 48px));
}

.plugin-github-oauth-modal {
  width: min(520px, calc(100vw - 48px));
}

.plugin-help-modal {
  width: min(640px, calc(100vw - 48px));
}

.plugin-help-body {
  gap: 14px;
}

.plugin-help-section {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.plugin-help-section-title {
  color: var(--text-color);
  font-size: 12px;
  font-weight: 650;
}

.plugin-help-section p {
  margin: 0;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.55;
}

.plugin-github-oauth-body {
  gap: 12px;
}

.plugin-github-oauth-status {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
  color: var(--text-secondary);
}

.plugin-github-oauth-status > div {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.plugin-github-oauth-status strong {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 650;
}

.plugin-github-oauth-status span {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.plugin-github-oauth-instruction {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.plugin-github-oauth-code {
  width: 100%;
  min-height: 54px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--input-bg);
  color: var(--text-color);
  cursor: pointer;
  text-align: left;
}

.plugin-github-oauth-code:hover {
  border-color: color-mix(in srgb, var(--accent-color) 46%, var(--border-color) 54%);
  background: var(--hover-bg);
}

.plugin-github-oauth-code > span:first-child {
  font-family: var(--font-mono-editor);
  font-size: 20px;
  font-weight: 700;
  letter-spacing: 0;
}

.plugin-github-oauth-code > span:last-child {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: 11px;
}

.plugin-github-oauth-code.copied {
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color) 58%);
}

.plugin-github-oauth-link {
  align-self: flex-start;
}

.plugin-github-oauth-waiting {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--text-secondary);
  font-size: 12px;
}

.plugin-github-oauth-spin {
  animation: plugin-spin 0.9s linear infinite;
}

@keyframes plugin-spin {
  to {
    transform: rotate(360deg);
  }
}

:global(.plugin-list-context-menu .plugin-list-context-menu-item) {
  gap: 8px;
}

:global(.plugin-list-context-menu .plugin-list-context-menu-item svg) {
  flex: 0 0 auto;
}

.plugin-hub-config-header {
  min-height: 48px;
  padding: 12px 14px 12px 16px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
}

.plugin-hub-config-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.plugin-hub-config-title span:first-child {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 650;
}

.plugin-hub-config-title span:last-child {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plugin-hub-config-close {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.plugin-hub-config-close:hover {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.plugin-hub-config-body {
  min-height: 0;
  overflow: auto;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.plugin-hub-registry-table {
  min-width: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
}

.plugin-hub-registry-row {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(120px, 0.9fr) minmax(180px, 1.35fr) minmax(90px, 0.7fr) minmax(70px, 0.55fr) 28px;
  gap: 8px;
  align-items: center;
  padding: 8px;
  border-top: 1px solid var(--border-color);
}

.plugin-hub-registry-row:first-child {
  border-top: 0;
}

.plugin-hub-registry-row.is-header {
  min-height: 30px;
  padding-top: 6px;
  padding-bottom: 6px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.plugin-hub-registry-input {
  min-width: 0;
  height: 28px;
  box-sizing: border-box;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 0 9px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  outline: none;
}

.plugin-hub-registry-input:focus {
  border-color: var(--accent-color);
}

.plugin-hub-registry-input::placeholder {
  color: var(--text-tertiary);
}

.plugin-hub-config-footer {
  min-height: 48px;
  padding: 10px 16px;
  border-top: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
}

.plugin-hub-config-footer-spacer {
  flex: 1 1 auto;
}

.plugin-import-field {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.plugin-import-source-row,
.plugin-import-scope-row {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.plugin-import-source-input {
  flex: 1;
}

.plugin-import-scope-row :deep(.base-button.active) {
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color) 58%);
  background: var(--accent-soft);
  color: var(--accent-color);
}

.plugin-hub-modal-enter-active,
.plugin-hub-modal-leave-active {
  transition: opacity 0.14s ease;
}

.plugin-hub-modal-enter-active .plugin-hub-config-modal,
.plugin-hub-modal-leave-active .plugin-hub-config-modal {
  transition: transform 0.14s ease, opacity 0.14s ease;
}

.plugin-hub-modal-enter-from,
.plugin-hub-modal-leave-to {
  opacity: 0;
}

.plugin-hub-modal-enter-from .plugin-hub-config-modal,
.plugin-hub-modal-leave-to .plugin-hub-config-modal {
  opacity: 0;
  transform: translateY(-8px);
}

.plugin-hub-search {
  flex: 1;
  min-width: 0;
  height: 26px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 8px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-secondary);
}

.plugin-hub-search:focus-within {
  border-color: var(--accent-color);
}

.plugin-hub-search-input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

.plugin-hub-search-input::placeholder {
  color: var(--text-tertiary);
}

.plugin-hub-content {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
}

.plugin-hub-icon-frame {
  position: relative;
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg) 84%, var(--panel-bg) 16%);
  color: var(--text-secondary);
  overflow: hidden;
}

.plugin-hub-icon-image {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  background: var(--panel-bg);
}

.plugin-hub-description {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.55;
}

.plugin-hub-description .markdown-body {
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-secondary);
}

.plugin-hub-description .markdown-body h1 {
  font-size: 16px;
}

.plugin-hub-description .markdown-body h2 {
  font-size: 14px;
}

.plugin-hub-description .markdown-body h3 {
  font-size: 13px;
}

.plugin-hub-description .markdown-body h1,
.plugin-hub-description .markdown-body h2,
.plugin-hub-description .markdown-body h3,
.plugin-hub-description .markdown-body h4,
.plugin-hub-description .markdown-body h5,
.plugin-hub-description .markdown-body h6 {
  margin-top: 14px;
}

.plugin-hub-description .markdown-body p,
.plugin-hub-description .markdown-body ul,
.plugin-hub-description .markdown-body ol,
.plugin-hub-description .markdown-body blockquote,
.plugin-hub-description .markdown-body pre,
.plugin-hub-description .markdown-body .md-table-wrap {
  margin-bottom: 10px;
}

.plugin-hub-description .markdown-body .md-image-preview {
  max-height: 480px;
}

.plugin-hub-grid {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  align-content: start;
  gap: 8px;
  padding: 8px 12px 12px;
}

.plugin-hub-card {
  min-width: 0;
  min-height: 104px;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--sidebar-bg) 6%);
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.plugin-hub-card:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
}

.plugin-hub-card.active {
  background: var(--active-bg);
  border-color: var(--border-strong);
}

.plugin-hub-card:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 1px;
}

.plugin-hub-card-icon {
  width: 36px;
  height: 36px;
  border-radius: 7px;
}

.plugin-hub-card-main {
  min-width: 0;
  flex: 1;
}

.plugin-hub-card-head {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.plugin-hub-card-summary {
  min-height: 0;
  margin-top: 5px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.35;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.plugin-hub-card-meta {
  min-height: 0;
  margin-top: 8px;
  display: flex;
  flex-wrap: wrap;
  gap: 0;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.3;
}

.plugin-hub-card-meta span {
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plugin-hub-card-meta span + span::before {
  content: "/";
  margin: 0 6px;
  color: var(--text-tertiary);
}

.plugin-hub-grid-footer {
  flex: 0 0 auto;
  padding: 0 12px 12px;
}

.plugin-hub-load-more {
  width: 100%;
  justify-content: center;
}

@media (max-width: 980px) {
  .plugin-view {
    flex-direction: column;
  }

  .plugin-installed-pane,
  .plugin-list-pane {
    width: 100% !important;
    min-height: 240px;
    flex: 0 0 auto;
  }

  .plugin-store-pane,
  .plugin-detail-pane {
    flex: 1 1 0;
    border-top: 1px solid var(--border-color);
  }

  .plugin-resize-handle {
    display: none;
  }

  .plugin-pane-header {
    flex-wrap: wrap;
  }

  .plugin-header-actions {
    width: 100%;
    justify-content: flex-end;
  }

  .plugin-hub-config-footer {
    align-items: stretch;
    flex-wrap: wrap;
  }

  .plugin-hub-registry-row {
    grid-template-columns: minmax(120px, 1fr) minmax(150px, 1.2fr) minmax(84px, 0.7fr) minmax(64px, 0.55fr) 28px;
  }

  .plugin-hub-config-footer-spacer {
    display: none;
  }

  .plugin-hub-grid {
    grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  }

  .plugin-detail-grid {
    grid-template-columns: 1fr;
  }
}
</style>
