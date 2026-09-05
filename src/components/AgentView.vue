
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { Plus, Trash2 } from "lucide";
import {
  type InternalDropDecision,
  type InternalDropTargetRegistration,
  useInternalDragController,
} from "../composables/useInternalDrag";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  deleteRule,
  getAgentEnvTemplate,
  getAgentRenderedEnvPrompt,
  getAgentSystemPrompt,
  getAgentSystemPromptStats,
  getWorkspaceAgentEnvTemplate,
  getWorkspaceAgentRenderedEnvPrompt,
  getWorkspaceAgentSystemPrompt,
  getWorkspaceAgentSystemPromptStats,
  listAgentInjectedItems,
  listAgents,
  listAppRules,
  listRules,
  listSubagentDefs,
  listWorkspaceAgentInjectedItems,
  listWorkspaceAgents,
  listWorkspaceSubagentDefs,
  readAppRule,
  readRule,
  saveRule,
  setAgentInjectionEnabled,
  setAgentToolDirectLoad,
  setAgentToolEnabled,
  setRuleEnabled,
  setRuleOrder,
} from "../services/agent";
import type { AgentInfo, AgentSystemPromptStats, InjectedPromptItem, InjectedToolLoadMode, RuleItem } from "../types";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";
import BaseContextMenu from "./ui/BaseContextMenu.vue";
import BaseSegmented from "./ui/BaseSegmented.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import { parseAgentToolDefinition } from "./agent/toolSchema";
import { buildAgentPromptDashboard, type AgentPromptHealthLevel, type AgentPromptPartKey } from "./agent/agentPromptDashboard";
import { useModelStore } from "../stores/model";
import type { WorkspaceRef } from "../services/project";
import { agentProjectTypesLabel } from "../utils/agentProjectTypes";

const props = defineProps<{
  active?: boolean;
  workingDir: string;
  agentList: AgentInfo[];
  workspaceRef?: WorkspaceRef | null;
}>();
const modelStore = useModelStore();

const selectedAgentId = ref<string>("");
const allAgents = ref<AgentInfo[]>([]);
const selectedAgent = computed(() =>
  allAgents.value.find((agent) => agent.id === selectedAgentId.value) ?? null,
);

type SelectedKind =
  | { type: "prompt" }
  | { type: "env" }
  | { type: "rule"; rule: RuleItem }
  | { type: "injected"; item: InjectedPromptItem };
const selected = ref<SelectedKind | null>(null);

// ── System Prompt ──
const systemPromptContent = ref("");
const systemPromptLoading = ref(false);
let systemPromptRequestId = 0;
const promptStats = ref<AgentSystemPromptStats | null>(null);
const promptStatsLoading = ref(false);
const promptStatsError = ref("");
let promptStatsRequestId = 0;

// ── Env Template ──
const envTemplateContent = ref("");
const envTemplateLoading = ref(false);
let envTemplateRequestId = 0;
const envRenderedContent = ref("");
const envRenderedLoading = ref(false);
type EnvPreviewMode = "structure" | "rendered";
const envPreviewMode = ref<EnvPreviewMode>("structure");
let envRenderedRequestId = 0;

const envPreviewModeOptions = computed(() => [
  { value: "structure", label: t("agent.envPreview.structure") },
  { value: "rendered", label: t("agent.envPreview.rendered") },
]);

const envPreviewContent = computed(() =>
  envPreviewMode.value === "rendered"
    ? envRenderedContent.value
    : envTemplateContent.value,
);

const envPreviewLoading = computed(() =>
  envPreviewMode.value === "rendered"
    ? envRenderedLoading.value
    : envTemplateLoading.value,
);

function highlightedEnv(raw: string): string {
  let s = raw
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  s = s.replace(/(\{\{[#/][a-z_]+\}\})/gi, '<span class="env-hl-block">$1</span>');
  s = s.replace(/(&lt;[a-z_]+&gt;)/gi, '<span class="env-hl-var">$1</span>');
  return s;
}

// ── Rule ──
const ruleItems = ref<RuleItem[]>([]);
const rulePreviewOrder = ref<RuleItem[] | null>(null);
const displayedRuleItems = computed(() => rulePreviewOrder.value ?? ruleItems.value);
const ruleLoading = ref(false);
let ruleRequestId = 0;
const ruleContent = ref("");
const ruleContentLoading = ref(false);
const ruleEditing = ref(false);
const ruleEditContent = ref("");
const ruleCreating = ref(false);
const ruleNewName = ref("");
const ruleNewContent = ref("");
const confirmingDeleteRule = ref<string | null>(null);
const error = ref("");

const ruleDragIndex = ref<number | null>(null);
const ruleDragOverIndex = ref<number | null>(null);
const ruleContextMenu = ref<{ x: number; y: number; rule: RuleItem | null } | null>(null);
const agentViewRef = ref<HTMLElement | null>(null);
const internalDrag = useInternalDragController();
let unregisterRuleDropTarget: (() => void) | null = null;
const AGENT_RULE_INTERNAL_DRAG_TYPE = "locus/agent-rule";

// ── Injected ──
const injectedItems = ref<InjectedPromptItem[]>([]);
const injectedLoading = ref(false);
let injectedRequestId = 0;
const toolLoadSaving = ref(false);
const toolLoadConfigError = ref("");
const toolEnabledSavingId = ref<string | null>(null);
const toolEnabledError = ref("");
const injectionSavingId = ref<string | null>(null);
const injectionError = ref("");
const availableToolItems = computed(() =>
  injectedItems.value.filter((item) => item.kind === "tools"),
);
const mcpToolItems = computed(() =>
  availableToolItems.value.filter((item) => toolMetaString(item.meta, "toolSource") === "mcp"),
);
const nonMcpToolItems = computed(() =>
  availableToolItems.value.filter((item) => toolMetaString(item.meta, "toolSource") !== "mcp"),
);
const directToolItems = computed(() =>
  nonMcpToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "direct"),
);
const lazyToolItems = computed(() =>
  nonMcpToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "lazy"),
);
const skillToolItems = computed(() =>
  nonMcpToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "skill"),
);
const injectedRuleItems = computed(() =>
  injectedItems.value.filter((item) => item.kind === "rule"),
);
const envInjectionItem = computed(() =>
  injectedItems.value.find((item) => item.id === "env") ?? null,
);
const injectedContextItems = computed(() =>
  injectedItems.value.filter((item) => item.kind === "context" && item.id !== "env"),
);
const ruleSectionEntryCount = computed(() =>
  ruleItems.value.length + injectedRuleItems.value.length,
);
const injectedContextEntryCount = computed(() =>
  injectedContextItems.value.length + 1,
);
const promptDashboard = computed(() =>
  buildAgentPromptDashboard(promptStats.value, ruleItems.value, injectedItems.value),
);

const toolGroups = computed(() => [
  { key: "tools:direct", label: t("agent.directTools"), items: directToolItems.value },
  { key: "tools:lazy", label: t("agent.lazyTools"), items: lazyToolItems.value },
  { key: "tools:skill", label: t("agent.skillTools"), items: skillToolItems.value },
  { key: "tools:mcp", label: t("agent.mcpTools"), items: mcpToolItems.value },
].filter((group) => group.items.length > 0));

// ── Collapsible sections ──
const collapsedSections = ref<Record<string, boolean>>({
  rules: false,
  injected: false,
  "tools:direct": false,
  "tools:lazy": true,
  "tools:skill": true,
  "tools:mcp": true,
});

function isSectionCollapsed(key: string): boolean {
  return collapsedSections.value[key] === true;
}

function toggleSection(key: string) {
  collapsedSections.value[key] = !isSectionCollapsed(key);
}

function toolMetaLoadMode(meta: InjectedPromptItem["meta"]): InjectedToolLoadMode {
  const record = toolMetaRecord(meta);
  if (record?.loadMode === "lazy") return "lazy";
  if (record?.loadMode === "skill") return "skill";
  return "direct";
}

function toolMetaRecord(meta: InjectedPromptItem["meta"]): Record<string, unknown> | null {
  if (!meta || typeof meta !== "object" || Array.isArray(meta)) return null;
  return meta as Record<string, unknown>;
}

function toolMetaBoolean(meta: InjectedPromptItem["meta"], key: string): boolean | null {
  const value = toolMetaRecord(meta)?.[key];
  return typeof value === "boolean" ? value : null;
}

function toolMetaString(meta: InjectedPromptItem["meta"], key: string): string {
  const value = toolMetaRecord(meta)?.[key];
  return typeof value === "string" ? value : "";
}

function currentSubagentModels(): Record<string, string> {
  return modelStore.modelDefaults?.subagentModels ?? {};
}

/// MCP wire names render as the bare tool name; the persisted identity
/// (item.title, override keys) stays the full wire name.
function toolItemDisplayTitle(item: InjectedPromptItem): string {
  const mcpToolName = toolMetaString(item.meta, "mcpToolName");
  return mcpToolName || item.title;
}

function toolItemMcpServer(item: InjectedPromptItem): string {
  return toolMetaString(item.meta, "mcpServerName")
    || toolMetaString(item.meta, "mcpServerId");
}

const sidebarWidth = ref(160);
const dirPanelWidth = ref(280);
let resizing: "sidebar" | "dir" | null = null;
let resizeStartX = 0;
let resizeStartWidth = 0;
let releaseSelectionLock: (() => void) | null = null;
let pluginsChangedUnlisten: UnlistenFn | null = null;
let agentsChangedUnlisten: UnlistenFn | null = null;
let agentViewUnmounted = false;

function selectedRule(): RuleItem | null {
  return selected.value?.type === "rule" ? selected.value.rule : null;
}

function ruleKey(rule: RuleItem | null | undefined): string {
  return rule?.key || rule?.fileName || "";
}

function selectedRuleKey(): string {
  return ruleKey(selectedRule());
}

function canEditRule(rule: RuleItem | null | undefined): boolean {
  return !!rule && !rule.readOnly;
}

function canToggleRule(rule: RuleItem | null | undefined): boolean {
  return !!props.workspaceRef && !!rule && !rule.readOnly && !rule.pluginId;
}

function closeRuleContextMenu() {
  ruleContextMenu.value = null;
}

function selectedInjectedItem(): InjectedPromptItem | null {
  return selected.value?.type === "injected" ? selected.value.item : null;
}

const selectedToolDefinition = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return null;
  return parseAgentToolDefinition(item.meta);
});

const selectedToolDescription = computed(() => {
  return selectedToolDefinition.value?.description || selectedInjectedItem()?.content || "";
});

const selectedToolLoadMode = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return null;
  return toolMetaLoadMode(item.meta);
});

const selectedToolLoadLabel = computed(() => {
  const mode = selectedToolLoadMode.value;
  if (mode === "lazy") return t("agent.tool.loadMode.lazy");
  if (mode === "skill") return t("agent.tool.loadMode.skill");
  return t("agent.tool.loadMode.direct");
});

const selectedToolRuntimeAvailable = computed(() => {
  const item = selectedInjectedItem();
  return !item || item.kind !== "tools" || toolItemRuntimeAvailable(item);
});

const selectedToolUnavailableReason = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return "";
  return toolItemUnavailableReason(item);
});

const selectedToolLoadSummary = computed(() => {
  if (!selectedToolRuntimeAvailable.value) {
    return t("agent.tool.loadSummary.unavailable");
  }
  const mode = selectedToolLoadMode.value;
  if (mode === "lazy") return t("agent.tool.loadSummary.lazy");
  if (mode === "skill") return t("agent.tool.loadSummary.skill");
  return t("agent.tool.loadSummary.direct");
});

const selectedToolCanConfigureDirectLoad = computed(() => {
  const item = selectedInjectedItem();
  if (!props.workspaceRef || !item || item.kind !== "tools") return false;
  return toolMetaBoolean(item.meta, "canConfigureDirectLoad") === true;
});

const selectedToolDirectLoadChecked = computed(() => selectedToolLoadMode.value === "direct");

const selectedToolEnabled = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return true;
  return toolItemEnabled(item);
});

const selectedToolCanToggleEnabled = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return false;
  return canToggleToolEnabled(item);
});

