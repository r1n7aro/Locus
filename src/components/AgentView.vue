
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { listAgents, listSubagentDefs, getAgentEnvTemplate, getAgentSystemPrompt, getAgentSystemPromptStats, listAgentInjectedItems, setAgentToolDirectLoad, listRules, readRule, saveRule, deleteRule, setRuleEnabled, setRuleOrder } from "../services/agent";
import type { AgentInfo, AgentSystemPromptStats, InjectedPromptItem, InjectedToolLoadMode, RuleItem } from "../types";
import { getWarmup } from "../composables/warmupCache";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import BaseButton from "./ui/BaseButton.vue";
import BaseCheckbox from "./ui/BaseCheckbox.vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { acquireSelectionLock } from "../composables/useSelectionLock";
import { parseAgentToolDefinition } from "./agent/toolSchema";
import { buildAgentPromptDashboard, type AgentPromptHealthLevel, type AgentPromptPartKey } from "./agent/agentPromptDashboard";

const props = defineProps<{
  workingDir: string;
  agentList: AgentInfo[];
}>();

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
const promptStats = ref<AgentSystemPromptStats | null>(null);
const promptStatsLoading = ref(false);
const promptStatsError = ref("");
let promptStatsRequestId = 0;

// ── Env Template ──
const envTemplateContent = ref("");
const envTemplateLoading = ref(false);

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
const ruleLoading = ref(false);
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

// ── Injected ──
const injectedItems = ref<InjectedPromptItem[]>([]);
const injectedLoading = ref(false);
const toolLoadSaving = ref(false);
const toolLoadConfigError = ref("");
const availableToolItems = computed(() =>
  injectedItems.value.filter((item) => item.kind === "tools"),
);
const directToolItems = computed(() =>
  availableToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "direct"),
);
const lazyToolItems = computed(() =>
  availableToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "lazy"),
);
const skillToolItems = computed(() =>
  availableToolItems.value.filter((item) => toolMetaLoadMode(item.meta) === "skill"),
);
const injectedContextItems = computed(() =>
  injectedItems.value.filter((item) => item.kind !== "tools"),
);
const injectedContextEntryCount = computed(() =>
  injectedContextItems.value.length + 1,
);
const promptDashboard = computed(() =>
  buildAgentPromptDashboard(promptStats.value, ruleItems.value, injectedItems.value),
);

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

const sidebarWidth = ref(160);
const dirPanelWidth = ref(280);
let resizing: "sidebar" | "dir" | null = null;
let resizeStartX = 0;
let resizeStartWidth = 0;
let releaseSelectionLock: (() => void) | null = null;