function toolItemEnabled(item: InjectedPromptItem): boolean {
  return toolMetaBoolean(item.meta, "enabled") !== false;
}

function toolItemRuntimeAvailable(item: InjectedPromptItem): boolean {
  return toolMetaBoolean(item.meta, "runtimeAvailable") !== false;
}

function toolUnavailableReasonText(reason: string): string {
  switch (reason) {
    case "requires_unity_workspace":
      return t("agent.tool.unavailableReason.requiresUnityWorkspace");
    case "unity_service_unavailable":
      return t("agent.tool.unavailableReason.unityServiceUnavailable");
    case "requires_workspace":
      return t("agent.tool.unavailableReason.requiresWorkspace");
    case "knowledge_access_disabled":
      return t("agent.tool.unavailableReason.knowledgeAccessDisabled");
    case "code_analysis_disabled":
      return t("agent.tool.unavailableReason.codeAnalysisDisabled");
    case "code_tool_disabled":
      return t("agent.tool.unavailableReason.codeToolDisabled");
    case "hot_reload_disabled":
      return t("agent.tool.unavailableReason.hotReloadDisabled");
    case "compile_server_disabled":
      return t("agent.tool.unavailableReason.compileServerDisabled");
    case "unity_test_framework_unavailable":
      return t("agent.tool.unavailableReason.unityTestFrameworkUnavailable");
    case "model_vision_unsupported":
      return t("agent.tool.unavailableReason.modelVisionUnsupported");
    case "subagent_depth_limit":
      return t("agent.tool.unavailableReason.subagentDepthLimit");
    case "multi_agent_disabled":
      return t("agent.tool.unavailableReason.multiAgentDisabled");
    case "tool_definition_unavailable":
      return t("agent.tool.unavailableReason.toolDefinitionUnavailable");
    default:
      return t("agent.tool.unavailableReason.runtimeUnavailable");
  }
}

function toolItemUnavailableReason(item: InjectedPromptItem): string {
  if (toolItemRuntimeAvailable(item)) return "";
  return toolUnavailableReasonText(toolMetaString(item.meta, "unavailableReason"));
}

function canToggleToolEnabled(item: InjectedPromptItem): boolean {
  return !!props.workspaceRef && toolMetaBoolean(item.meta, "canToggleEnabled") === true;
}

function injectionItemEnabled(item: InjectedPromptItem | null | undefined): boolean {
  return toolMetaBoolean(item?.meta, "enabled") !== false;
}

function canToggleInjectionItem(item: InjectedPromptItem | null | undefined): boolean {
  return !!props.workspaceRef
    && !!item
    && item.kind !== "tools"
    && toolMetaBoolean(item.meta, "canToggleEnabled") === true;
}

const selectedToolLoadConfigSummary = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return "";

  const directLoadOverride = toolMetaBoolean(item.meta, "directLoadOverride");
  const directLoadDefault =
    toolMetaBoolean(item.meta, "directLoadDefault") ?? selectedToolDirectLoadChecked.value;
  const directText = directLoadDefault
    ? t("agent.tool.loadConfig.defaultDirect")
    : t("agent.tool.loadConfig.defaultLazy");

  if (selectedToolCanConfigureDirectLoad.value) {
    if (directLoadOverride !== null) {
      return directLoadOverride
        ? t("agent.tool.loadConfig.overrideDirect")
        : t("agent.tool.loadConfig.overrideLazy");
    }
    return directText;
  }

  if (selectedToolLoadMode.value === "skill") {
    return t("agent.tool.loadConfig.skillOnly");
  }
  return directText;
});

const selectedToolFooterMeta = computed(() => {
  const tool = selectedToolDefinition.value;
  if (!tool) {
    return injectedItemMeta(selectedInjectedItem()?.kind || "context");
  }

  return t(
    "agent.tool.footerMeta",
    tool.topLevelParameterCount,
    tool.topLevelRequired.length,
    tool.parameterRows.length,
    formatCount(tool.promptCharCount),
    formatTokenCount(tool.estimatedPromptTokens),
  );
});

const selectedToolPreviewMeta = computed(() => {
  if (selectedInjectedItem()?.kind !== "tools") {
    return injectedItemMeta(selectedInjectedItem()?.kind || "context");
  }
  return `${selectedToolLoadLabel.value} · ${selectedToolFooterMeta.value}`;
});

type DashboardNoteTone = "good" | "warn" | "danger";

const numberFormatter = new Intl.NumberFormat();

function formatCount(value: number): string {
  return numberFormatter.format(value);
}

function formatPercent(value: number): string {
  return `${Math.round(value * 100)}%`;
}

function formatTokenCount(value: number): string {
  return t("knowledge.injectionPreview.tokenCount", formatCount(value));
}

function dashboardPartTitle(key: AgentPromptPartKey): string {
  switch (key) {
    case "base":
      return t("agent.dashboard.part.base");
    case "env":
      return t("agent.dashboard.part.env");
    case "rules":
      return t("agent.dashboard.part.rules");
    case "knowledge":
      return t("agent.dashboard.part.knowledge");
    case "tools":
      return t("agent.dashboard.part.tools");
  }
}

function dashboardPartMeta(key: AgentPromptPartKey): string {
  switch (key) {
    case "base":
      return t("agent.dashboard.partMeta.base");
    case "env":
      return t("agent.dashboard.partMeta.env");
    case "rules":
      return t(
        "agent.dashboard.partMeta.rules",
        formatCount(promptDashboard.value.enabledRuleCount),
        formatCount(promptDashboard.value.totalRuleCount),
      );
    case "knowledge":
      return t(
        "agent.dashboard.partMeta.knowledge",
        formatCount(promptDashboard.value.injectedContextCount),
      );
    case "tools":
      return t(
        "agent.dashboard.partMeta.tools",
        formatCount(promptDashboard.value.directToolCount),
        formatCount(promptDashboard.value.lazyToolCount),
        formatCount(promptDashboard.value.skillToolCount),
      );
  }
}

function dashboardHealthLabel(level: AgentPromptHealthLevel): string {
  return t(`agent.dashboard.health.${level}`);
}

const dashboardHealthSummary = computed(() => {
  switch (promptDashboard.value.health.level) {
    case "healthy":
      return t("agent.dashboard.healthSummary.healthy");
    case "watch":
      return t("agent.dashboard.healthSummary.watch");
    case "heavy":
      return t("agent.dashboard.healthSummary.heavy");
  }
});

const dashboardHealthNotes = computed<Array<{ tone: DashboardNoteTone; text: string }>>(() => {
  const dashboard = promptDashboard.value;
  const knowledgePart = dashboard.parts.find((part) => part.key === "knowledge");
  const dominantShare = dashboard.health.dominantShare;

  const totalNote = dashboard.totalTokens <= 8_000
    ? { tone: "good" as const, text: t("agent.dashboard.note.total.light") }
    : dashboard.totalTokens <= 20_000
      ? { tone: "good" as const, text: t("agent.dashboard.note.total.steady") }
      : { tone: "danger" as const, text: t("agent.dashboard.note.total.heavy") };

  const ruleNote = dashboard.enabledRuleCount > 0
    ? {
        tone: "good" as const,
        text: t(
          "agent.dashboard.note.rules.active",
          formatCount(dashboard.enabledRuleCount),
          formatCount(dashboard.totalRuleCount),
        ),
      }
    : {
        tone: "warn" as const,
        text: t("agent.dashboard.note.rules.empty"),
      };

  const distributionNote = dominantShare <= 0.58
    ? { tone: "good" as const, text: t("agent.dashboard.note.distribution.balanced") }
    : {
        tone: "warn" as const,
        text: t(
          "agent.dashboard.note.distribution.dominant",
          dashboardPartTitle(dashboard.health.dominantPartKey),
        ),
      };

  const knowledgeNote = knowledgePart && knowledgePart.tokens > 900 && knowledgePart.share > 0.35
    ? { tone: "warn" as const, text: t("agent.dashboard.note.knowledge.heavy") }
    : { tone: "good" as const, text: t("agent.dashboard.note.knowledge.light") };

  return [totalNote, ruleNote, distributionNote, knowledgeNote];
});

function injectedItemBadge(kind: InjectedPromptItem["kind"]): string {
  if (kind === "rule") return t("agent.injected.rule");
  if (kind === "tools") return t("agent.injected.tools");
  return t("agent.injected.context");
}

function injectedItemMeta(kind: InjectedPromptItem["kind"]): string {
  if (kind === "rule") return t("agent.injectedRule");
  if (kind === "tools") return t("agent.availableTools");
  return t("agent.injectedContext");
}

function sourceBadgeLabel(source: string | null | undefined): string {
  if (source === "app") return t("common.builtIn");
  if (source === "project") return t("common.project");
  if (source === "both") return t("common.builtInAndProject");
  if (source === "user") return t("agent.source.user");
  if (source === "appUser") return t("agent.source.appUser");
  if (source === "pluginApp" || source?.startsWith("pluginApp:")) {
    return t("agent.source.pluginApp");
  }
  if (source === "pluginProject" || source?.startsWith("pluginProject:")) {
    return t("agent.source.pluginProject");
  }
  return "";
}

function sourceBadgeClass(source: string | null | undefined): string {
  if (source === "app") return "source-app";
  if (source === "project") return "source-project";
  if (source === "both") return "source-both";
  if (source === "user") return "source-project";
  if (source === "appUser") return "source-both";
  if (source === "pluginApp" || source?.startsWith("pluginApp:")) return "source-plugin";
  if (source === "pluginProject" || source?.startsWith("pluginProject:")) return "source-plugin";
  return "";
}

function resetAgentDetailState() {
  systemPromptRequestId += 1;
  envTemplateRequestId += 1;
  ruleRequestId += 1;
  injectedRequestId += 1;
  selected.value = null;
  closeRuleContextMenu();
  ruleContent.value = "";
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
  systemPromptContent.value = "";
  systemPromptLoading.value = false;
  envTemplateContent.value = "";
  envTemplateLoading.value = false;
  envRenderedContent.value = "";
  envRenderedLoading.value = false;
  envRenderedRequestId += 1;
  ruleItems.value = [];
  ruleLoading.value = false;
  injectedItems.value = [];
  injectedLoading.value = false;
  injectionError.value = "";
  promptStats.value = null;
  promptStatsError.value = "";
  promptStatsLoading.value = false;
  promptStatsRequestId += 1;
}

function preferredAgentId(agents: AgentInfo[]): string {
  if (agents.length === 0) return "";
  const def = agents.find((agent) => agent.isDefault);
  return def ? def.id : agents[0].id;
}

function mergeAgentLists(...groups: AgentInfo[][]): AgentInfo[] {
  const seen = new Set<string>();
  return groups.flatMap((group) => group.filter((agent) => {
    if (seen.has(agent.id)) return false;
    seen.add(agent.id);
    return true;
  }));
}

let agentListRequestId = 0;

async function loadAllAgents() {
  const requestId = ++agentListRequestId;
  try {
    const workspaceRef = props.workspaceRef;
    let nextAgents: AgentInfo[];
    if (workspaceRef) {
      const [topLevel, subLevel] = await Promise.all([
        listWorkspaceAgents(workspaceRef),
        listWorkspaceSubagentDefs(workspaceRef),
      ]);
      nextAgents = mergeAgentLists(topLevel, subLevel);
    } else if (props.agentList.length > 0) {
      nextAgents = mergeAgentLists(props.agentList);
    } else {
      const [topLevel, subLevel] = await Promise.all([listAgents(), listSubagentDefs()]);
      nextAgents = mergeAgentLists(topLevel, subLevel);
    }
    if (requestId !== agentListRequestId) return;
    allAgents.value = nextAgents;
    const selectedStillAvailable = allAgents.value.some(
      (agent) => agent.id === selectedAgentId.value,
    );
    if (!selectedStillAvailable) {
      const nextAgentId = preferredAgentId(allAgents.value);
      if (selectedAgentId.value !== nextAgentId) {
        selectedAgentId.value = nextAgentId;
        resetAgentDetailState();
      }
    }
  } catch (e) {
    if (requestId !== agentListRequestId) return;
    if (props.agentList.length > 0) {
      allAgents.value = mergeAgentLists(props.agentList);
    }
    console.error("loadAllAgents failed:", e);
  }
}

async function switchAgent(agentId: string) {
  selectedAgentId.value = agentId;
  resetAgentDetailState();
  await loadAgentData();
}

async function loadAgentData() {
  if (!selectedAgentId.value) return;
  await Promise.all([
    loadSystemPrompt(),
    loadEnvTemplate(),
    loadPromptStats(),
    loadInjectedItems(),
    loadRules(),
  ]);
}

function selectPrompt() {
  selected.value = { type: "prompt" };
  closeRuleContextMenu();
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
  if (!systemPromptContent.value) loadSystemPrompt();
}

function selectEnv() {
  selected.value = { type: "env" };
  injectionError.value = "";
  closeRuleContextMenu();
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
  if (!envTemplateContent.value) loadEnvTemplate();
  if (envPreviewMode.value === "rendered" && !envRenderedContent.value) loadRenderedEnvPrompt();
}

// ── Env Template ──
async function loadEnvTemplate() {
  if (!selectedAgentId.value) return;
  const requestId = ++envTemplateRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  envTemplateLoading.value = true;
  try {
    const content = workspaceRef
      ? await getWorkspaceAgentEnvTemplate(workspaceRef, agentId)
      : await getAgentEnvTemplate(agentId);
    if (requestId !== envTemplateRequestId) return;
    envTemplateContent.value = content;
  } catch (e) {
    if (requestId !== envTemplateRequestId) return;
    envTemplateContent.value = t("common.loadFailed", normalizeAppError(e).message);
  } finally {
    if (requestId === envTemplateRequestId) {
      envTemplateLoading.value = false;
    }
  }
}

function setEnvPreviewMode(value: string) {
  if (value !== "structure" && value !== "rendered") return;
  envPreviewMode.value = value;
  if (value === "rendered" && !envRenderedContent.value) {
    loadRenderedEnvPrompt();
  }
}

async function loadRenderedEnvPrompt() {
  if (!selectedAgentId.value) return;
  const requestId = ++envRenderedRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  envRenderedLoading.value = true;
  try {
    const content = workspaceRef
      ? await getWorkspaceAgentRenderedEnvPrompt(
          workspaceRef,
          agentId,
          modelStore.selectedModelId,
        )
      : await getAgentRenderedEnvPrompt(agentId);
    if (requestId !== envRenderedRequestId) return;
    envRenderedContent.value = content;
  } catch (e) {
    if (requestId !== envRenderedRequestId) return;
    envRenderedContent.value = t("common.loadFailed", normalizeAppError(e).message);
  } finally {
    if (requestId === envRenderedRequestId) {
      envRenderedLoading.value = false;
    }
  }
}

// ── System Prompt ──
async function loadSystemPrompt() {
  if (!selectedAgentId.value) return;
  const requestId = ++systemPromptRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  systemPromptLoading.value = true;
  try {
    const content = workspaceRef
      ? await getWorkspaceAgentSystemPrompt(workspaceRef, agentId)
      : await getAgentSystemPrompt(agentId);
    if (requestId !== systemPromptRequestId) return;
    systemPromptContent.value = content;
  } catch (e) {
    if (requestId !== systemPromptRequestId) return;
    systemPromptContent.value = t("common.loadFailed", normalizeAppError(e).message);
  } finally {
    if (requestId === systemPromptRequestId) {
      systemPromptLoading.value = false;
    }
  }
}

async function loadPromptStats() {
  if (!selectedAgentId.value) return;
  const requestId = ++promptStatsRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  promptStatsLoading.value = true;
  try {
    const stats = workspaceRef
      ? await getWorkspaceAgentSystemPromptStats(
          workspaceRef,
          agentId,
          modelStore.selectedModelId,
        )
      : await getAgentSystemPromptStats(agentId);
    if (requestId !== promptStatsRequestId) return;
    promptStats.value = stats;
    promptStatsError.value = "";
  } catch (e) {
    if (requestId !== promptStatsRequestId) return;
    promptStats.value = null;
    promptStatsError.value = normalizeAppError(e).message;
  } finally {
    if (requestId === promptStatsRequestId) {
      promptStatsLoading.value = false;
    }
  }
}

// ── Rule CRUD ──
async function loadRules() {
  if (!selectedAgentId.value) return;
  const requestId = ++ruleRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  ruleLoading.value = true;
  try {
    const items = workspaceRef
      ? await listRules(workspaceRef, agentId)
      : await listAppRules(agentId);
    if (requestId !== ruleRequestId) return;
    ruleItems.value = items;
  } catch (e) {
    if (requestId !== ruleRequestId) return;
    console.error("list_rules failed:", e);
    ruleItems.value = [];
  } finally {
    if (requestId === ruleRequestId) {
      ruleLoading.value = false;
    }
  }
}

async function loadInjectedItems() {
  if (!selectedAgentId.value) return;
  const requestId = ++injectedRequestId;
  const agentId = selectedAgentId.value;
  const workspaceRef = props.workspaceRef;
  injectedLoading.value = true;
  try {
    const items = workspaceRef
      ? await listWorkspaceAgentInjectedItems(
          workspaceRef,
          agentId,
          null,
          modelStore.selectedModelId,
          currentSubagentModels(),
        )
      : await listAgentInjectedItems(
          agentId,
          null,
          modelStore.selectedModelId,
          currentSubagentModels(),
        );
    if (requestId !== injectedRequestId) return;
    injectedItems.value = items;
    if (selected.value?.type === "injected") {
      const selectedId = selected.value.item.id;
      const updated = items.find(item => item.id === selectedId);
      if (updated) {
        selected.value = { type: "injected", item: updated };
      } else {
        selected.value = null;
      }
    }
  } catch (e) {
    if (requestId !== injectedRequestId) return;
    console.error("list_agent_injected_items failed:", e);
    injectedItems.value = [];
  } finally {
    if (requestId === injectedRequestId) {
      injectedLoading.value = false;
    }
  }
}

function selectInjectedItem(item: InjectedPromptItem) {
  selected.value = { type: "injected", item };
  toolLoadConfigError.value = "";
  toolEnabledError.value = "";
  injectionError.value = "";
  closeRuleContextMenu();
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
}

async function setSelectedToolDirectLoadState(directLoad: boolean) {
  const item = selectedInjectedItem();
  const tool = selectedToolDefinition.value;
  if (!selectedAgentId.value || !item || item.kind !== "tools" || !tool) return;
  if (!props.workspaceRef) return;
  if (!selectedToolCanConfigureDirectLoad.value || toolLoadSaving.value) return;

  toolLoadSaving.value = true;
  toolLoadConfigError.value = "";
  try {
    await setAgentToolDirectLoad(props.workspaceRef, selectedAgentId.value, tool.name, directLoad);
    await loadInjectedItems();
    void loadPromptStats();
  } catch (e) {
    console.error("set_agent_tool_direct_load failed:", e);
    toolLoadConfigError.value = t("agent.tool.loadConfigSaveFailed", normalizeAppError(e).message);
  } finally {
    toolLoadSaving.value = false;
  }
}

async function setToolEnabledState(item: InjectedPromptItem, enabled: boolean) {
  if (!selectedAgentId.value || item.kind !== "tools") return;
  if (!props.workspaceRef) return;
  if (!canToggleToolEnabled(item) || toolEnabledSavingId.value) return;

  toolEnabledSavingId.value = item.id;
  toolEnabledError.value = "";
  const record = toolMetaRecord(item.meta);
  const previous = record?.enabled;
  if (record) record.enabled = enabled;
  try {
    await setAgentToolEnabled(props.workspaceRef, selectedAgentId.value, item.title, enabled);
    await loadInjectedItems();
    void loadPromptStats();
  } catch (e) {
    console.error("set_agent_tool_enabled failed:", e);
    if (record) record.enabled = previous;
    toolEnabledError.value = t("agent.tool.enableSaveFailed", normalizeAppError(e).message);
  } finally {
    toolEnabledSavingId.value = null;
  }
}

async function setSelectedToolEnabledState(enabled: boolean) {
  const item = selectedInjectedItem();
  if (!item) return;
  await setToolEnabledState(item, enabled);
}

async function setInjectionEnabledState(item: InjectedPromptItem, enabled: boolean) {
  if (!selectedAgentId.value || !canToggleInjectionItem(item) || injectionSavingId.value) return;
  if (!props.workspaceRef) return;

  injectionSavingId.value = item.id;
  injectionError.value = "";
  const record = toolMetaRecord(item.meta);
  const previous = record?.enabled;
  if (record) record.enabled = enabled;
  try {
    await setAgentInjectionEnabled(props.workspaceRef, selectedAgentId.value, item.id, enabled);
    envRenderedContent.value = "";
    await loadInjectedItems();
    void loadPromptStats();
    if (envPreviewMode.value === "rendered") void loadRenderedEnvPrompt();
  } catch (e) {
    console.error("set_agent_injection_enabled failed:", e);
    if (record) record.enabled = previous;
    injectionError.value = t("agent.injection.saveFailed", normalizeAppError(e).message);
  } finally {
    injectionSavingId.value = null;
  }
}

async function setSelectedInjectionEnabledState(enabled: boolean) {
  const item = selectedInjectedItem();
  if (!item) return;
  await setInjectionEnabledState(item, enabled);
}

async function selectRuleItem(rule: RuleItem) {
  selected.value = { type: "rule", rule };
  closeRuleContextMenu();
  ruleEditing.value = false;
  confirmingDeleteRule.value = null;
  ruleContentLoading.value = true;
  try {
    ruleContent.value = props.workspaceRef
      ? await readRule(props.workspaceRef, selectedAgentId.value, ruleKey(rule))
      : await readAppRule(selectedAgentId.value, ruleKey(rule));
  } catch (e) {
    ruleContent.value = t("common.readFailed", normalizeAppError(e).message);
  } finally {
    ruleContentLoading.value = false;
  }
}

async function setRuleEnabledState(rule: RuleItem, enabled: boolean) {
  if (!canToggleRule(rule)) return;
  if (!props.workspaceRef) return;
  const previous = rule.enabled;
  rule.enabled = enabled;
  try {
    await setRuleEnabled(props.workspaceRef, selectedAgentId.value, ruleKey(rule), enabled);
    void loadPromptStats();
  } catch (e) {
    console.error("set_rule_enabled failed:", e);
    rule.enabled = previous;
  }
}

function startEditRule() {
  if (!canEditRule(selectedRule())) return;
  closeRuleContextMenu();
  ruleEditing.value = true;
  ruleEditContent.value = ruleContent.value;
}

async function saveEditRule() {
  const sr = selectedRule();
  if (!sr || !canEditRule(sr)) return;
  if (!props.workspaceRef) return;
  try {
    await saveRule(props.workspaceRef, selectedAgentId.value, sr.fileName, ruleEditContent.value);
    ruleContent.value = ruleEditContent.value;
    ruleEditing.value = false;
    await loadRules();
    await loadPromptStats();
    const updated = ruleItems.value.find(r => ruleKey(r) === ruleKey(sr));
    if (updated) selected.value = { type: "rule", rule: updated };
  } catch (e) {
    console.error("save_rule failed:", e);
    error.value = normalizeAppError(e).message;
  }
}

function cancelEditRule() {
  ruleEditing.value = false;
}

function startCreateRule() {
  if (!props.workspaceRef) return;
  closeRuleContextMenu();
  confirmingDeleteRule.value = null;
  collapsedSections.value.rules = false;
  ruleCreating.value = true;
  ruleNewName.value = "";
  ruleNewContent.value = "";
}

async function commitCreateRule() {
  const name = ruleNewName.value.trim();
  if (!name) return;
  if (!props.workspaceRef) return;
  try {
    const content = ruleNewContent.value || `# ${name}\n\n`;
    const item = await saveRule(props.workspaceRef, selectedAgentId.value, name, content);
    ruleCreating.value = false;
    await loadRules();
    await loadPromptStats();
    selectRuleItem(item);
  } catch (e) {
    console.error("save_rule failed:", e);
    error.value = normalizeAppError(e).message;
  }
}