function selectedRule(): RuleItem | null {
  return selected.value?.type === "rule" ? selected.value.rule : null;
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

const selectedToolLoadSummary = computed(() => {
  const mode = selectedToolLoadMode.value;
  if (mode === "lazy") return t("agent.tool.loadSummary.lazy");
  if (mode === "skill") return t("agent.tool.loadSummary.skill");
  return t("agent.tool.loadSummary.direct");
});

const selectedToolCanConfigureDirectLoad = computed(() => {
  const item = selectedInjectedItem();
  if (!item || item.kind !== "tools") return false;
  return toolMetaBoolean(item.meta, "canConfigureDirectLoad") === true;
});

const selectedToolDirectLoadChecked = computed(() => selectedToolLoadMode.value === "direct");

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

function injectedItemIcon(kind: InjectedPromptItem["kind"]): string {
  if (kind === "rule") return "◎";
  if (kind === "tools") return "◈";
  return "◌";
}

async function loadAllAgents() {
  try {
    const topLevel = await listAgents();
    const subLevel = await listSubagentDefs();
    allAgents.value = [...topLevel, ...subLevel];
    if (!selectedAgentId.value && allAgents.value.length > 0) {
      const def = allAgents.value.find(a => a.isDefault);
      selectedAgentId.value = def ? def.id : allAgents.value[0].id;
    }
  } catch (e) {
    console.error("loadAllAgents failed:", e);
  }
}

async function switchAgent(agentId: string) {
  selectedAgentId.value = agentId;
  selected.value = null;
  closeRuleContextMenu();
  ruleContent.value = "";
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
  systemPromptContent.value = "";
  envTemplateContent.value = "";
  injectedItems.value = [];
  promptStats.value = null;
  promptStatsError.value = "";
  promptStatsLoading.value = false;
  promptStatsRequestId += 1;
  await loadAgentData();
}

async function loadAgentData() {
  if (!selectedAgentId.value) return;
  loadSystemPrompt();
  loadEnvTemplate();
  loadPromptStats();
  loadInjectedItems();
  loadRules();
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
  closeRuleContextMenu();
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
  if (!envTemplateContent.value) loadEnvTemplate();
}

// ── Env Template ──
async function loadEnvTemplate() {
  if (!selectedAgentId.value) return;
  envTemplateLoading.value = true;
  try {
    envTemplateContent.value = await getAgentEnvTemplate(selectedAgentId.value);
  } catch (e) {
    envTemplateContent.value = t("common.loadFailed", normalizeAppError(e).message);
  } finally {
    envTemplateLoading.value = false;
  }
}

// ── System Prompt ──
async function loadSystemPrompt() {
  if (!selectedAgentId.value) return;
  systemPromptLoading.value = true;
  try {
    systemPromptContent.value = await getAgentSystemPrompt(selectedAgentId.value);
  } catch (e) {
    systemPromptContent.value = t("common.loadFailed", normalizeAppError(e).message);
  } finally {
    systemPromptLoading.value = false;
  }
}

async function loadPromptStats() {
  if (!selectedAgentId.value) return;
  const requestId = ++promptStatsRequestId;
  promptStatsLoading.value = true;
  try {
    const stats = await getAgentSystemPromptStats(selectedAgentId.value);
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
  ruleLoading.value = true;
  try {
    ruleItems.value = await listRules(selectedAgentId.value);
  } catch (e) {
    console.error("list_rules failed:", e);
    ruleItems.value = [];
  } finally {
    ruleLoading.value = false;
  }
}

async function loadInjectedItems() {
  if (!selectedAgentId.value) return;
  injectedLoading.value = true;
  try {
    const items = await listAgentInjectedItems(selectedAgentId.value);
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
    console.error("list_agent_injected_items failed:", e);
    injectedItems.value = [];
  } finally {
    injectedLoading.value = false;
  }
}

function selectInjectedItem(item: InjectedPromptItem) {
  selected.value = { type: "injected", item };
  toolLoadConfigError.value = "";
  closeRuleContextMenu();
  ruleEditing.value = false;
  ruleCreating.value = false;
  confirmingDeleteRule.value = null;
}

async function setSelectedToolDirectLoadState(directLoad: boolean) {
  const item = selectedInjectedItem();
  const tool = selectedToolDefinition.value;
  if (!selectedAgentId.value || !item || item.kind !== "tools" || !tool) return;
  if (!selectedToolCanConfigureDirectLoad.value || toolLoadSaving.value) return;

  toolLoadSaving.value = true;
  toolLoadConfigError.value = "";
  try {
    await setAgentToolDirectLoad(selectedAgentId.value, tool.name, directLoad);
    await loadInjectedItems();
  } catch (e) {
    console.error("set_agent_tool_direct_load failed:", e);
    toolLoadConfigError.value = t("agent.tool.loadConfigSaveFailed", normalizeAppError(e).message);
  } finally {
    toolLoadSaving.value = false;
  }
}

async function selectRuleItem(rule: RuleItem) {
  selected.value = { type: "rule", rule };
  closeRuleContextMenu();
  ruleEditing.value = false;
  confirmingDeleteRule.value = null;
  ruleContentLoading.value = true;
  try {
    ruleContent.value = await readRule(selectedAgentId.value, rule.fileName);
  } catch (e) {
    ruleContent.value = t("common.readFailed", normalizeAppError(e).message);
  } finally {
    ruleContentLoading.value = false;
  }
}

async function setRuleEnabledState(rule: RuleItem, enabled: boolean) {
  const previous = rule.enabled;
  rule.enabled = enabled;
  try {
    await setRuleEnabled(selectedAgentId.value, rule.fileName, enabled);
    void loadPromptStats();
  } catch (e) {
    console.error("set_rule_enabled failed:", e);
    rule.enabled = previous;
  }
}

function startEditRule() {
  closeRuleContextMenu();
  ruleEditing.value = true;
  ruleEditContent.value = ruleContent.value;
}

async function saveEditRule() {
  const sr = selectedRule();
  if (!sr) return;
  try {
    await saveRule(selectedAgentId.value, sr.fileName, ruleEditContent.value);
    ruleContent.value = ruleEditContent.value;
    ruleEditing.value = false;
    await loadRules();
    await loadPromptStats();
    const updated = ruleItems.value.find(r => r.fileName === sr.fileName);
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
  closeRuleContextMenu();
  confirmingDeleteRule.value = null;
  ruleCreating.value = true;
  ruleNewName.value = "";
  ruleNewContent.value = "";
}

async function commitCreateRule() {
  const name = ruleNewName.value.trim();
  if (!name) return;
  try {
    const content = ruleNewContent.value || `# ${name}\n\n`;
    const item = await saveRule(selectedAgentId.value, name, content);
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
  try {
    await deleteRule(selectedAgentId.value, rule.fileName);
    if (selectedRule()?.fileName === rule.fileName) {
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

function onRuleDragStart(index: number, e: DragEvent) {
  closeRuleContextMenu();
  ruleDragIndex.value = index;
  if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
}
function onRuleDragOver(index: number, e: DragEvent) {
  e.preventDefault();
  ruleDragOverIndex.value = index;
}
function onRuleDragLeave() {
  ruleDragOverIndex.value = null;
}
async function onRuleDrop(index: number) {
  const from = ruleDragIndex.value;
  ruleDragOverIndex.value = null;
  ruleDragIndex.value = null;
  if (from === null || from === index) return;
  const arr = [...ruleItems.value];
  const [moved] = arr.splice(from, 1);
  arr.splice(index, 0, moved);
  ruleItems.value = arr;
  const fileNames = arr.map(r => r.fileName);
  try {
    await setRuleOrder(selectedAgentId.value, fileNames);
    void loadPromptStats();
  } catch (e) {
    console.error("set_rule_order failed:", e);
    await loadRules();
  }
}
function onRuleDragEnd() {
  ruleDragIndex.value = null;
  ruleDragOverIndex.value = null;
}

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
    && target.closest(".rule-item, .inline-create-row")
  ) {
    return;
  }
  openRuleContextMenu(event);
}

async function requestDeleteRuleFromContext() {
  const rule = ruleContextMenu.value?.rule;
  if (!rule) return;
  closeRuleContextMenu();
  if (selectedRule()?.fileName !== rule.fileName) {
    await selectRuleItem(rule);
  }
  confirmingDeleteRule.value = rule.fileName;
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
  // Use background warmup cache if available
  const cachedAgents = getWarmup<AgentInfo[]>("agent:agents");
  const cachedSubagents = getWarmup<AgentInfo[]>("agent:subagents");
  if (cachedAgents && cachedSubagents) {
    allAgents.value = [...cachedAgents, ...cachedSubagents];
    if (!selectedAgentId.value && allAgents.value.length > 0) {
      const def = allAgents.value.find(a => a.isDefault);
      selectedAgentId.value = def ? def.id : allAgents.value[0].id;
    }
  } else {
    await loadAllAgents();
  }
  if (selectedAgentId.value) {
    loadAgentData();
  }
});

onUnmounted(() => {
  closeRuleContextMenu();
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
  releaseSelectionLock?.();
  releaseSelectionLock = null;
});

watch(
  () => props.workingDir,
  () => {
    loadAllAgents().then(() => {
      if (selectedAgentId.value) loadAgentData();
    });
  },
);
</script>

<template>
  <div class="agent-view">
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
            <BaseButton class="dir-btn" :aria-label="t('agent.newRule')" @click="startCreateRule" :title="t('agent.newRule')">+</BaseButton>
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
            <div class="section-label">
              <span>Rule</span>
              <span v-if="ruleItems.length" class="section-count">{{ ruleItems.length }}</span>
            </div>
            <div v-if="ruleLoading && ruleItems.length === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
            <div class="rule-drag-zone" @dragover.prevent>
              <button
                v-for="(rule, idx) in ruleItems"
                :key="rule.fileName"
                type="button"
                class="kb-item rule-item"
                :class="{
                  selected: selected?.type === 'rule' && selectedRule()?.fileName === rule.fileName,
                  'rule-context-target': ruleContextMenu?.rule?.fileName === rule.fileName && selectedRule()?.fileName !== rule.fileName,
                  'rule-disabled': !rule.enabled,
                  'rule-dragging': ruleDragIndex === idx,
                  'rule-drag-over': ruleDragOverIndex === idx && ruleDragIndex !== idx,
                }"
                draggable="true"
                @dragstart="onRuleDragStart(idx, $event)"
                @dragover="onRuleDragOver(idx, $event)"
                @dragleave="onRuleDragLeave"
                @drop.prevent="onRuleDrop(idx)"
                @dragend="onRuleDragEnd"
                @contextmenu.prevent.stop="openRuleContextMenu($event, rule)"
                @click.stop="selectRuleItem(rule)"
              >
                <span class="rule-order-num" title="Drag to reorder">{{ idx + 1 }}</span>
                <label class="rule-toggle-label" @click.stop>
                  <BaseCheckbox
                    :model-value="rule.enabled"
                    :aria-label="rule.enabled ? t('common.enabled') : t('common.disabled')"
                    @update:model-value="setRuleEnabledState(rule, $event)"
                  />
                </label>
                <span class="item-title" :class="{ 'rule-title-disabled': !rule.enabled }">{{ rule.title }}</span>
                <span v-if="!rule.enabled" class="rule-off-badge">OFF</span>
                <span class="item-date">{{ formatDate(rule.updatedAt) }}</span>
              </button>
            </div>
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
          </div>

          <div class="injected-section">
            <div class="section-label">
              <span>{{ t("agent.injected") }}</span>
              <span class="section-count">{{ injectedContextEntryCount }}</span>
            </div>
            <button
              type="button"
              class="kb-item injected-item"
              :class="{ selected: selected?.type === 'env' }"
              @click="selectEnv"
            >
              <span class="prompt-icon injected-icon">&#9881;</span>
              <span class="item-title">{{ t("agent.envTemplate") }}</span>
              <span class="injected-kind-badge">{{ t("agent.injected.context") }}</span>
            </button>
            <div v-if="injectedLoading && injectedContextItems.length === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
            <button
              v-for="item in injectedContextItems"
              :key="item.id"
              type="button"
              class="kb-item injected-item"
              :class="{ selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id }"
              @click="selectInjectedItem(item)"
            >
              <span class="prompt-icon injected-icon">{{ injectedItemIcon(item.kind) }}</span>
              <span class="item-title">{{ item.title }}</span>
              <span class="injected-kind-badge">{{ injectedItemBadge(item.kind) }}</span>
            </button>
          </div>

          <template v-if="injectedLoading || directToolItems.length > 0">
            <div class="section-label">
              <span>{{ t("agent.directTools") }}</span>
              <span v-if="directToolItems.length" class="section-count">{{ directToolItems.length }}</span>
            </div>
            <div v-if="injectedLoading && directToolItems.length === 0" class="dir-empty-inline">{{ t("common.loading") }}</div>
            <button
              v-for="item in directToolItems"
              :key="item.id"
              type="button"
              class="kb-item injected-item"
              :class="{ selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id }"
              @click="selectInjectedItem(item)"
            >
              <span class="prompt-icon injected-icon">{{ injectedItemIcon(item.kind) }}</span>
              <span class="item-title">{{ item.title }}</span>
            </button>
          </template>

          <template v-if="lazyToolItems.length > 0">
            <div class="section-label">
              <span>{{ t("agent.lazyTools") }}</span>
              <span class="section-count">{{ lazyToolItems.length }}</span>
            </div>
            <button
              v-for="item in lazyToolItems"
              :key="item.id"
              type="button"
              class="kb-item injected-item"
              :class="{ selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id }"
              @click="selectInjectedItem(item)"
            >
              <span class="prompt-icon injected-icon">{{ injectedItemIcon(item.kind) }}</span>
              <span class="item-title">{{ item.title }}</span>
            </button>
          </template>

          <template v-if="skillToolItems.length > 0">
            <div class="section-label">
              <span>{{ t("agent.skillTools") }}</span>
              <span class="section-count">{{ skillToolItems.length }}</span>
            </div>
            <button
              v-for="item in skillToolItems"
              :key="item.id"
              type="button"
              class="kb-item injected-item"
              :class="{ selected: selected?.type === 'injected' && selectedInjectedItem()?.id === item.id }"
              @click="selectInjectedItem(item)"
            >
              <span class="prompt-icon injected-icon">{{ injectedItemIcon(item.kind) }}</span>
              <span class="item-title">{{ item.title }}</span>
            </button>
          </template>
        </div>
      </div>
      <div class="resize-handle" @mousedown="onResizeStart($event, 'dir')"></div>


      <div v-if="selected?.type === 'prompt'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedAgent?.name || selectedAgentId }}</span>
          <span class="preview-path">{{ t("agent.systemPrompt") }}</span>
          <span v-if="selectedAgent?.source === 'app'" class="source-badge source-app">{{ t("common.builtIn") }}</span>
          <span v-else-if="selectedAgent?.source === 'project'" class="source-badge source-project">{{ t("common.project") }}</span>
          <span v-else-if="selectedAgent?.source === 'both'" class="source-badge source-both">{{ t("common.builtInAndProject") }}</span>
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
          <span v-if="selectedAgent?.source === 'app'" class="source-badge source-app">{{ t("common.builtIn") }}</span>
          <span v-else-if="selectedAgent?.source === 'project'" class="source-badge source-project">{{ t("common.project") }}</span>
          <span v-else-if="selectedAgent?.source === 'both'" class="source-badge source-both">{{ t("common.builtInAndProject") }}</span>
        </div>
        <div class="preview-body env-template-body" :class="{ 'is-loading': envTemplateLoading }">
          <div v-if="envTemplateLoading && !envTemplateContent" class="preview-loading">{{ t("common.loading") }}</div>
          <pre v-show="!envTemplateLoading || envTemplateContent" class="env-template-pre" v-html="highlightedEnv(envTemplateContent)"></pre>
        </div>
        <div class="preview-footer">
          <span class="preview-meta">{{ selectedAgentId }}</span>
        </div>
      </div>

      <div v-else-if="selected?.type === 'rule'" class="preview-panel">
        <div class="preview-header">
          <span class="preview-title">{{ selectedRule()?.title }}</span>
          <span class="preview-path">{{ selectedRule()?.fileName }}</span>
          <span v-if="selectedRule()?.source === 'app'" class="source-badge source-app">{{ t("common.builtIn") }}</span>
          <span v-else-if="selectedRule()?.source === 'project'" class="source-badge source-project">{{ t("common.project") }}</span>
          <BaseButton v-if="!ruleEditing" class="preview-open-btn" :aria-label="t('agent.editRule')" @click="startEditRule" :title="t('common.edit')">&#9998;</BaseButton>
          <button class="preview-close" :aria-label="t('agent.closeRulePreview')" @click="selected = null; ruleContent = ''; ruleEditing = false" :title="t('common.close')">&times;</button>
        </div>
        <div class="rule-action-bar">
          <label class="skill-toggle">
            <BaseCheckbox
              :model-value="!!selectedRule()?.enabled"
              :aria-label="selectedRule()?.enabled ? t('common.enabled') : t('common.disabled')"
              @update:model-value="setRuleEnabledState(selectedRule()!, $event)"
            />
            <span>{{ selectedRule()?.enabled ? t("common.enabled") : t("common.disabled") }}</span>
          </label>
          <div class="rule-action-spacer"></div>
          <template v-if="confirmingDeleteRule === selectedRule()?.fileName">
            <span class="rule-delete-confirm-text">{{ t("agent.deleteConfirm") }}</span>
            <BaseButton class="rule-delete-confirm-btn" variant="danger" @click="removeRule(selectedRule()!)">{{ t("common.confirm") }}</BaseButton>
            <BaseButton class="rule-delete-cancel-btn" @click="confirmingDeleteRule = null">{{ t("common.cancel") }}</BaseButton>
          </template>
          <BaseButton v-else class="rule-delete-btn" variant="danger" @click="confirmingDeleteRule = selectedRule()!.fileName">{{ t("common.delete") }}</BaseButton>
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
        <div class="preview-body" :class="{ 'is-loading': injectedLoading }">
          <div v-if="injectedLoading && !selectedInjectedItem()?.content" class="preview-loading">{{ t("common.loading") }}</div>
          <template v-else-if="selectedInjectedItem()?.kind === 'tools' && selectedToolDefinition">
            <div class="tool-detail">
              <div class="tool-summary-line">{{ selectedToolLoadSummary }}</div>
              <div class="tool-summary-line">{{ selectedToolFooterMeta }}</div>

              <section class="tool-section tool-load-config-section">
                <div class="tool-section-title">{{ t("agent.tool.loadConfig.title") }}</div>
                <div v-if="selectedToolCanConfigureDirectLoad" class="tool-load-config-row">
                  <BaseCheckbox
                    :model-value="selectedToolDirectLoadChecked"
                    :disabled="toolLoadSaving"
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
          <span v-if="selectedAgent?.source === 'app'" class="source-badge source-app">{{ t("common.builtIn") }}</span>
          <span v-else-if="selectedAgent?.source === 'project'" class="source-badge source-project">{{ t("common.project") }}</span>
          <span v-else-if="selectedAgent?.source === 'both'" class="source-badge source-both">{{ t("common.builtInAndProject") }}</span>
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
                </div>
                <div v-if="promptStatsError" class="dashboard-inline-note">{{ promptStatsError }}</div>
              </section>
            </div>
          </template>
        </div>
      </div>

      <Teleport to="body">
      <div
        v-if="ruleContextMenu"
        class="agent-rule-ctx-backdrop"
        @click="closeRuleContextMenu"
        @contextmenu.prevent="closeRuleContextMenu"
      >
        <div
          class="agent-rule-ctx-menu"
          :style="{ left: ruleContextMenu.x + 'px', top: ruleContextMenu.y + 'px' }"
          @click.stop
        >
          <button type="button" class="agent-rule-ctx-item" @click="startCreateRule">
            {{ t("agent.newRule") }}
          </button>
          <div v-if="ruleContextMenu.rule" class="agent-rule-ctx-sep"></div>
          <button
            v-if="ruleContextMenu.rule"
            type="button"
            class="agent-rule-ctx-item agent-rule-ctx-item-danger"
            @click="requestDeleteRuleFromContext"
          >
            {{ t("common.delete") }}
          </button>
        </div>
      </div>
      </Teleport>
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

.item-date {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.4;
  flex-shrink: 0;
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

.injected-item {
  gap: 8px;
  cursor: pointer;
}

.injected-icon {
  width: 18px;
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

.agent-rule-ctx-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
}

.agent-rule-ctx-menu {
  position: fixed;
  min-width: 156px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--elevated-bg, var(--panel-bg));
  box-shadow: 0 12px 28px rgba(0, 0, 0, 0.24);
}

.agent-rule-ctx-item {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.agent-rule-ctx-item:hover {
  background: var(--hover-bg);
}

.agent-rule-ctx-item-danger {
  color: var(--status-danger-fg);
}

.agent-rule-ctx-item-danger:hover {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

.agent-rule-ctx-sep {
  height: 1px;
  margin: 4px 0;
  background: var(--border-color);
}

.env-template-body {
  padding: 0 !important;
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

.source-runtime {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
  color: var(--text-secondary);
}

.source-readonly {
  background: color-mix(in srgb, var(--accent-color) 10%, transparent);
  color: var(--accent-color);
}
</style>