async function removeRule(rule: RuleItem) {
  closeRuleContextMenu();
  if (!canEditRule(rule)) return;
  if (!props.workspaceRef) return;
  try {
    await deleteRule(props.workspaceRef, selectedAgentId.value, rule.fileName);
    if (selectedRuleKey() === ruleKey(rule)) {
      selected.value = null;
      ruleContent.value = "";
      ruleEditing.value = false;
    }
    await loadRules();
    await loadPromptStats();
  } catch (e) {
    console.error("delete_rule failed:", e);
    error.value = normalizeAppError(e).message;
  }
}

interface AgentRuleInternalDragData {
  sourceKey: string;
  label: string;
  originalRules: RuleItem[];
  previewRules?: RuleItem[];
}

async function persistRuleOrder(rules: RuleItem[]) {
  if (!props.workspaceRef) return;
  ruleItems.value = [...rules];
  const fileNames = rules.map(r => ruleKey(r));
  try {
    await setRuleOrder(props.workspaceRef, selectedAgentId.value, fileNames);
    void loadPromptStats();
  } catch (e) {
    console.error("set_rule_order failed:", e);
    await loadRules();
  }
}

function onRulePointerDown(index: number, rule: RuleItem, event: PointerEvent) {
  if (!props.workspaceRef || rule.readOnly) return;
  const originalRules = [...ruleItems.value];
  internalDrag.start(event, {
    id: `agent-rule:${ruleKey(rule)}`,
    payload: {
      type: AGENT_RULE_INTERNAL_DRAG_TYPE,
      data: {
        sourceKey: ruleKey(rule),
        label: rule.title,
        originalRules,
      } satisfies AgentRuleInternalDragData,
    },
    preview: { label: rule.title, kind: "item" },
    allowedOperations: ["move"],
    onActivated: () => {
      closeRuleContextMenu();
      ruleDragIndex.value = index;
    },
    onFinished: () => {
      rulePreviewOrder.value = null;
      ruleDragIndex.value = null;
      ruleDragOverIndex.value = null;
    },
  });
}

const ruleDropTarget: InternalDropTargetRegistration<
  AgentRuleInternalDragData,
  { targetKey: string; position: "before" | "after" }
> = {
  id: "agent-rule-list",
  root: () => agentViewRef.value,
  accepts: (source) => source.payload.type === AGENT_RULE_INTERNAL_DRAG_TYPE,
  resolve: ({ source, point, hit }) => {
    const row = hit.closest<HTMLElement>(".rule-item[data-rule-index]");
    if (!row || !agentViewRef.value?.contains(row)) {
      if (!hit.closest(".rule-drag-zone")) return null;
      const data = source.payload.data;
      const remaining = data.originalRules.filter((rule) => ruleKey(rule) !== data.sourceKey);
      const last = remaining[remaining.length - 1];
      return last
        ? {
            key: `rule:${ruleKey(last)}:after`,
            operation: "move",
            intent: { targetKey: ruleKey(last), position: "after" },
          }
        : null;
    }
    const index = Number(row.dataset.ruleIndex);
    const targetKey = row.dataset.ruleKey ?? "";
    const data = source.payload.data;
    if (!Number.isInteger(index) || !targetKey) return null;
    if (targetKey === data.sourceKey) {
      const current = internalDrag.activeTarget.value?.decision;
      return current?.key.startsWith("rule:")
        ? current as InternalDropDecision<{ targetKey: string; position: "before" | "after" }>
        : null;
    }
    const bounds = row.getBoundingClientRect();
    const position = point.y < bounds.top + bounds.height / 2 ? "before" : "after";
    return { key: `rule:${targetKey}:${position}`, operation: "move", intent: { targetKey, position } };
  },
  onTargetChange: (decision) => {
    const source = internalDrag.source.value;
    if (source?.payload.type !== AGENT_RULE_INTERNAL_DRAG_TYPE) return;
    const data = source.payload.data as AgentRuleInternalDragData;
    if (!decision) {
      rulePreviewOrder.value = null;
      ruleDragIndex.value = data.originalRules.findIndex((rule) => ruleKey(rule) === data.sourceKey);
      return;
    }
    const moved = data.originalRules.find((rule) => ruleKey(rule) === data.sourceKey);
    if (!moved) return;
    const next = data.originalRules.filter((rule) => ruleKey(rule) !== data.sourceKey);
    const targetIndex = next.findIndex((rule) => ruleKey(rule) === decision.intent.targetKey);
    if (targetIndex < 0) return;
    const insertIndex = targetIndex + (decision.intent.position === "after" ? 1 : 0);
    next.splice(insertIndex, 0, moved);
    data.previewRules = next;
    rulePreviewOrder.value = next;
    ruleDragIndex.value = insertIndex;
    ruleDragOverIndex.value = null;
  },
  drop: async ({ source }) => {
    await persistRuleOrder(source.payload.data.previewRules ?? source.payload.data.originalRules);
  },
  previewMode: ({ hit }) => hit.closest(".rule-drag-zone") ? "floating-with-gap" : "floating",
};

function openRuleContextMenu(event: MouseEvent, rule: RuleItem | null = null) {
  event.preventDefault();
  event.stopPropagation();
  confirmingDeleteRule.value = null;
  ruleCreating.value = false;
  ruleContextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    rule,
  };
}

function onRuleListContextMenu(event: MouseEvent) {
  const target = event.target;
  if (
    target instanceof Element
    && target.closest(".rule-item, .rule-injected-item, .inline-create-row")
  ) {
    return;
  }
  openRuleContextMenu(event);
}

async function requestDeleteRuleFromContext() {
  const rule = ruleContextMenu.value?.rule;
  if (!rule || !canEditRule(rule)) return;
  closeRuleContextMenu();
  if (selectedRuleKey() !== ruleKey(rule)) {
    await selectRuleItem(rule);
  }
  confirmingDeleteRule.value = ruleKey(rule);
}

function onResizeStart(e: MouseEvent, target: "sidebar" | "dir") {
  closeRuleContextMenu();
  resizing = target;
  resizeStartX = e.clientX;
  resizeStartWidth = target === "sidebar" ? sidebarWidth.value : dirPanelWidth.value;
  document.addEventListener("mousemove", onResizeMove);
  document.addEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "col-resize";
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
}

function onResizeMove(e: MouseEvent) {
  if (!resizing) return;
  const delta = e.clientX - resizeStartX;
  const newWidth = Math.max(80, resizeStartWidth + delta);
  if (resizing === "sidebar") {
    sidebarWidth.value = Math.min(newWidth, 300);
  } else {
    dirPanelWidth.value = Math.min(newWidth, 500);
  }
}

function onResizeEnd() {
  resizing = null;
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "";
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

function refreshAll() {
  closeRuleContextMenu();
  loadSystemPrompt();
  loadEnvTemplate();
  envRenderedContent.value = "";
  envRenderedLoading.value = false;
  envRenderedRequestId += 1;
  if (envPreviewMode.value === "rendered") {
    loadRenderedEnvPrompt();
  }
  loadPromptStats();
  loadInjectedItems();
  loadRules();
}

function formatDate(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts * 1000);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  if (isToday) {
    return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
}

onMounted(async () => {
  unregisterRuleDropTarget = internalDrag.registerTarget(ruleDropTarget);
  agentViewUnmounted = false;
  await loadAllAgents();
  if (agentViewUnmounted) return;
  if (selectedAgentId.value) {
    loadAgentData();
  }
  const releasePluginsChanged = await listen<void>("plugins-changed", async () => {
    await loadAllAgents();
    if (selectedAgentId.value) {
      refreshAll();
    }
  });
  if (agentViewUnmounted) {
    releasePluginsChanged();
  } else {
    pluginsChangedUnlisten = releasePluginsChanged;
  }
  const releaseAgentsChanged = await listen<void>("agents-changed", async () => {
    await loadAllAgents();
    if (selectedAgentId.value) {
      refreshAll();
    }
  });
  if (agentViewUnmounted) {
    releaseAgentsChanged();
  } else {
    agentsChangedUnlisten = releaseAgentsChanged;
  }
});

onUnmounted(() => {
  unregisterRuleDropTarget?.();
  unregisterRuleDropTarget = null;
  agentViewUnmounted = true;
  closeRuleContextMenu();
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
  releaseSelectionLock?.();
  releaseSelectionLock = null;
  pluginsChangedUnlisten?.();
  pluginsChangedUnlisten = null;
  agentsChangedUnlisten?.();
  agentsChangedUnlisten = null;
});

watch(
  () => [
    props.workspaceRef?.checkoutId ?? "",
    props.workspaceRef?.expectedGeneration ?? -1,
    props.workingDir,
  ] as const,
  async () => {
    resetAgentDetailState();
    await loadAllAgents();
    if (selectedAgentId.value) await loadAgentData();
  },
);

watch(
  () => props.workspaceRef ? "" : props.agentList.map((agent) => agent.id).join("\u0000"),
  async () => {
    if (props.workspaceRef) return;
    resetAgentDetailState();
    await loadAllAgents();
    if (selectedAgentId.value) await loadAgentData();
  },
);

watch(
  () => props.active,
  (active) => {
    if (active && selectedAgentId.value) refreshAll();
  },
);

watch(
  () => [
    modelStore.selectedModelId,
    JSON.stringify(currentSubagentModels()),
  ],
  () => {
    if (!selectedAgentId.value) return;
    void loadPromptStats();
    void loadInjectedItems();
    if (envPreviewMode.value === "rendered") {
      void loadRenderedEnvPrompt();
    }
  },
);
</script>

<template>
  <div ref="agentViewRef" class="agent-view">
    <div class="agent-sidebar" :style="{ width: sidebarWidth + 'px' }">
      <div class="sidebar-title">Agent</div>
      <div v-if="allAgents.length === 0" class="sidebar-empty">{{ t("common.loading") }}</div>
      <button
        v-for="ag in allAgents"
        :key="ag.id"
        type="button"
        class="agent-tab"
        :class="{ active: selectedAgentId === ag.id }"
        @click="switchAgent(ag.id)"
      >
        <div class="agent-tab-head">
          <div class="agent-tab-name">{{ ag.name }}</div>
          <span v-if="agentProjectTypesLabel(ag)" class="agent-tab-project-types">
            {{ agentProjectTypesLabel(ag) }}
          </span>
        </div>
        <div class="agent-tab-desc">{{ ag.description }}</div>
      </button>
    </div>
    <div class="resize-handle" @mousedown="onResizeStart($event, 'sidebar')"></div>

    <template v-if="selectedAgentId">
      <div class="dir-panel" :style="{ width: dirPanelWidth + 'px' }">
        <div class="dir-toolbar">
          <span class="dir-title">Context</span>
          <div class="dir-actions">
            <BaseButton class="dir-btn" :aria-label="t('agent.newRule')" @click="startCreateRule" :disabled="!props.workspaceRef" :title="t('agent.newRule')">+</BaseButton>
            <BaseButton class="dir-btn" :aria-label="t('common.refresh')" @click="refreshAll" :disabled="systemPromptLoading || ruleLoading" :title="t('common.refresh')">
              <span :class="{ spinning: systemPromptLoading || ruleLoading }">&#8635;</span>
            </BaseButton>
          </div>
        </div>
        <div class="dir-content">
          <div class="section-label">System Prompt</div>
          <button
            type="button"
            class="kb-item prompt-item"
            :class="{ selected: selected?.type === 'prompt' }"
            @click="selectPrompt"
          >
            <span class="prompt-icon">&#9672;</span>
            <span class="item-title">{{ t("agent.systemPrompt") }}</span>
          </button>

          <div class="rule-section" @contextmenu.prevent="onRuleListContextMenu">
            <button
              type="button"
              class="section-header"
              :aria-expanded="!isSectionCollapsed('rules')"
              @click="toggleSection('rules')"
            >
              <span class="section-chevron" :class="{ collapsed: isSectionCollapsed('rules') }">&#9662;</span>
              <span class="section-name">Rule</span>
              <span v-if="ruleSectionEntryCount" class="section-count">{{ ruleSectionEntryCount }}</span>
            </button>
            <template v-if="!isSectionCollapsed('rules')">
              <div v-if="(ruleLoading || injectedLoading) && ruleSectionEntryCount === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
              <div class="rule-drag-zone">
                <button
                  v-for="(rule, idx) in displayedRuleItems"
                  :key="ruleKey(rule)"
                  type="button"
                  class="kb-item rule-item"
                  :class="{
                    selected: selected?.type === 'rule' && selectedRuleKey() === ruleKey(rule),
                    'rule-context-target': ruleKey(ruleContextMenu?.rule) === ruleKey(rule) && selectedRuleKey() !== ruleKey(rule),
                    'rule-disabled': !rule.enabled,
                    'rule-dragging': ruleDragIndex === idx && internalDrag.previewMode.value === 'floating',
                    'rule-drop-gap': ruleDragIndex === idx && internalDrag.previewMode.value === 'floating-with-gap',
                    'rule-drag-over': ruleDragOverIndex === idx && ruleDragIndex !== idx,
                  }"
                  :data-rule-index="idx"
                  :data-rule-key="ruleKey(rule)"
                  @contextmenu.prevent.stop="openRuleContextMenu($event, rule)"
                  @click.stop="selectRuleItem(rule)"
                >
                  <span
                    class="rule-order-num"
                    title="Drag to reorder"
                    @pointerdown.stop="onRulePointerDown(idx, rule, $event)"
                  >{{ idx + 1 }}</span>
                  <label class="rule-toggle-label" @click.stop>
                    <BaseCheckbox
                      :model-value="rule.enabled"
                      :disabled="!canToggleRule(rule)"
                      :aria-label="rule.enabled ? t('common.enabled') : t('common.disabled')"
                      :title="!canToggleRule(rule) ? t('agent.rulePluginEnableManagedByPlugin') : undefined"
                      @update:model-value="setRuleEnabledState(rule, $event)"
                    />
                  </label>
                  <span class="item-title" :class="{ 'rule-title-disabled': !rule.enabled }">{{ rule.title }}</span>
                  <span v-if="!rule.enabled" class="rule-off-badge">OFF</span>
                </button>
              </div>
              <button
                v-for="item in injectedRuleItems"
                :key="item.id"
                type="button"
                class="kb-item injected-item rule-injected-item"
                :class="{
                  selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id,
                  'injection-disabled': !injectionItemEnabled(item),
                }"
                @click="selectInjectedItem(item)"
              >
                <label class="rule-toggle-label" @click.stop>
                  <BaseCheckbox
                    :model-value="injectionItemEnabled(item)"
                    :disabled="!canToggleInjectionItem(item) || injectionSavingId === item.id"
                    :aria-label="injectionItemEnabled(item) ? t('common.enabled') : t('common.disabled')"
                    @update:model-value="setInjectionEnabledState(item, $event)"
                  />
                </label>
                <span class="item-title" :class="{ 'rule-title-disabled': !injectionItemEnabled(item) }">{{ item.title }}</span>
                <span class="injected-kind-badge">{{ injectedItemBadge(item.kind) }}</span>
                <span v-if="!injectionItemEnabled(item)" class="rule-off-badge">OFF</span>
              </button>
              <div v-if="ruleCreating" class="kb-item inline-create-row">
                <input
                  v-model="ruleNewName"
                  class="inline-input"
                  :placeholder="t('agent.ruleName')"
                  @keydown.enter="commitCreateRule"
                  @keydown.escape="ruleCreating = false"
                  autofocus
                />
              </div>
            </template>
          </div>

          <div class="injected-section">
            <button
              type="button"
              class="section-header"
              :aria-expanded="!isSectionCollapsed('injected')"
              @click="toggleSection('injected')"
            >
              <span class="section-chevron" :class="{ collapsed: isSectionCollapsed('injected') }">&#9662;</span>
              <span class="section-name">{{ t("agent.injected") }}</span>
              <span class="section-count">{{ injectedContextEntryCount }}</span>
            </button>
            <template v-if="!isSectionCollapsed('injected')">
              <button
                type="button"
                class="kb-item injected-item"
                :class="{
                  selected: selected?.type === 'env',
                  'injection-disabled': envInjectionItem && !injectionItemEnabled(envInjectionItem),
                }"
                @click="selectEnv"
              >
                <label v-if="envInjectionItem" class="rule-toggle-label" @click.stop>
                  <BaseCheckbox
                    :model-value="injectionItemEnabled(envInjectionItem)"
                    :disabled="!canToggleInjectionItem(envInjectionItem) || injectionSavingId === envInjectionItem.id"
                    :aria-label="injectionItemEnabled(envInjectionItem) ? t('common.enabled') : t('common.disabled')"
                    @update:model-value="setInjectionEnabledState(envInjectionItem, $event)"
                  />
                </label>
                <span v-else class="prompt-icon injected-icon">&#9881;</span>
                <span class="item-title" :class="{ 'rule-title-disabled': envInjectionItem && !injectionItemEnabled(envInjectionItem) }">{{ t("agent.envTemplate") }}</span>
                <span class="injected-kind-badge">{{ t("agent.injected.context") }}</span>
                <span v-if="envInjectionItem && !injectionItemEnabled(envInjectionItem)" class="rule-off-badge">OFF</span>
              </button>
              <div v-if="injectedLoading && injectedContextItems.length === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
              <button
                v-for="item in injectedContextItems"
                :key="item.id"
                type="button"
                class="kb-item injected-item"
                :class="{
                  selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id,
                  'injection-disabled': !injectionItemEnabled(item),
                }"
                @click="selectInjectedItem(item)"
              >
                <label class="rule-toggle-label" @click.stop>
                  <BaseCheckbox
                    :model-value="injectionItemEnabled(item)"
                    :disabled="!canToggleInjectionItem(item) || injectionSavingId === item.id"
                    :aria-label="injectionItemEnabled(item) ? t('common.enabled') : t('common.disabled')"
                    @update:model-value="setInjectionEnabledState(item, $event)"
                  />
                </label>
                <span class="item-title" :class="{ 'rule-title-disabled': !injectionItemEnabled(item) }">{{ item.title }}</span>
                <span class="injected-kind-badge">{{ injectedItemBadge(item.kind) }}</span>
                <span v-if="!injectionItemEnabled(item)" class="rule-off-badge">OFF</span>
              </button>
            </template>
          </div>

          <div v-if="injectedLoading && availableToolItems.length === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
          <div v-for="group in toolGroups" :key="group.key" class="tool-group">
            <button
              type="button"
              class="section-header"
              :aria-expanded="!isSectionCollapsed(group.key)"
              @click="toggleSection(group.key)"
            >
              <span class="section-chevron" :class="{ collapsed: isSectionCollapsed(group.key) }">&#9662;</span>
              <span class="section-name">{{ group.label }}</span>
              <span class="section-count">{{ group.items.length }}</span>
            </button>
            <template v-if="!isSectionCollapsed(group.key)">
              <button
                v-for="(item, idx) in group.items"
                :key="item.id"
                type="button"
                class="kb-item injected-item tool-item"
                :class="{
                  selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id,
                  'tool-disabled': !toolItemEnabled(item),
                  'tool-unavailable': !toolItemRuntimeAvailable(item),
                }"
                @click="selectInjectedItem(item)"
              >
                <span class="tool-order-num">{{ idx + 1 }}</span>
                <label class="rule-toggle-label" @click.stop>
                  <BaseCheckbox
                    :model-value="toolItemEnabled(item)"
                    :disabled="!canToggleToolEnabled(item) || toolEnabledSavingId === item.id"
                    :aria-label="toolItemEnabled(item) ? t('common.enabled') : t('common.disabled')"
                    :title="!canToggleToolEnabled(item) ? t('agent.tool.enableFixed') : undefined"
                    @update:model-value="setToolEnabledState(item, $event)"
                  />
                </label>
                <span class="item-title tool-title" :class="{ 'rule-title-disabled': !toolItemEnabled(item) }">{{ toolItemDisplayTitle(item) }}</span>
                <span
                  v-if="group.key === 'tools:mcp' && toolItemMcpServer(item)"
                  class="tool-mcp-server-badge"
                >{{ toolItemMcpServer(item) }}</span>
                <span v-if="!toolItemEnabled(item)" class="rule-off-badge">OFF</span>
              </button>
            </template>
          </div>
        </div>
      </div>
      <div class="resize-handle" @mousedown="onResizeStart($event, 'dir')"></div>


      <div v-if="selected?.type === 'prompt'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedAgent?.name || selectedAgentId }}</span>
          <span class="preview-path">{{ t("agent.systemPrompt") }}</span>
          <span v-if="sourceBadgeLabel(selectedAgent?.source)" class="source-badge" :class="sourceBadgeClass(selectedAgent?.source)">{{ sourceBadgeLabel(selectedAgent?.source) }}</span>
        </div>
        <div class="preview-body" :class="{ 'is-loading': systemPromptLoading }">
          <div v-if="systemPromptLoading && !systemPromptContent" class="preview-loading">{{ t("common.loading") }}</div>
          <MarkdownRenderer v-show="!systemPromptLoading || systemPromptContent" :content="systemPromptContent" />
        </div>
        <div class="preview-footer">
          <span class="preview-meta">{{ selectedAgentId }}</span>
        </div>
      </div>

      <div v-else-if="selected?.type === 'env'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedAgent?.name || selectedAgentId }}</span>
          <span class="preview-path">env.md</span>
          <span v-if="sourceBadgeLabel(selectedAgent?.source)" class="source-badge" :class="sourceBadgeClass(selectedAgent?.source)">{{ sourceBadgeLabel(selectedAgent?.source) }}</span>
          <BaseSegmented
            class="env-preview-mode"
            :model-value="envPreviewMode"
            :options="envPreviewModeOptions"
            size="sm"
            @update:model-value="setEnvPreviewMode"
          />
        </div>
        <div v-if="envInjectionItem" class="rule-action-bar">
          <label class="skill-toggle">
            <BaseCheckbox
              :model-value="injectionItemEnabled(envInjectionItem)"
              :disabled="!canToggleInjectionItem(envInjectionItem) || injectionSavingId !== null"
              :aria-label="injectionItemEnabled(envInjectionItem) ? t('common.enabled') : t('common.disabled')"
              @update:model-value="setInjectionEnabledState(envInjectionItem, $event)"
            />
            <span>{{ injectionItemEnabled(envInjectionItem) ? t("common.enabled") : t("common.disabled") }}</span>
          </label>
          <span v-if="injectionError" class="tool-config-error">{{ injectionError }}</span>
        </div>
        <div class="preview-body env-template-body" :class="{ 'is-loading': envPreviewLoading }">
          <div v-if="envPreviewLoading && !envPreviewContent" class="preview-loading">{{ t("common.loading") }}</div>
          <pre v-show="!envPreviewLoading || envPreviewContent" class="env-template-pre" v-html="highlightedEnv(envPreviewContent)"></pre>
        </div>
        <div class="preview-footer">
          <span class="preview-meta">{{ selectedAgentId }}</span>
        </div>
      </div>

      <div v-else-if="selected?.type === 'rule'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedRule()?.title }}</span>
          <span class="preview-path">{{ selectedRule()?.fileName }}</span>
          <span v-if="sourceBadgeLabel(selectedRule()?.source)" class="source-badge" :class="sourceBadgeClass(selectedRule()?.source)">{{ sourceBadgeLabel(selectedRule()?.source) }}</span>
          <span v-if="selectedRule()?.readOnly" class="source-badge source-readonly">{{ t("agent.readOnly") }}</span>
          <BaseButton v-if="!ruleEditing && canEditRule(selectedRule())" class="preview-open-btn" :aria-label="t('agent.editRule')" @click="startEditRule" :title="t('common.edit')">&#9998;</BaseButton>
          <button class="preview-close" :aria-label="t('agent.closeRulePreview')" @click="selected = null; ruleContent = ''; ruleEditing = false" :title="t('common.close')">&times;</button>
        </div>
        <div class="rule-action-bar">
          <label class="skill-toggle">
            <BaseCheckbox
              :model-value="!!selectedRule()?.enabled"
              :disabled="!canToggleRule(selectedRule())"
              :aria-label="selectedRule()?.enabled ? t('common.enabled') : t('common.disabled')"
              :title="!canToggleRule(selectedRule()) ? t('agent.rulePluginEnableManagedByPlugin') : undefined"
              @update:model-value="setRuleEnabledState(selectedRule()!, $event)"
            />
            <span>{{ selectedRule()?.enabled ? t("common.enabled") : t("common.disabled") }}</span>
          </label>
          <div class="rule-action-spacer"></div>
          <template v-if="canEditRule(selectedRule()) && confirmingDeleteRule === selectedRuleKey()">
            <span class="rule-delete-confirm-text">{{ t("agent.deleteConfirm") }}</span>
            <BaseButton class="rule-delete-confirm-btn" variant="danger" @click="removeRule(selectedRule()!)">{{ t("common.confirm") }}</BaseButton>
            <BaseButton class="rule-delete-cancel-btn" @click="confirmingDeleteRule = null">{{ t("common.cancel") }}</BaseButton>
          </template>
          <BaseButton v-else-if="canEditRule(selectedRule())" class="rule-delete-btn" variant="danger" @click="confirmingDeleteRule = selectedRuleKey()">{{ t("common.delete") }}</BaseButton>
        </div>
        <div v-if="ruleEditing" class="preview-body rule-edit-body">
          <textarea
            v-model="ruleEditContent"
            class="rule-edit-textarea"
            :placeholder="t('agent.ruleContentPlaceholder')"
          ></textarea>
          <div class="rule-edit-actions">
            <BaseButton class="rule-save-btn" variant="primary" @click="saveEditRule">{{ t("common.save") }}</BaseButton>
            <BaseButton class="rule-cancel-btn" @click="cancelEditRule">{{ t("common.cancel") }}</BaseButton>
          </div>
        </div>
        <div v-else class="preview-body" :class="{ 'is-loading': ruleContentLoading }">
          <div v-if="ruleContentLoading && !ruleContent" class="preview-loading">{{ t("common.loading") }}</div>
          <MarkdownRenderer v-show="!ruleContentLoading || ruleContent" :content="ruleContent" />
        </div>
        <div class="preview-footer">
          <span class="preview-meta">Rule</span>
          <span class="preview-date">{{ formatDate(selectedRule()?.updatedAt || 0) }}</span>
        </div>
      </div>

      <div v-else-if="selected?.type === 'injected'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedInjectedItem()?.title }}</span>
          <span class="preview-path">{{ selectedInjectedItem()?.kind === "tools" ? selectedToolLoadLabel : injectedItemMeta(selectedInjectedItem()?.kind || "context") }}</span>
          <span class="source-badge source-runtime">{{ selectedInjectedItem()?.source === "builtIn" ? t("common.builtIn") : t("agent.runtime") }}</span>
          <span class="source-badge source-readonly">{{ t("agent.readOnly") }}</span>
          <button class="preview-close" :aria-label="t('agent.closePreview')" @click="selected = null" :title="t('common.close')">&times;</button>
        </div>
        <div v-if="selectedInjectedItem()?.kind !== 'tools'" class="rule-action-bar">
          <label class="skill-toggle">
            <BaseCheckbox
              :model-value="injectionItemEnabled(selectedInjectedItem())"
              :disabled="!canToggleInjectionItem(selectedInjectedItem()) || injectionSavingId !== null"
              :aria-label="injectionItemEnabled(selectedInjectedItem()) ? t('common.enabled') : t('common.disabled')"
              @update:model-value="setSelectedInjectionEnabledState"
            />
            <span>{{ injectionItemEnabled(selectedInjectedItem()) ? t("common.enabled") : t("common.disabled") }}</span>
          </label>
          <span v-if="injectionError" class="tool-config-error">{{ injectionError }}</span>
        </div>
        <div class="preview-body" :class="{ 'is-loading': injectedLoading }">
          <div v-if="injectedLoading && !selectedInjectedItem()?.content" class="preview-loading">{{ t("common.loading") }}</div>
          <template v-else-if="selectedInjectedItem()?.kind === 'tools' && selectedToolDefinition">
            <div class="tool-detail">
              <div class="tool-summary-line">{{ selectedToolLoadSummary }}</div>
              <div class="tool-summary-line">{{ selectedToolFooterMeta }}</div>

              <section v-if="!selectedToolRuntimeAvailable" class="tool-section tool-availability-section">
                <div class="tool-section-title">{{ t("agent.tool.currentlyUnavailable") }}</div>
                <div class="tool-availability-reason">{{ selectedToolUnavailableReason }}</div>
              </section>

              <section class="tool-section tool-load-config-section">
                <div class="tool-section-title">{{ t("agent.tool.loadConfig.title") }}</div>
                <div class="tool-load-config-row">
                  <BaseCheckbox
                    :model-value="selectedToolEnabled"
                    :disabled="!selectedToolCanToggleEnabled || toolEnabledSavingId !== null"
                    :aria-label="t('agent.tool.loadConfig.enabled')"
                    :title="!selectedToolCanToggleEnabled ? t('agent.tool.enableFixed') : undefined"
                    @update:model-value="setSelectedToolEnabledState"
                  />
                  <span class="tool-load-config-label">{{ t("agent.tool.loadConfig.enabled") }}</span>
                </div>
                <div v-if="!selectedToolEnabled" class="tool-load-config-summary tool-config-disabled-note">{{ t("agent.tool.loadConfig.disabledNote") }}</div>
                <div v-if="toolEnabledError" class="tool-config-error">{{ toolEnabledError }}</div>
                <div v-if="selectedToolCanConfigureDirectLoad" class="tool-load-config-row">
                  <BaseCheckbox
                    :model-value="selectedToolDirectLoadChecked"
                    :disabled="toolLoadSaving || !selectedToolEnabled"
                    :aria-label="t('agent.tool.loadConfig.directLoad')"
                    @update:model-value="setSelectedToolDirectLoadState"
                  />
                  <span class="tool-load-config-label">{{ t("agent.tool.loadConfig.directLoad") }}</span>
                </div>
                <div class="tool-load-config-summary">{{ selectedToolLoadConfigSummary }}</div>
                <div v-if="toolLoadConfigError" class="tool-config-error">{{ toolLoadConfigError }}</div>
              </section>

              <section class="tool-section">
                <div class="tool-section-title">{{ t("agent.tool.overview") }}</div>
                <MarkdownRenderer :content="selectedToolDescription" />
              </section>

              <section v-if="selectedToolDefinition.topLevelRequired.length > 0" class="tool-section">
                <div class="tool-section-title">{{ t("agent.tool.requiredParameters") }}</div>
                <div class="tool-required-list">
                  <code
                    v-for="name in selectedToolDefinition.topLevelRequired"
                    :key="name"
                    class="tool-required-item"
                  >{{ name }}</code>
                </div>
              </section>

              <section class="tool-section">
                <div class="tool-section-title">{{ t("agent.tool.parametersTitle") }}</div>
                <div v-if="selectedToolDefinition.parameterRows.length > 0" class="tool-parameter-list">
                  <div
                    v-for="row in selectedToolDefinition.parameterRows"
                    :key="row.path"
                    class="tool-parameter-row"
                    :style="{ paddingInlineStart: `${14 + row.depth * 14}px` }"
                  >
                    <div class="tool-parameter-head">
                      <code class="tool-parameter-path">{{ row.path }}</code>
                      <span class="tool-parameter-type">{{ row.typeLabel }}</span>
                      <span v-if="row.required" class="tool-parameter-required">{{ t("agent.tool.requiredTag") }}</span>
                    </div>
                    <div v-if="row.description" class="tool-parameter-desc">{{ row.description }}</div>
                    <div v-if="row.defaultValue !== null" class="tool-parameter-extra">
                      <span class="tool-parameter-extra-label">{{ t("agent.tool.default") }}</span>
                      <code>{{ row.defaultValue }}</code>
                    </div>
                    <div v-if="row.enumValues.length > 0" class="tool-parameter-extra">
                      <span class="tool-parameter-extra-label">{{ t("agent.tool.allowedValues") }}</span>
                      <code>{{ row.enumValues.join(", ") }}</code>
                    </div>
                  </div>
                </div>
                <div v-else class="tool-empty-state">{{ t("agent.tool.noParameters") }}</div>
              </section>

              <section class="tool-section">
                <div class="tool-section-title">{{ t("agent.tool.rawJson") }}</div>
                <pre class="tool-json-block ui-select-text">{{ selectedToolDefinition.rawJson }}</pre>
              </section>
            </div>
          </template>
          <MarkdownRenderer v-else v-show="!injectedLoading || selectedInjectedItem()?.content" :content="selectedInjectedItem()?.content || ''" />
        </div>
        <div class="preview-footer">
          <span class="preview-meta">{{ selectedInjectedItem()?.kind === "tools" ? selectedToolPreviewMeta : injectedItemMeta(selectedInjectedItem()?.kind || "context") }}</span>
        </div>
      </div>

      <div v-else class="preview-panel dashboard-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedAgent?.name || selectedAgentId }}</span>
          <span class="preview-path">{{ t("agent.dashboard.headerPath") }}</span>
          <span v-if="sourceBadgeLabel(selectedAgent?.source)" class="source-badge" :class="sourceBadgeClass(selectedAgent?.source)">{{ sourceBadgeLabel(selectedAgent?.source) }}</span>
        </div>
        <div class="preview-body dashboard-body" :class="{ 'is-loading': promptStatsLoading && !!promptStats }">
          <div v-if="promptStatsLoading && !promptStats" class="preview-loading">{{ t("agent.dashboard.loading") }}</div>
          <div v-else-if="promptStatsError && !promptStats" class="preview-loading">{{ promptStatsError }}</div>
          <template v-else>
            <div class="dashboard-header-block">
              <div class="dashboard-header-main">
                <div class="dashboard-title">{{ t("agent.dashboard.title") }}</div>
                <div class="dashboard-subtitle">{{ t("agent.dashboard.desc") }}</div>
              </div>
            </div>

            <div class="dashboard-top-grid">
              <section class="dashboard-card dashboard-card-total">
                <div class="dashboard-card-title">{{ t("agent.dashboard.total") }}</div>
                <div class="dashboard-hero-line">
                  <span class="dashboard-hero-value">{{ formatCount(promptDashboard.totalTokens) }}</span>
                  <span class="dashboard-hero-label">{{ t("agent.dashboard.totalUnit") }}</span>
                </div>
                <div class="dashboard-meta-grid">
                  <div class="dashboard-meta-cell">
                    <span class="dashboard-meta-label">{{ t("agent.dashboard.totalChars") }}</span>
                    <span class="dashboard-meta-value">{{ formatCount(promptDashboard.totalChars) }}</span>
                  </div>
                  <div class="dashboard-meta-cell">
                    <span class="dashboard-meta-label">{{ t("agent.dashboard.dominantPart") }}</span>
                    <span class="dashboard-meta-value dashboard-meta-value-secondary">
                      {{ dashboardPartTitle(promptDashboard.health.dominantPartKey) }}
                    </span>
                  </div>
                </div>
                <div class="dashboard-inline-note">{{ t("agent.dashboard.footerMeta") }}</div>
              </section>

              <section class="dashboard-card dashboard-card-health">
                <div class="dashboard-card-title">{{ t("agent.dashboard.healthTitle") }}</div>
                <div class="dashboard-health-row">
                  <div
                    class="dashboard-health-score"
                    :class="`dashboard-health-${promptDashboard.health.level}`"
                  >
                    {{ promptDashboard.health.score }}
                  </div>
                  <div class="dashboard-health-copy">
                    <div class="dashboard-health-label">
                      {{ dashboardHealthLabel(promptDashboard.health.level) }}
                    </div>
                    <div class="dashboard-health-summary">{{ dashboardHealthSummary }}</div>
                  </div>
                </div>
                <div class="dashboard-note-list">
                  <div
                    v-for="note in dashboardHealthNotes"
                    :key="note.text"
                    class="dashboard-note"
                    :class="`dashboard-note-${note.tone}`"
                  >
                    {{ note.text }}
                  </div>
                </div>
              </section>
            </div>

            <div class="dashboard-bottom-grid">
              <section class="dashboard-card dashboard-card-breakdown">
                <div class="dashboard-card-head">
                  <div class="dashboard-card-title">{{ t("agent.dashboard.composition") }}</div>
                  <div class="dashboard-card-meta">{{ formatTokenCount(promptDashboard.totalTokens) }}</div>
                </div>
                <div class="dashboard-breakdown-list">
                  <div
                    v-for="part in promptDashboard.parts"
                    :key="part.key"
                    class="dashboard-part-row"
                    :class="`dashboard-part-${part.key}`"
                  >
                    <div class="dashboard-part-head">
                      <div class="dashboard-part-main">
                        <span class="dashboard-part-name">{{ dashboardPartTitle(part.key) }}</span>
                        <span class="dashboard-part-meta">{{ dashboardPartMeta(part.key) }}</span>
                      </div>
                      <div class="dashboard-part-values">
                        <span class="dashboard-part-share">{{ formatPercent(part.share) }}</span>
                        <span class="dashboard-part-count">
                          {{ formatTokenCount(part.tokens) }} · {{ formatCount(part.chars) }} {{ t("agent.dashboard.charsUnit") }}
                        </span>
                      </div>
                    </div>
                    <div class="dashboard-part-bar">
                      <span class="dashboard-part-bar-fill" :style="{ width: formatPercent(part.share) }"></span>
                    </div>
                  </div>
                </div>
              </section>

              <section class="dashboard-card dashboard-card-runtime">
                <div class="dashboard-card-title">{{ t("agent.dashboard.runtimeTitle") }}</div>
                <div class="dashboard-stat-grid">
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.activeRules") }}</span>
                    <span class="dashboard-stat-value">
                      {{ formatCount(promptDashboard.enabledRuleCount) }} / {{ formatCount(promptDashboard.totalRuleCount) }}
                    </span>
                  </div>
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.injectedContext") }}</span>
                    <span class="dashboard-stat-value">{{ formatCount(promptDashboard.injectedContextCount) }}</span>
                  </div>
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.directTools") }}</span>
                    <span class="dashboard-stat-value">{{ formatCount(promptDashboard.directToolCount) }}</span>
                  </div>
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.lazyTools") }}</span>
                    <span class="dashboard-stat-value">{{ formatCount(promptDashboard.lazyToolCount) }}</span>
                  </div>
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.skillTools") }}</span>
                    <span class="dashboard-stat-value">{{ formatCount(promptDashboard.skillToolCount) }}</span>
                  </div>
                  <div class="dashboard-stat-cell">
                    <span class="dashboard-stat-label">{{ t("agent.dashboard.runtime.restrictedTools") }}</span>
                    <span class="dashboard-stat-value">
                      {{ formatCount(promptDashboard.unavailableToolCount) }} / {{ formatCount(promptDashboard.disabledToolCount) }}
                    </span>
                  </div>
                </div>
                <div v-if="promptStatsError" class="dashboard-inline-note">{{ promptStatsError }}</div>
              </section>
            </div>
          </template>
        </div>
      </div>

      <BaseContextMenu
        v-if="ruleContextMenu"
        class="agent-rule-ctx-menu"
        :x="ruleContextMenu.x"
        :y="ruleContextMenu.y"
        :z-index="80"
        @close="closeRuleContextMenu"
      >
          <button type="button" class="agent-rule-ctx-item" :disabled="!props.workspaceRef" @click="startCreateRule">
            <LucideIcon :icon="Plus" :size="13" />
            {{ t("agent.newRule") }}
          </button>
          <div v-if="canEditRule(ruleContextMenu.rule)" class="agent-rule-ctx-sep"></div>
          <button
            v-if="canEditRule(ruleContextMenu.rule)"
            type="button"
            class="agent-rule-ctx-item agent-rule-ctx-item-danger"
            @click="requestDeleteRuleFromContext"
          >
            <LucideIcon :icon="Trash2" :size="13" />
            {{ t("common.delete") }}
          </button>
      </BaseContextMenu>
    </template>

    <div v-else class="guide-panel" style="flex: 1;">
      <div class="guide-content static">
        <div class="guide-icon">A</div>
        <div class="guide-title">{{ t("agent.noAgent.title") }}</div>
        <div class="guide-desc">{{ t("agent.noAgent.desc") }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.agent-view {
  flex: 1;
  display: flex;
  flex-direction: row;
  height: 100%;
  min-width: 0;
  background: var(--bg-color);
  overflow: hidden;
}

.agent-sidebar {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 90%, var(--bg-color) 10%);
  overflow-y: auto;
}

.sidebar-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  padding: 12px 14px 8px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.sidebar-empty {
  padding: 20px 14px;
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0.5;
}

.agent-tab {
  appearance: none;
  width: 100%;
  padding: 10px 14px;
  cursor: pointer;
  text-align: left;
  border: none;
  border-left: 3px solid transparent;
  background: transparent;
  transition: all 0.12s;
  position: relative;
}

.agent-tab:hover {
  background: var(--hover-bg);
}

.agent-tab.active {
  background: var(--active-bg, var(--hover-bg));
  border-left-color: var(--accent-color);
}

.agent-tab-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.3;
}

.agent-tab-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.agent-tab-project-types {
  margin-left: auto;
  color: var(--text-tertiary, var(--text-secondary));
  font-size: 10px;
  font-weight: 500;
  line-height: 1.3;
  white-space: nowrap;
}

.agent-tab.active .agent-tab-name {
  color: var(--accent-color);
}

.agent-tab-desc {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.6;
  margin-top: 1px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dir-panel {
  min-width: 120px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.dir-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  flex-shrink: 0;
}

.dir-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
  flex: 1;
}

.dir-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.dir-btn {
  width: 28px;
  min-width: 28px;
  padding: 0;
  font-size: 14px;
}

.spinning {
  display: inline-block;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.dir-content {
  flex: 1;
  overflow-y: auto;
  padding-bottom: 20px;
}

.dir-empty-inline {
  padding: 8px 14px;
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0.5;
}

.section-label {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 14px 4px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  opacity: 0.7;
}

.section-header {
  appearance: none;
  width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px 4px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  opacity: 0.7;
  transition: opacity 0.1s;
}

.section-header:hover {
  opacity: 1;
}

.section-chevron {
  display: inline-block;
  width: 10px;
  font-size: 9px;
  line-height: 1;
  flex-shrink: 0;
  text-align: center;
  transition: transform 0.12s ease;
}

.section-chevron.collapsed {
  transform: rotate(-90deg);
}

.section-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rule-section,
.injected-section,
.tool-group {
  margin-top: 6px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 45%, transparent);
}

.section-count {
  font-size: 10px;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  padding: 0 5px;
  border-radius: 7px;
  line-height: 16px;
  opacity: 0.8;
}

.kb-item {
  appearance: none;
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  padding: 5px 10px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background 0.1s;
}

.kb-item:hover {
  background: var(--hover-bg);
}

.kb-item.selected {
  background: var(--accent-soft);
}

.kb-item.selected .item-title {
  color: var(--accent-color);
}

.prompt-item {
  padding: 6px 10px;
}

.prompt-icon {
  font-size: 12px;
  color: var(--accent-color);
  opacity: 0.6;
  flex-shrink: 0;
  width: 18px;
  text-align: center;
}

.kb-item.selected .prompt-icon {
  opacity: 1;
}

.item-title {
  font-size: 13px;
  color: var(--text-color);
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.inline-create-row {
  cursor: default;
}

.inline-input {
  flex: 1;
  min-width: 0;
  min-height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg, var(--bg-color)) 24%);
  color: var(--text-color);
  font-size: 13px;
  outline: none;
}

.inline-input:focus {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 12%, transparent);
}

.preview-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  background: var(--panel-bg);
  border-left: 1px solid var(--border-color);
}

.preview-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  flex-shrink: 0;
}

.preview-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-path {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.4;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-open-btn {
  width: 26px;
  min-width: 26px;
  padding: 0;
  font-size: 14px;
  flex-shrink: 0;
}

.preview-close {
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 16px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: all 0.1s;
}

.preview-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.preview-body {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  transition: opacity 0.15s ease;
}

.preview-body.is-loading {
  opacity: 0.5;
  pointer-events: none;
}

.preview-loading {
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0.5;
}

.preview-footer {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 16px;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  flex-shrink: 0;
}

.preview-meta {
  font-size: 12px;
  color: var(--text-color);
  opacity: 0.75;
}

.preview-date {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.4;
  flex: 1;
  text-align: right;
}

.dashboard-panel {
  background: var(--panel-bg);
}

.dashboard-body {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.dashboard-header-block {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.dashboard-header-main {
  min-width: 0;
  flex: 1;
}

.dashboard-title {
  font-size: 18px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--text-color);
  margin-bottom: 4px;
}

.dashboard-subtitle {
  max-width: 720px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.dashboard-top-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.08fr) minmax(0, 0.92fr);
  gap: 12px;
}

.dashboard-bottom-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.08fr) minmax(0, 0.92fr);
  gap: 12px;
}

.dashboard-top-grid > .dashboard-card,
.dashboard-bottom-grid > .dashboard-card {
  height: 100%;
}

.dashboard-card {
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.dashboard-card-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.dashboard-card-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.dashboard-card-meta {
  font-size: 12px;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.dashboard-hero-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.dashboard-hero-value {
  font-size: 34px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.dashboard-hero-label {
  font-size: 12px;
  color: var(--text-secondary);
}

.dashboard-meta-grid,
.dashboard-stat-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.dashboard-meta-cell,
.dashboard-stat-cell {
  min-width: 0;
  padding: 10px 11px;
  border: 1px solid color-mix(in srgb, var(--border-color) 80%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg, var(--bg-color)) 26%);
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.dashboard-meta-label,
.dashboard-stat-label {
  font-size: 11px;
  line-height: 1.35;
  color: var(--text-secondary);
}

.dashboard-meta-value,
.dashboard-stat-value {
  font-size: 17px;
  line-height: 1.2;
  font-weight: 700;
  color: var(--text-color);
  font-variant-numeric: tabular-nums;
}

.dashboard-meta-value-secondary {
  font-size: 12px;
  line-height: 1.45;
  font-weight: 600;
  word-break: break-word;
}

.dashboard-health-row {
  display: flex;
  align-items: center;
  gap: 14px;
}

.dashboard-health-score {
  width: 64px;
  height: 64px;
  border-radius: 16px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 26px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.dashboard-health-healthy {
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-soft) 80%, transparent);
  border-color: color-mix(in srgb, var(--accent-color) 24%, var(--border-color));
}

.dashboard-health-watch {
  color: var(--status-warn-fg);
  background: var(--status-warn-bg);
  border-color: var(--status-warn-border);
}

.dashboard-health-heavy {
  color: var(--status-danger-fg);
  background: var(--status-danger-bg);
  border-color: var(--status-danger-border);
}

.dashboard-health-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dashboard-health-label {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-color);
}

.dashboard-health-summary {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.dashboard-note-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.dashboard-note {
  padding: 9px 10px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg, var(--bg-color)) 26%);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.dashboard-note-good {
  color: var(--text-color);
}

.dashboard-note-warn {
  color: var(--status-warn-fg);
  background: color-mix(in srgb, var(--status-warn-bg) 70%, var(--panel-bg) 30%);
  border-color: var(--status-warn-border);
}

.dashboard-note-danger {
  color: var(--status-danger-fg);
  background: color-mix(in srgb, var(--status-danger-bg) 70%, var(--panel-bg) 30%);
  border-color: var(--status-danger-border);
}

.dashboard-breakdown-list {
  display: grid;
  gap: 10px;
}

.dashboard-card-runtime {
  justify-content: flex-start;
}

.dashboard-part-row {
  min-width: 0;
  padding: 10px 11px;
  border: 1px solid color-mix(in srgb, var(--border-color) 80%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg, var(--bg-color)) 24%);
  display: flex;
  flex-direction: column;
  gap: 8px;
  --dashboard-part-color: var(--accent-color);
}

.dashboard-part-base {
  --dashboard-part-color: var(--accent-color);
}

.dashboard-part-env {
  --dashboard-part-color: var(--status-warn-fg);
}

.dashboard-part-rules {
  --dashboard-part-color: color-mix(in srgb, var(--text-color) 72%, var(--accent-color) 28%);
}

.dashboard-part-knowledge {
  --dashboard-part-color: color-mix(in srgb, var(--accent-color) 64%, var(--text-secondary) 36%);
}

.dashboard-part-tools {
  --dashboard-part-color: color-mix(in srgb, var(--status-warn-fg) 58%, var(--accent-color) 42%);
}

.dashboard-part-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.dashboard-part-main,
.dashboard-part-values {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.dashboard-part-values {
  align-items: flex-end;
  text-align: right;
  flex-shrink: 0;
}

.dashboard-part-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.dashboard-part-meta,
.dashboard-part-count {
  font-size: 11px;
  line-height: 1.45;
  color: var(--text-secondary);
}

.dashboard-part-share {
  font-size: 12px;
  font-weight: 700;
  color: var(--dashboard-part-color);
  font-variant-numeric: tabular-nums;
}

.dashboard-part-bar {
  height: 6px;
  border-radius: 999px;
  overflow: hidden;
  background: color-mix(in srgb, var(--border-color) 46%, transparent);
}

.dashboard-part-bar-fill {
  display: block;
  height: 100%;
  min-width: 0;
  border-radius: inherit;
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--dashboard-part-color) 68%, transparent),
    var(--dashboard-part-color)
  );
}

.dashboard-inline-note {
  padding: 10px 11px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 74%, var(--input-bg, var(--bg-color)) 26%);
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

@media (max-width: 1180px) {
  .dashboard-top-grid,
  .dashboard-bottom-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 760px) {
  .dashboard-header-block {
    flex-direction: column;
    align-items: stretch;
  }

  .dashboard-meta-grid,
  .dashboard-stat-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .dashboard-note-list {
    grid-template-columns: minmax(0, 1fr);
  }

  .dashboard-health-row,
  .dashboard-part-head,
  .dashboard-card-head {
    flex-direction: column;
    align-items: flex-start;
  }

  .dashboard-part-values {
    align-items: flex-start;
    text-align: left;
  }
}

@media (max-width: 560px) {
  .dashboard-meta-grid,
  .dashboard-stat-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .dashboard-note-list {
    grid-template-columns: minmax(0, 1fr);
  }
}

.guide-panel {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  border-left: 1px solid var(--border-color);
}

.guide-content {
  appearance: none;
  border: 1px solid transparent;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 32px 28px;
  max-width: 340px;
  cursor: pointer;
  border-radius: 10px;
  background: transparent;
  transition: background 0.15s, border-color 0.15s;
}

.guide-content:hover {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, var(--border-color) 82%, transparent);
}

.guide-content.static {
  cursor: default;
}

.guide-content.static:hover {
  background: transparent;
  border-color: transparent;
}

.guide-icon {
  width: 40px;
  height: 40px;
  margin-bottom: 14px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--accent-soft) 70%, transparent);
  color: var(--accent-color);
  font-size: 18px;
  font-weight: 700;
}

.guide-title {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-color);
  margin-bottom: 8px;
}

.guide-desc {
  font-size: 13px;
  color: var(--text-secondary);
  opacity: 0.65;
  line-height: 1.6;
  margin-bottom: 20px;
}

/* ── Skill toggle ── */
.skill-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-secondary);
  cursor: pointer;
}

.rule-item {
  display: flex;
  align-items: center;
  gap: 8px;
  transition: opacity 0.15s;
}

.rule-item.rule-dragging {
  opacity: 0.35;
}

.rule-item.rule-drag-over {
  border-top: 2px solid var(--accent-color);
}

.rule-item.rule-drop-gap {
  opacity: 0;
  transition: none;
}

.rule-item.rule-context-target {
  background: color-mix(in srgb, var(--active-bg) 52%, var(--hover-bg) 48%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-border) 52%, transparent);
}

.rule-order-num {
  flex-shrink: 0;
  width: 18px;
  text-align: center;
  font-size: 10px;
  font-weight: 600;
  color: var(--text-secondary);
  opacity: 0.6;
  cursor: grab;
  touch-action: none;
}
.rule-order-num:hover {
  opacity: 1;
}

.rule-item.rule-disabled {
  opacity: 0.6;
}

.rule-toggle-label {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  cursor: pointer;
}

.rule-title-disabled {
  text-decoration: line-through;
  opacity: 0.6;
}

.rule-off-badge {
  font-size: 9px;
  padding: 1px 6px;
  border-radius: var(--radius-badge);
  font-weight: 600;
  line-height: 1.2;
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-secondary);
  opacity: 0.5;
}

.tool-mcp-server-badge {
  font-size: 9px;
  padding: 1px 6px;
  border-radius: var(--radius-badge);
  line-height: 1.2;
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  color: var(--text-secondary);
  opacity: 0.75;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 90px;
}

.injected-item {
  gap: 8px;
  cursor: pointer;
}

.injected-item.injection-disabled {
  opacity: 0.6;
}

.injected-icon {
  width: 18px;
}

.tool-item {
  padding: 4px 10px;
}

.tool-order-num {
  flex-shrink: 0;
  width: 18px;
  text-align: center;
  font-size: 10px;
  font-weight: 600;
  color: var(--text-secondary);
  opacity: 0.6;
  font-variant-numeric: tabular-nums;
}

.tool-title {
  font-family: var(--font-mono-editor);
  font-size: 12px;
}

.tool-item.tool-disabled {
  opacity: 0.6;
}

.tool-item.tool-unavailable .tool-title {
  color: var(--text-secondary);
}

/* Soften the enable checkboxes in the list: dozens of solid accent squares
   read as noise, so keep checked state as a translucent accent fill. */
.rule-toggle-label :deep(.base-checkbox.checked .base-checkbox-box) {
  background: color-mix(in srgb, var(--accent-color) 16%, transparent);
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-strong));
  color: color-mix(in srgb, var(--accent-color) 82%, var(--text-color) 18%);
}

.rule-toggle-label :deep(.base-checkbox:not(.checked) .base-checkbox-box) {
  background: transparent;
  border-color: color-mix(in srgb, var(--border-strong) 72%, transparent);
}

.injected-kind-badge {
  font-size: 9px;
  padding: 1px 6px;
  border-radius: var(--radius-badge);
  font-weight: 600;
  line-height: 1.2;
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-secondary);
  opacity: 0.75;
  text-transform: uppercase;
}

.rule-action-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
}

.rule-action-spacer {
  flex: 1;
}

.rule-delete-btn {
  min-width: 0;
}

.rule-delete-confirm-text {
  font-size: 12px;
  color: var(--status-danger-fg);
}

.rule-delete-confirm-btn {
  min-width: 0;
}

.rule-delete-cancel-btn {
  min-width: 0;
}

.rule-edit-body {
  display: flex;
  flex-direction: column;
  padding: 0 !important;
}

.rule-edit-textarea {
  flex: 1;
  width: 100%;
  padding: 16px 24px;
  font-size: 13px;
  font-family: var(--font-mono-editor);
  line-height: 1.6;
  border: none;
  outline: none;
  resize: none;
  background: var(--bg-color);
  color: var(--text-color);
}

.rule-edit-actions {
  display: flex;
  gap: 8px;
  padding: 8px 16px;
  border-top: 1px solid var(--border-color);
  justify-content: flex-end;
}

.rule-save-btn {
  min-width: 0;
}

.rule-cancel-btn {
  min-width: 0;
}

.env-template-body {
  padding: 0 !important;
}

.env-preview-mode {
  flex-shrink: 0;
}

.env-template-pre {
  margin: 0;
  padding: 20px 24px;
  font-size: 13px;
  font-family: var(--font-mono-editor);
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-color);
  background: transparent;
}

.tool-detail {
  display: flex;
  flex-direction: column;
  gap: 22px;
}

.tool-summary-line {
  font-size: 12px;
  color: var(--text-secondary);
  opacity: 0.8;
}

.tool-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.tool-section-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  letter-spacing: 0.5px;
  text-transform: uppercase;
  opacity: 0.82;
}

.tool-load-config-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
}

.tool-load-config-label {
  font-size: 13px;
  color: var(--text-color);
}

.tool-load-config-summary {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.tool-config-error {
  font-size: 12px;
  line-height: 1.5;
  color: var(--status-danger-fg);
}

.tool-config-disabled-note {
  color: var(--status-warn-fg);
}

.tool-availability-section {
  padding-inline-start: 10px;
  border-inline-start: 2px solid var(--status-warn-border);
}

.tool-availability-reason {
  font-size: 13px;
  line-height: 1.6;
  color: var(--status-warn-fg);
}

.tool-required-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.tool-required-item {
  margin: 0;
  padding: 3px 8px;
  border-radius: 6px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--bg-color) 24%);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-editor);
}

.tool-parameter-list {
  border: 1px solid color-mix(in srgb, var(--border-color) 90%, transparent);
  border-radius: 10px;
  overflow: hidden;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
}

.tool-parameter-row {
  padding-top: 10px;
  padding-right: 14px;
  padding-bottom: 12px;
}

.tool-parameter-row + .tool-parameter-row {
  border-top: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
}

.tool-parameter-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  flex-wrap: wrap;
}

.tool-parameter-path {
  font-size: 12px;
  font-family: var(--font-mono-editor);
  color: var(--text-color);
  word-break: break-word;
}

.tool-parameter-type {
  font-size: 12px;
  color: var(--text-secondary);
  font-family: var(--font-mono-editor);
}

.tool-parameter-required {
  font-size: 11px;
  font-weight: 600;
  color: var(--status-warn-fg);
}

.tool-parameter-desc {
  margin-top: 5px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.tool-parameter-extra {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-secondary);
}

.tool-parameter-extra-label {
  opacity: 0.78;
}

.tool-empty-state {
  padding: 12px 14px;
  font-size: 12px;
  color: var(--text-secondary);
  border: 1px solid color-mix(in srgb, var(--border-color) 90%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
}

.tool-json-block {
  margin: 0;
  padding: 14px 16px;
  border-radius: 10px;
  border: 1px solid color-mix(in srgb, var(--border-color) 90%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color) 18%);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-editor);
  line-height: 1.6;
  overflow: auto;
  white-space: pre;
}

:deep(.env-hl-var) {
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 10%, transparent);
  padding: 1px 4px;
  border-radius: 3px;
  font-weight: 600;
}

:deep(.env-hl-block) {
  color: var(--status-warn-fg);
  background: color-mix(in srgb, var(--status-warn-fg) 10%, transparent);
  padding: 1px 4px;
  border-radius: 3px;
  font-weight: 600;
}

.resize-handle {
  width: 0;
  flex-shrink: 0;
  cursor: col-resize;
  position: relative;
  z-index: 10;
}

.resize-handle::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  width: 6px;
  z-index: 10;
}

.resize-handle::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -1px;
  width: 2px;
  background: transparent;
  transition: background 0.15s;
}

.resize-handle:hover::after {
  background: color-mix(in srgb, var(--accent-color) 40%, transparent);
}

.source-badge {
  font-size: 9px;
  padding: 1px 6px;
  border-radius: var(--radius-badge);
  font-weight: 600;
  line-height: 1.2;
  flex-shrink: 0;
  vertical-align: middle;
  margin-left: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-secondary);
}

.source-app {
  border-color: var(--status-warn-border);
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
}

.source-project {
  border-color: var(--accent-border);
  background: var(--accent-soft);
  color: var(--accent-color);
}

.source-both {
  border-color: color-mix(in srgb, var(--accent-border) 65%, var(--status-warn-border) 35%);
  background: color-mix(in srgb, var(--accent-soft) 60%, var(--status-warn-bg) 40%);
  color: var(--text-color);
}

.source-plugin {
  border-color: color-mix(in srgb, var(--border-color) 82%, transparent);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-secondary);
}

.source-runtime {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
  color: var(--text-secondary);
}

.source-readonly {
  background: color-mix(in srgb, var(--accent-color) 10%, transparent);
  color: var(--accent-color);
}
</style>
