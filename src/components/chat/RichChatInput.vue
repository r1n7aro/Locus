<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, useSlots, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import { t } from "../../i18n";
import { searchWorkspaceAssets } from "../../services/asset";
import { knowledgeQuery } from "../../services/knowledge";
import {
  listDirEntriesPage,
  type DirEntry,
} from "../../services/project";
import { useNotificationStore } from "../../stores/notification";
import type {
  AssetRefAttachment,
  ChatComposerSendPayload,
  ImageAttachment,
  KnowledgeDocumentType,
  KnowledgeSearchResult,
  SkillIntentItem,
  SkillManifest,
} from "../../types";
import {
  buildUserIntentMeta,
  detectActiveOperator,
  emptyComposerIntent,
  hasComposerIntent,
  insertInlineMention,
  mergeComposerIntent,
  normalizeComposerText,
  parseInlineIntentCommands,
  removeTextRange,
  type ActiveOperator,
  type CommandDef,
  type ComposerIntentState,
} from "../../composables/chatInputIntents";
import { extractChatAssetRefs } from "../../composables/chatAssetRefs";
import { rankSearchResults } from "../../composables/searchMatcher";
import { useCommandRegistry } from "../../composables/useCommandRegistry";
import {
  shouldSelectPopupOnEnter,
  shouldSubmitOnEnter,
  useChatInputSettings,
} from "../../composables/useChatInputSettings";
import { subscribeUnityEmbedAssetDrop } from "../../services/unity";
import AssetChip from "../AssetChip.vue";
import MentionPopup from "./MentionPopup.vue";
import ChatComposer from "./ChatComposer.vue";
import ChatInputShell from "./ChatInputShell.vue";

interface MentionSearchResult {
  relPath: string;
  name: string;
  parentPath: string;
  isDir: boolean;
  matchScore: number;
  meta?: string;
  entryKind: "asset" | "knowledge";
}

interface MentionDisplayEntry {
  relPath: string;
  name: string;
  parentPath?: string;
  isDir: boolean;
  meta?: string;
  canNavigate?: boolean;
  isCurrentPath?: boolean;
  entryKind?: "asset" | "knowledge";
}

const PASTE_THRESHOLD = 500;
const ASSET_REF_SYNC_CHANNEL = "locus-chat-asset-ref-drafts";
const ASSET_REF_SYNC_STORAGE_KEY = "locus:chatAssetRefDraftSync";
const UNITY_ASSET_REF_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;
const PROJECT_KNOWLEDGE_REF_ROOT_RE = /^(?:design|memory|skill|reference)\/.+\.md$/i;
const KNOWLEDGE_MENTION_TYPES: KnowledgeDocumentType[] = ["design", "memory", "skill", "reference"];

interface AssetRefSyncMessage {
  kind: "assetRefs";
  sourceId: string;
  syncKey: string;
  refs: AssetRefAttachment[];
  seq: number;
}

const props = withDefaults(defineProps<{
  modelValue: string;
  selectedAgentId: string;
  skills?: SkillManifest[];
  placeholder?: string;
  disabled?: boolean;
  isStreaming?: boolean;
  sendLabel?: string;
  cancelLabel?: string;
  allowImages?: boolean;
  maxImages?: number;
  showTopPlanBadge?: boolean;
  showSkillBadges?: boolean;
  compact?: boolean;
  showAction?: boolean;
  assetRefSyncKey?: string;
}>(), {
  skills: () => [],
  placeholder: "",
  disabled: false,
  isStreaming: false,
  sendLabel: "",
  cancelLabel: "",
  allowImages: true,
  maxImages: 5,
  showTopPlanBadge: true,
  showSkillBadges: true,
  compact: false,
  showAction: true,
  assetRefSyncKey: "",
});

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "send", payload: ChatComposerSendPayload): void;
  (e: "cancel"): void;
  (e: "clear"): void;
  (e: "compact"): void;
}>();

const composerRef = ref<InstanceType<typeof ChatComposer> | null>(null);
const notificationStore = useNotificationStore();
const slots = useSlots();
const { state: chatInputSettings } = useChatInputSettings();

const skillsRef = computed(() => props.skills);
const agentIdRef = computed(() => props.selectedAgentId);
const {
  allCommands,
  filteredCommands: getFilteredCommands,
  findExactAvailableCommand,
} = useCommandRegistry(skillsRef, agentIdRef);

const pastedContent = ref("");
const showPasteEditor = ref(false);
const imageAttachments = ref<ImageAttachment[]>([]);
const assetRefAttachments = ref<AssetRefAttachment[]>([]);
const previewImageIndex = ref<number | null>(null);
const composerIntent = ref<ComposerIntentState>(emptyComposerIntent());
const activeOperator = ref<ActiveOperator | null>(null);
const dismissedOperatorKey = ref<string | null>(null);
const showCommandPopup = ref(false);
const commandHighlightIndex = ref(0);
const commandPopupRef = ref<HTMLElement | null>(null);
const commandItemRefs = ref<HTMLElement[]>([]);
const showMentionPopup = ref(false);
const mentionHighlightIndex = ref(0);
const mentionMode = ref<"search" | "browse">("search");
const mentionEntries = ref<DirEntry[]>([]);
const mentionEntriesPath = ref<string | null>(null);
const mentionSearchResults = ref<MentionSearchResult[]>([]);
const mentionAnchor = ref(-1);
const mentionTokenEnd = ref(-1);
const mentionSubPath = ref("");
const mentionLoading = ref(false);
const assetRefDrafts = new Map<string, AssetRefAttachment[]>();

let mentionDebounceTimer: ReturnType<typeof setTimeout> | null = null;
let mentionRequestSeq = 0;
let lastSearchQuery = "";
let pendingMentionCursor: number | null = null;
let releaseUnityAssetDrop: (() => void) | null = null;
let unityAssetDropSubscriptionDisposed = false;
let assetRefSyncChannel: BroadcastChannel | null = null;
let assetRefSyncSeq = 0;
let lastAssetRefSyncKey = "";
const assetRefSyncSourceId = `rich-chat-input-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

const hasTopAttachments = computed(() =>
  imageAttachments.value.length > 0 || assetRefAttachments.value.length > 0,
);

const canSend = computed(() =>
  props.isStreaming
  || !!props.modelValue.trim()
  || !!pastedContent.value
  || imageAttachments.value.length > 0
  || assetRefAttachments.value.length > 0,
);
const hasHeaderStart = computed(() =>
  !!slots["header-start"]
  || (!!props.showTopPlanBadge && !!composerPlanBadge.value)
  || (!!props.showSkillBadges && composerSkillBadges.value.length > 0),
);
const hasHeaderEnd = computed(() => !!slots["header-end"]);
const hasHeaderContent = computed(() => hasHeaderStart.value || hasHeaderEnd.value);
const hasFooterStart = computed(() => !!slots["footer-start"] || !!slots["top-start"]);
const hasFooterEnd = computed(() =>
  !!slots["footer-end"] || !!slots["top-end"] || !!slots.footer,
);

const commandToken = computed(() =>
  activeOperator.value?.kind === "slash" ? activeOperator.value.token : "",
);

const allowActionCommands = computed(() =>
  !!activeOperator.value
  && activeOperator.value.kind === "slash"
  && props.modelValue.trim() === activeOperator.value.token.trim(),
);

const filteredCommands = computed(() =>
  commandToken.value
    ? getFilteredCommands(commandToken.value, { includeActions: allowActionCommands.value })
    : [],
);

const composerBadges = computed(() => buildIntentBadges(composerIntent.value));
const composerPlanBadge = computed(() =>
  composerBadges.value.find((badge) => badge.kind === "plan") ?? null,
);
const composerSkillBadges = computed(() =>
  composerBadges.value.filter((badge) => badge.kind === "skill"),
);
const previewImage = computed(() => {
  const index = previewImageIndex.value;
  return index == null ? null : imageAttachments.value[index] ?? null;
});
const previewImageSrc = computed(() =>
  previewImage.value ? imagePreviewUrl(previewImage.value) : "",
);

const mentionBreadcrumbs = computed(() => {
  if (!mentionSubPath.value) return [];
  return mentionSubPath.value.split("/").filter(Boolean);
});

const mentionQuery = computed(() =>
  activeOperator.value?.kind === "mention" ? activeOperator.value.query : "",
);

const mentionBrowseFilter = computed(() => {
  const query = mentionQuery.value;
  if (!query) return "";
  const lastSlash = query.lastIndexOf("/");
  return lastSlash >= 0 ? query.slice(lastSlash + 1) : query;
});

function parentPathFor(relPath: string): string {
  const normalized = relPath.replace(/\/+$/, "");
  const slashIndex = normalized.lastIndexOf("/");
  return slashIndex >= 0 ? normalized.slice(0, slashIndex) : "";
}

function mapAssetSearchResult(result: {
  path: string;
  name: string;
  matchScore: number;
}): MentionSearchResult {
  return {
    relPath: result.path,
    name: result.name,
    parentPath: parentPathFor(result.path),
    isDir: false,
    matchScore: result.matchScore,
    entryKind: "asset",
  };
}

function fallbackKnowledgeName(path: string): string {
  const fileName = path.split("/").pop() || path;
  const dotIndex = fileName.lastIndexOf(".");
  return dotIndex > 0 ? fileName.slice(0, dotIndex) : fileName;
}

function mapKnowledgeSearchResult(result: KnowledgeSearchResult): MentionSearchResult {
  return {
    relPath: result.path,
    name: result.title?.trim() || fallbackKnowledgeName(result.path),
    parentPath: parentPathFor(result.path),
    isDir: false,
    matchScore: Math.max(1, Math.round(result.score || 1)),
    meta: result.path,
    entryKind: "knowledge",
  };
}

const rankedMentionSearchResults = computed(() =>
  rankSearchResults(mentionSearchResults.value, mentionQuery.value, (result) => [
    {
      text: result.name,
      weight: result.entryKind === "knowledge"
        ? 165 + Math.min(Math.floor(result.matchScore / 12), 60)
        : 180 + Math.min(Math.floor(result.matchScore / 12), 90),
    },
    {
      text: result.relPath,
      weight: result.entryKind === "knowledge"
        ? 120 + Math.min(Math.floor(result.matchScore / 24), 35)
        : 90 + Math.min(Math.floor(result.matchScore / 24), 45),
    },
    { text: result.parentPath, weight: 30 },
    { text: result.meta || "", weight: 50 },
  ]),
);

const mentionCurrentFolderEntry = computed<MentionDisplayEntry | null>(() => {
  if (mentionMode.value !== "browse" || !mentionSubPath.value) return null;
  const parts = mentionSubPath.value.split("/").filter(Boolean);
  const currentName = parts[parts.length - 1] ?? mentionSubPath.value;
  return {
    relPath: `${mentionSubPath.value}/`,
    name: `${currentName}/`,
    parentPath: parentPathFor(mentionSubPath.value),
    isDir: true,
    meta: t("chat.mention.currentFolder"),
    canNavigate: false,
    isCurrentPath: true,
    entryKind: "asset",
  };
});

const filteredMentionEntries = computed(() => {
  if (mentionMode.value !== "browse") return [];
  if (mentionEntriesPath.value !== mentionSubPath.value) return [];
  const query = mentionBrowseFilter.value;
  if (!query) return mentionEntries.value;
  return rankSearchResults(mentionEntries.value, query, (entry) => [
    { text: entry.name, weight: 170 },
    { text: entry.relPath, weight: 80 },
  ]);
});

const mentionDisplayList = computed<MentionDisplayEntry[]>(() => {
  if (mentionMode.value === "search") {
    return rankedMentionSearchResults.value.map((result) => ({
      relPath: result.relPath,
      name: result.name,
      parentPath: result.parentPath,
      isDir: result.isDir,
      meta: result.meta,
      canNavigate: result.isDir,
      entryKind: result.entryKind,
    }));
  }
  const entries = filteredMentionEntries.value.map((entry) => ({
    relPath: entry.relPath,
    name: entry.name,
    parentPath: "",
    isDir: entry.isDir,
    canNavigate: entry.isDir,
    entryKind: "asset" as const,
  }));
  return mentionCurrentFolderEntry.value
    ? [mentionCurrentFolderEntry.value, ...entries]
    : entries;
});

function buildIntentBadges(
  intent: Pick<ComposerIntentState, "mode" | "skills"> | null | undefined,
) {
  const badges: Array<{ key: string; label: string; kind: "plan" | "skill"; skill?: SkillIntentItem }> = [];
  if (!intent) return badges;

  if (intent.mode === "plan") {
    badges.push({ key: "plan", label: "Plan", kind: "plan" });
  }

  for (const skill of intent.skills || []) {
    badges.push({
      key: `${skill.source}:${skill.dirName}`,
      label: `SKILL: ${skill.name}`,
      kind: "skill",
      skill,
    });
  }

  return badges;
}

function commandTypeLabel(command: CommandDef): string {
  if (command.commandKind === "action") return "ACTION";
  return command.commandType === "plan" ? "MODE" : "SKILL";
}

function showIntentBlockedNotice(command: CommandDef) {
  if (command.commandType === "plan") {
    notificationStore.addNotice("error", t("chat.operator.planOnlyDev"), { operation: "chatIntent" });
    return;
  }

  notificationStore.addNotice("error", command.description, { operation: "chatIntent" });
}

function setInputValue(value: string) {
  emit("update:modelValue", value);
}

function autoResizeTextarea() {
  composerRef.value?.resizeTextarea();
}

function getComposerTextarea() {
  return composerRef.value?.getTextarea() ?? null;
}

function focusComposerSelection(start: number, end = start) {
  composerRef.value?.focus();
  composerRef.value?.setSelectionRange(start, end);
}

function resolveTemplateElement(
  element: Element | ComponentPublicInstance | null,
): Element | null {
  if (element instanceof Element) return element;
  if (element && "$el" in element && element.$el instanceof Element) {
    return element.$el;
  }
  return null;
}

function setCommandItemRef(index: number, element: Element | ComponentPublicInstance | null) {
  const resolved = resolveTemplateElement(element);
  if (!(resolved instanceof HTMLElement)) return;
  commandItemRefs.value[index] = resolved;
}

function clearMentionDebounce() {
  if (!mentionDebounceTimer) return;
  clearTimeout(mentionDebounceTimer);
  mentionDebounceTimer = null;
}

function invalidateMentionRequests() {
  mentionRequestSeq += 1;
  mentionLoading.value = false;
}

function scheduleMentionFetch(task: () => void, delay: number) {
  clearMentionDebounce();
  mentionDebounceTimer = setTimeout(() => {
    mentionDebounceTimer = null;
    task();
  }, delay);
}

function syncMentionCursor(position: number) {
  pendingMentionCursor = position;
  nextTick(() => {
    if (pendingMentionCursor !== position) return;
    focusComposerSelection(position);
    pendingMentionCursor = null;
  });
}

function replaceMentionToken(nextQuery: string) {
  const before = props.modelValue.slice(0, mentionAnchor.value + 1);
  const after = props.modelValue.slice(mentionTokenEnd.value);
  const nextText = before + nextQuery + after;
  const cursor = mentionAnchor.value + 1 + nextQuery.length;
  mentionTokenEnd.value = cursor;
  setInputValue(nextText);
  syncMentionCursor(cursor);
}

async function loadDirEntries(subPath: string) {
  const requestSeq = ++mentionRequestSeq;
  mentionLoading.value = true;
  mentionEntries.value = [];
  mentionEntriesPath.value = subPath;
  try {
    let offset = 0;
    let hasMore = true;
    const allEntries: DirEntry[] = [];

    while (hasMore) {
      const page = await listDirEntriesPage(subPath, offset, 200, false);
      if (
        requestSeq !== mentionRequestSeq
        || !showMentionPopup.value
        || mentionMode.value !== "browse"
        || mentionSubPath.value !== subPath
      ) {
        return;
      }

      allEntries.push(...page.entries);
      mentionEntries.value = [...allEntries];
      mentionEntriesPath.value = subPath;
      offset = page.nextOffset;
      hasMore = page.hasMore;
    }
  } catch {
    if (
      requestSeq !== mentionRequestSeq
      || !showMentionPopup.value
      || mentionMode.value !== "browse"
      || mentionSubPath.value !== subPath
    ) {
      return;
    }
    mentionEntries.value = [];
    mentionEntriesPath.value = subPath;
  } finally {
    if (requestSeq === mentionRequestSeq) {
      mentionLoading.value = false;
    }
  }
}

async function searchAssets(query: string) {
  if (query === lastSearchQuery) return;
  lastSearchQuery = query;
  const requestSeq = ++mentionRequestSeq;
  mentionLoading.value = true;
  try {
    const [assetSearch, knowledgeSearch] = await Promise.allSettled([
      searchWorkspaceAssets(query, [
        "Assets",
        "Packages",
        "ProjectSettings",
      ]),
      knowledgeQuery({
        query,
        limit: 16,
        types: KNOWLEDGE_MENTION_TYPES,
      }),
    ]);
    if (
      requestSeq !== mentionRequestSeq
      || !showMentionPopup.value
      || mentionMode.value !== "search"
      || mentionQuery.value !== query
    ) {
      return;
    }

    const assetResults = assetSearch.status === "fulfilled"
      ? assetSearch.value.map(mapAssetSearchResult)
      : [];
    const knowledgeResults = knowledgeSearch.status === "fulfilled"
      ? knowledgeSearch.value
        .filter((result) => (result.storageSource ?? "project") === "project")
        .map(mapKnowledgeSearchResult)
      : [];

    mentionSearchResults.value = [...assetResults, ...knowledgeResults];

    if (assetSearch.status === "rejected" && mentionSearchResults.value.length === 0) {
      mentionMode.value = "browse";
      mentionSubPath.value = "";
      await loadDirEntries("");
    }
  } catch {
    if (
      requestSeq !== mentionRequestSeq
      || !showMentionPopup.value
      || mentionMode.value !== "search"
      || mentionQuery.value !== query
    ) {
      return;
    }
    mentionSearchResults.value = [];
    mentionMode.value = "browse";
    mentionSubPath.value = "";
    await loadDirEntries("");
  } finally {
    if (requestSeq === mentionRequestSeq) {
      mentionLoading.value = false;
    }
  }
}

function closeMentionPopup() {
  clearMentionDebounce();
  invalidateMentionRequests();
  showMentionPopup.value = false;
  mentionAnchor.value = -1;
  mentionTokenEnd.value = -1;
  mentionSubPath.value = "";
  mentionEntries.value = [];
  mentionEntriesPath.value = null;
  mentionSearchResults.value = [];
  mentionHighlightIndex.value = 0;
  mentionMode.value = "search";
  lastSearchQuery = "";
  pendingMentionCursor = null;
}

function checkMentionTrigger(operator: ActiveOperator, preserveSelection = false) {
  mentionAnchor.value = operator.start;
  mentionTokenEnd.value = operator.end;

  clearMentionDebounce();

  const lastSlash = operator.query.lastIndexOf("/");
  const browseSubPath = lastSlash >= 0
    ? operator.query.slice(0, lastSlash)
    : operator.query.length === 0
      ? ""
      : null;

  if (browseSubPath !== null) {
    const modeChanged = mentionMode.value !== "browse";
    const subPathChanged = mentionSubPath.value !== browseSubPath;
    mentionMode.value = "browse";
    mentionSubPath.value = browseSubPath;
    lastSearchQuery = "";
    if (!preserveSelection) {
      mentionHighlightIndex.value = 0;
    } else if (mentionDisplayList.value.length > 0) {
      mentionHighlightIndex.value = Math.min(mentionHighlightIndex.value, mentionDisplayList.value.length - 1);
    }
    if (modeChanged || subPathChanged || (mentionEntriesPath.value !== browseSubPath && !mentionLoading.value)) {
      scheduleMentionFetch(() => { void loadDirEntries(browseSubPath); }, 120);
    }
  } else {
    mentionMode.value = "search";
    mentionSubPath.value = "";
    if (!preserveSelection) {
      mentionHighlightIndex.value = 0;
    } else if (mentionDisplayList.value.length > 0) {
      mentionHighlightIndex.value = Math.min(mentionHighlightIndex.value, mentionDisplayList.value.length - 1);
    }
    scheduleMentionFetch(() => { void searchAssets(operator.query); }, 150);
  }

  showMentionPopup.value = true;
}

function getOperatorKey(operator: ActiveOperator | null | undefined): string | null {
  if (!operator) return null;
  return `${operator.kind}:${operator.start}:${operator.end}:${operator.token}`;
}

function dismissActiveOperatorPopup() {
  dismissedOperatorKey.value = getOperatorKey(activeOperator.value);
  showCommandPopup.value = false;
  closeMentionPopup();
}

function dismissOperatorPopupForCursor(text: string, cursor: number) {
  dismissedOperatorKey.value = getOperatorKey(detectActiveOperator(text, cursor));
}

function syncOperatorState() {
  const previousOperator = activeOperator.value;
  if (props.isStreaming) {
    activeOperator.value = null;
    showCommandPopup.value = false;
    closeMentionPopup();
    return;
  }

  const textarea = getComposerTextarea();
  if (!textarea) return;

  const cursor = pendingMentionCursor ?? textarea.selectionStart ?? props.modelValue.length;
  const operator = detectActiveOperator(props.modelValue, cursor);
  activeOperator.value = operator;
  const operatorKey = getOperatorKey(operator);

  if (!operator) {
    dismissedOperatorKey.value = null;
    showCommandPopup.value = false;
    closeMentionPopup();
    return;
  }

  if (dismissedOperatorKey.value && dismissedOperatorKey.value !== operatorKey) {
    dismissedOperatorKey.value = null;
  }

  if (dismissedOperatorKey.value && dismissedOperatorKey.value === operatorKey) {
    showCommandPopup.value = false;
    closeMentionPopup();
    return;
  }

  if (operator.kind === "slash") {
    const matches = getFilteredCommands(operator.token, { includeActions: allowActionCommands.value });
    showCommandPopup.value = matches.length > 0;
    const sameSlashToken =
      previousOperator?.kind === "slash"
      && previousOperator.start === operator.start
      && previousOperator.end === operator.end
      && previousOperator.token === operator.token;
    if (!sameSlashToken) {
      commandHighlightIndex.value = 0;
    } else if (matches.length > 0) {
      commandHighlightIndex.value = Math.min(commandHighlightIndex.value, matches.length - 1);
    }
    closeMentionPopup();
    return;
  }

  showCommandPopup.value = false;
  checkMentionTrigger(
    operator,
    previousOperator?.kind === "mention"
    && previousOperator.start === operator.start
    && previousOperator.end === operator.end
    && previousOperator.token === operator.token,
  );
}

function handleTextareaInteraction() {
  nextTick(syncOperatorState);
}

function applyIntentCommand(command: CommandDef) {
  if (command.commandType === "plan") {
    composerIntent.value = {
      ...composerIntent.value,
      mode: "plan",
    };
    return;
  }

  if (command.commandType === "skill" && command.skill) {
    composerIntent.value = mergeComposerIntent(composerIntent.value, {
      skills: [command.skill],
    });
  }
}

function executeCommandFromPopup(command: CommandDef) {
  const operator = activeOperator.value;
  if (!operator || operator.kind !== "slash") return;

  showCommandPopup.value = false;

  if (command.commandKind === "action") {
    setInputValue(command.name);
    nextTick(() => {
      const end = command.name.length;
      focusComposerSelection(end);
      syncOperatorState();
    });
    return;
  }

  setInputValue(removeTextRange(props.modelValue, operator.start, operator.end));
  applyIntentCommand(command);
  nextTick(() => {
    const cursor = Math.min(operator.start, props.modelValue.length);
    focusComposerSelection(cursor);
    syncOperatorState();
  });
}

function browseMentionDirectory(entry: MentionDisplayEntry) {
  const nextPath = entry.relPath.replace(/\/+$/, "");
  mentionMode.value = "browse";
  mentionSubPath.value = nextPath;
  mentionHighlightIndex.value = 0;
  lastSearchQuery = "";
  replaceMentionToken(nextPath ? `${nextPath}/` : "");
}

function selectMentionEntry(entry: MentionDisplayEntry) {
  const mentionPath = entry.isDir && !entry.relPath.endsWith("/")
    ? `${entry.relPath}/`
    : entry.relPath;

  const assetRef = buildManualAssetRef(mentionPath);
  if (assetRef) {
    const nextText = removeTextRange(props.modelValue, mentionAnchor.value, mentionTokenEnd.value);
    const cursor = Math.max(0, Math.min(mentionAnchor.value, nextText.length));
    dismissOperatorPopupForCursor(nextText, cursor);
    setInputValue(nextText);
    addAssetRefs([assetRef]);
    closeMentionPopup();
    nextTick(() => {
      focusComposerSelection(cursor);
      syncOperatorState();
    });
    return;
  }

  const nextMention = insertInlineMention(
    props.modelValue,
    mentionAnchor.value,
    mentionTokenEnd.value,
    mentionPath,
  );
  dismissOperatorPopupForCursor(nextMention.text, nextMention.cursor);
  setInputValue(nextMention.text);
  closeMentionPopup();
  nextTick(() => {
    focusComposerSelection(nextMention.cursor);
    syncOperatorState();
  });
}

function mentionNavigateTo(level: number) {
  const parts = mentionBreadcrumbs.value.slice(0, level + 1);
  const newSubPath = parts.join("/");
  const path = newSubPath ? `${newSubPath}/` : "";
  mentionSubPath.value = newSubPath;
  mentionHighlightIndex.value = 0;
  lastSearchQuery = "";
  replaceMentionToken(path);
}

function mentionNavigateRoot() {
  mentionSubPath.value = "";
  mentionHighlightIndex.value = 0;
  lastSearchQuery = "";
  replaceMentionToken("");
}

function mentionNavigateParent() {
  if (mentionBreadcrumbs.value.length <= 1) {
    mentionNavigateRoot();
    return;
  }
  mentionNavigateTo(mentionBreadcrumbs.value.length - 2);
}

function inferAssetRefKind(path: string, kind?: AssetRefAttachment["kind"]): AssetRefAttachment["kind"] {
  if (kind === "knowledge") return "knowledge";
  if (kind === "sceneObject") return "sceneObject";
  return /^((?:Assets|Packages)\/.+?\.unity)\/.+/i.test(path) ? "sceneObject" : "asset";
}

function normalizeUnityAssetRefPath(path: string) {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function isSupportedUnityAssetRefPath(path: string) {
  return UNITY_ASSET_REF_ROOT_RE.test(normalizeUnityAssetRefPath(path));
}

function isSupportedProjectKnowledgeRefPath(path: string) {
  return PROJECT_KNOWLEDGE_REF_ROOT_RE.test(normalizeUnityAssetRefPath(path));
}

function buildManualAssetRef(path: string): AssetRefAttachment | null {
  const normalizedPath = normalizeUnityAssetRefPath(path);
  if (!normalizedPath) return null;
  if (isSupportedProjectKnowledgeRefPath(normalizedPath)) {
    return {
      path: normalizedPath,
      kind: "knowledge",
      source: "manual",
    };
  }
  if (!isSupportedUnityAssetRefPath(normalizedPath)) return null;
  return {
    path: normalizedPath,
    kind: inferAssetRefKind(normalizedPath),
    source: "manual",
  };
}

function buildManualAssetRefs(paths: string[]) {
  return paths
    .map((path) => buildManualAssetRef(path))
    .filter((assetRef): assetRef is AssetRefAttachment => !!assetRef);
}

function extractInlineUnityAssetRefs(text: string) {
  const extracted = extractChatAssetRefs(text);
  return {
    text: extracted.text,
    assetRefs: buildManualAssetRefs(extracted.refs),
  };
}

function normalizeAssetRef(assetRef: AssetRefAttachment): AssetRefAttachment | null {
  const path = assetRef.path.trim().replace(/\\/g, "/").replace(/\/+$/, "");
  if (!path) return null;
  return {
    path,
    kind: inferAssetRefKind(path, assetRef.kind),
    name: assetRef.name?.trim() || undefined,
    typeLabel: assetRef.typeLabel?.trim() || undefined,
    source: assetRef.source ?? "unity",
  };
}

function assetRefKey(assetRef: Pick<AssetRefAttachment, "kind" | "path">) {
  return `${assetRef.kind}\u{0}${assetRef.path.toLowerCase()}`;
}

function dedupeAssetRefs(assetRefs: AssetRefAttachment[]) {
  const seen = new Set<string>();
  const next: AssetRefAttachment[] = [];
  for (const assetRef of assetRefs) {
    const normalized = normalizeAssetRef(assetRef);
    if (!normalized) continue;
    const key = assetRefKey(normalized);
    if (seen.has(key)) continue;
    seen.add(key);
    next.push(normalized);
  }
  return next;
}

function cloneAssetRefs(assetRefs: AssetRefAttachment[]) {
  return assetRefs.map((assetRef) => ({ ...assetRef }));
}

function currentAssetRefSyncKey() {
  return props.assetRefSyncKey.trim();
}

function rememberAssetRefDraft(assetRefs: AssetRefAttachment[], key = currentAssetRefSyncKey()) {
  if (!key) return;
  if (assetRefs.length > 0) {
    assetRefDrafts.set(key, cloneAssetRefs(assetRefs));
    return;
  }
  assetRefDrafts.delete(key);
}

function broadcastAssetRefDraft(assetRefs: AssetRefAttachment[], key = currentAssetRefSyncKey()) {
  if (!key) return;
  const message: AssetRefSyncMessage = {
    kind: "assetRefs",
    sourceId: assetRefSyncSourceId,
    syncKey: key,
    refs: cloneAssetRefs(assetRefs),
    seq: ++assetRefSyncSeq,
  };

  assetRefSyncChannel?.postMessage(message);

  try {
    window.localStorage.setItem(ASSET_REF_SYNC_STORAGE_KEY, JSON.stringify(message));
    window.localStorage.removeItem(ASSET_REF_SYNC_STORAGE_KEY);
  } catch {
    // Local storage can be disabled; BroadcastChannel is the primary path.
  }
}

function setAssetRefAttachments(
  assetRefs: AssetRefAttachment[],
  options: { broadcast?: boolean } = {},
) {
  const next = dedupeAssetRefs(assetRefs);
  assetRefAttachments.value = next;
  rememberAssetRefDraft(next);
  if (options.broadcast !== false) {
    broadcastAssetRefDraft(next);
  }
  return next;
}

function applyAssetRefSyncMessage(message: unknown) {
  if (!message || typeof message !== "object") return;
  const payload = message as Partial<AssetRefSyncMessage>;
  if (payload.kind !== "assetRefs") return;
  if (!payload.syncKey || payload.sourceId === assetRefSyncSourceId) return;

  const refs = dedupeAssetRefs(Array.isArray(payload.refs) ? payload.refs : []);
  rememberAssetRefDraft(refs, payload.syncKey);
  if (payload.syncKey === currentAssetRefSyncKey()) {
    assetRefAttachments.value = cloneAssetRefs(refs);
  }
}

function handleAssetRefSyncStorage(event: StorageEvent) {
  if (event.key !== ASSET_REF_SYNC_STORAGE_KEY || !event.newValue) return;
  try {
    applyAssetRefSyncMessage(JSON.parse(event.newValue));
  } catch {
    // Ignore malformed cross-window draft sync payloads.
  }
}

function setupAssetRefSync() {
  if (typeof BroadcastChannel !== "undefined") {
    assetRefSyncChannel = new BroadcastChannel(ASSET_REF_SYNC_CHANNEL);
    assetRefSyncChannel.onmessage = (event) => {
      applyAssetRefSyncMessage(event.data);
    };
  }
  window.addEventListener("storage", handleAssetRefSyncStorage);
}

function teardownAssetRefSync() {
  window.removeEventListener("storage", handleAssetRefSyncStorage);
  assetRefSyncChannel?.close();
  assetRefSyncChannel = null;
}

function addAssetRefs(assetRefs: AssetRefAttachment[]) {
  const next = setAssetRefAttachments([...assetRefAttachments.value, ...assetRefs]);
  if (next.length > 0) {
    nextTick(() => composerRef.value?.focus());
  }
}

function removeAssetRef(index: number) {
  const next = [...assetRefAttachments.value];
  next.splice(index, 1);
  setAssetRefAttachments(next);
}

function buildAssetRefsPromptBlock(assetRefs: AssetRefAttachment[]) {
  if (assetRefs.length === 0) return "";
  const lines = assetRefs.map((assetRef) => {
    if (assetRef.kind === "knowledge") {
      return `- project knowledge: \`${assetRef.path}\` (use \`knowledge_read\`)`;
    }
    const label = assetRef.kind === "sceneObject" ? "scene object" : "asset";
    return `- ${label}: {@${assetRef.path}}`;
  });
  return `<locus-references>\nUse Unity refs as exact asset anchors. Use project knowledge refs as exact knowledge_read paths.\n${lines.join("\n")}\n</locus-references>`;
}

function appendAssetRefsPromptBlock(text: string, assetRefs: AssetRefAttachment[]) {
  const block = buildAssetRefsPromptBlock(assetRefs);
  if (!block) return text;
  return text.trim() ? `${text}\n\n${block}` : block;
}

function resetDraft() {
  setInputValue("");
  pastedContent.value = "";
  imageAttachments.value = [];
  setAssetRefAttachments([]);
  closeImagePreview();
  composerIntent.value = emptyComposerIntent();
  activeOperator.value = null;
  dismissedOperatorKey.value = null;
  showCommandPopup.value = false;
  closeMentionPopup();
  nextTick(() => {
    autoResizeTextarea();
  });
}

async function applyPrefill(text: string) {
  resetDraft();
  setInputValue(text);
  await nextTick();
  autoResizeTextarea();
  const end = text.length;
  focusComposerSelection(end);
  syncOperatorState();
}

function showUserIntentMissingInputNotice() {
  notificationStore.addNotice("error", t("chat.operator.intentNeedsInput"), { operation: "chatIntent" });
}

function buildSendPayload(
  text: string,
  images: ImageAttachment[],
  assetRefs: AssetRefAttachment[],
  intent: ComposerIntentState,
  displayText?: string,
): ChatComposerSendPayload {
  return {
    text,
    displayText: displayText ?? text,
    images,
    assetRefs,
    mode: intent.mode === "plan" ? "plan" : null,
    userIntent: buildUserIntentMeta(intent),
  };
}

function canExecuteActionCommand(): boolean {
  return !pastedContent.value
    && imageAttachments.value.length === 0
    && assetRefAttachments.value.length === 0
    && !hasComposerIntent(composerIntent.value);
}

function executeActionCommand(command: CommandDef): boolean {
  if (command.commandKind !== "action" || !canExecuteActionCommand()) return false;

  if (command.commandType === "clear") {
    resetDraft();
    emit("clear");
    return true;
  }

  if (command.commandType === "compact") {
    resetDraft();
    emit("compact");
    return true;
  }

  return false;
}

function tryHandleExactActionCommand(): boolean {
  const typed = props.modelValue.trim();
  if (!typed || !canExecuteActionCommand()) {
    return false;
  }

  const command = findExactAvailableCommand(typed);
  return command ? executeActionCommand(command) : false;
}

function handleSend() {
  if (props.isStreaming) return;

  if (tryHandleExactActionCommand()) {
    return;
  }

  const parsed = parseInlineIntentCommands(props.modelValue, allCommands.value, props.selectedAgentId);
  if (parsed.blockedCommand) {
    showIntentBlockedNotice(parsed.blockedCommand);
    return;
  }

  const mergedIntent = mergeComposerIntent(composerIntent.value, parsed.intent);
  const inlineAssetRefs = extractInlineUnityAssetRefs(parsed.cleanedText);
  const cleanedInput = normalizeComposerText(inlineAssetRefs.text);
  const images: ImageAttachment[] = props.allowImages
    ? imageAttachments.value.map(({ data, mimeType }) => ({ data, mimeType }))
    : [];
  const assetRefs = dedupeAssetRefs([...assetRefAttachments.value, ...inlineAssetRefs.assetRefs]);

  if (!cleanedInput && !pastedContent.value && images.length === 0 && assetRefs.length === 0) {
    if (hasComposerIntent(mergedIntent)) {
      showUserIntentMissingInputNotice();
    }
    return;
  }

  const text = pastedContent.value
    ? (cleanedInput ? `${cleanedInput}\n\n${pastedContent.value}` : pastedContent.value)
    : cleanedInput;

  const sendText = appendAssetRefsPromptBlock(text, assetRefs);
  const payload = buildSendPayload(sendText, images, assetRefs, mergedIntent, text);
  resetDraft();
  emit("send", payload);
}

function handleKeydown(event: KeyboardEvent) {
  if (showMentionPopup.value) {
    const items = mentionDisplayList.value;
    if (event.key === "ArrowDown") {
      if (items.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      mentionHighlightIndex.value = (mentionHighlightIndex.value + 1) % items.length;
      return;
    }
    if (event.key === "ArrowUp") {
      if (items.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      mentionHighlightIndex.value = (mentionHighlightIndex.value - 1 + items.length) % items.length;
      return;
    }
    if (shouldSelectPopupOnEnter(event, chatInputSettings.submitMode)) {
      if (items.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      const current = items[mentionHighlightIndex.value];
      if (mentionMode.value === "browse" && current.isDir && current.canNavigate) {
        browseMentionDirectory(current);
        return;
      }
      selectMentionEntry(current);
      return;
    }
    if (event.key === "Tab" && !event.shiftKey) {
      if (items.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      selectMentionEntry(items[mentionHighlightIndex.value]);
      return;
    }
    if (event.key === "ArrowRight") {
      if (items.length === 0) return;
      const current = items[mentionHighlightIndex.value];
      if (!current.isDir || !current.canNavigate) return;
      event.preventDefault();
      event.stopPropagation();
      browseMentionDirectory(current);
      return;
    }
    if (event.key === "ArrowLeft" && mentionMode.value === "browse" && mentionSubPath.value) {
      event.preventDefault();
      event.stopPropagation();
      mentionNavigateParent();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      dismissActiveOperatorPopup();
      return;
    }
  }

  if (showCommandPopup.value) {
    const commands = filteredCommands.value;
    if (event.key === "ArrowDown") {
      if (commands.length === 0) return;
      event.preventDefault();
      commandHighlightIndex.value = (commandHighlightIndex.value + 1) % commands.length;
      return;
    }
    if (event.key === "ArrowUp") {
      if (commands.length === 0) return;
      event.preventDefault();
      commandHighlightIndex.value = (commandHighlightIndex.value - 1 + commands.length) % commands.length;
      return;
    }
    if (shouldSubmitOnEnter(event, chatInputSettings.submitMode)) {
      const command = commands[commandHighlightIndex.value];
      if (command && executeActionCommand(command)) {
        event.preventDefault();
        return;
      }
      if (commands.length === 0) return;
      event.preventDefault();
      executeCommandFromPopup(commands[commandHighlightIndex.value]);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      dismissActiveOperatorPopup();
      return;
    }
    if (event.key === "Tab" && commands.length > 0) {
      event.preventDefault();
      executeCommandFromPopup(commands[commandHighlightIndex.value]);
      return;
    }
  }

  if (shouldSubmitOnEnter(event, chatInputSettings.submitMode)) {
    event.preventDefault();
    handleSend();
  }
}

function handleTextareaKeyup(event: KeyboardEvent) {
  if (event.key === "Escape") return;
  if (showMentionPopup.value || showCommandPopup.value) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Enter" || event.key === "Tab" || event.key === "Escape") {
      return;
    }
  }
  handleTextareaInteraction();
}

function handlePaste(event: ClipboardEvent) {
  const items = event.clipboardData?.items;
  if (props.allowImages && items) {
    for (let index = 0; index < items.length; index += 1) {
      const item = items[index];
      const file = item.kind === "file" ? item.getAsFile() : null;
      const mimeType = item.type || file?.type || "";
      if (!file || !mimeType.startsWith("image/")) continue;
      event.preventDefault();
      addImageFile(file);
      return;
    }
  }

  const text = event.clipboardData?.getData("text/plain") || "";
  if (text.length > PASTE_THRESHOLD) {
    event.preventDefault();
    pastedContent.value = text;
  }
}

function addImageFile(file: File) {
  if (imageAttachments.value.length >= props.maxImages) return;
  const reader = new FileReader();
  reader.onload = () => {
    const dataUrl = reader.result as string;
    const commaIndex = dataUrl.indexOf(",");
    if (commaIndex < 0) return;
    imageAttachments.value.push({
      data: dataUrl.substring(commaIndex + 1),
      mimeType: file.type || "image/png",
    });
  };
  reader.readAsDataURL(file);
}

function removeImage(index: number) {
  closeImagePreview();
  imageAttachments.value.splice(index, 1);
}

function imagePreviewUrl(image: ImageAttachment): string {
  return `data:${image.mimeType};base64,${image.data}`;
}

function openImagePreview(index: number) {
  previewImageIndex.value = index;
}

function closeImagePreview() {
  previewImageIndex.value = null;
}

function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && previewImageIndex.value != null) {
    closeImagePreview();
  }
}

function openPasteEditor() {
  showPasteEditor.value = true;
}

function closePasteEditor() {
  showPasteEditor.value = false;
}

function removePastedContent() {
  pastedContent.value = "";
}

function removePlanBadge() {
  composerIntent.value = {
    ...composerIntent.value,
    mode: "build",
  };
}

function removeSkillBadge(skill: SkillIntentItem) {
  composerIntent.value = {
    ...composerIntent.value,
    skills: composerIntent.value.skills.filter(
      (item) => !(item.dirName === skill.dirName && item.source === skill.source),
    ),
  };
}

watch(() => props.modelValue, () => {
  nextTick(syncOperatorState);
});

watch(
  () => [showCommandPopup.value, commandHighlightIndex.value, filteredCommands.value.length],
  async ([visible]) => {
    if (!visible) return;
    await nextTick();
    const popup = commandPopupRef.value;
    const selected = commandItemRefs.value[commandHighlightIndex.value];
    if (!popup || !selected) return;

    const itemTop = selected.offsetTop;
    const itemBottom = itemTop + selected.offsetHeight;
    const viewTop = popup.scrollTop;
    const viewBottom = viewTop + popup.clientHeight;

    if (itemTop < viewTop) {
      popup.scrollTop = itemTop;
      return;
    }

    if (itemBottom > viewBottom) {
      popup.scrollTop = itemBottom - popup.clientHeight;
    }
  },
);

watch(
  () => filteredCommands.value,
  () => {
    commandItemRefs.value = [];
  },
);

watch(
  () => props.assetRefSyncKey,
  (nextKey, previousKey) => {
    const previous = (previousKey ?? lastAssetRefSyncKey).trim();
    if (previous) {
      rememberAssetRefDraft(assetRefAttachments.value, previous);
    }

    const next = nextKey.trim();
    lastAssetRefSyncKey = next;
    if (!next) return;
    assetRefAttachments.value = cloneAssetRefs(assetRefDrafts.get(next) ?? []);
  },
  { immediate: true },
);

onMounted(() => {
  unityAssetDropSubscriptionDisposed = false;
  setupAssetRefSync();
  document.addEventListener("keydown", handleDocumentKeydown);
  subscribeUnityEmbedAssetDrop((payload) => {
    addAssetRefs(payload.refs ?? []);
  })
    .then((release) => {
      if (unityAssetDropSubscriptionDisposed) {
        release();
        return;
      }
      releaseUnityAssetDrop = release;
    })
    .catch((error) => {
      console.warn("[Locus] Unity asset drop subscription failed:", error);
    });
});

onUnmounted(() => {
  unityAssetDropSubscriptionDisposed = true;
  document.removeEventListener("keydown", handleDocumentKeydown);
  teardownAssetRefSync();
  releaseUnityAssetDrop?.();
  releaseUnityAssetDrop = null;
  clearMentionDebounce();
  invalidateMentionRequests();
});

defineExpose({
  focus() {
    composerRef.value?.focus();
  },
  setSelectionRange(start: number, end = start) {
    composerRef.value?.setSelectionRange(start, end);
  },
  resizeTextarea() {
    composerRef.value?.resizeTextarea();
  },
  getTextarea() {
    return composerRef.value?.getTextarea() ?? null;
  },
  resetDraft,
  applyPrefill,
});
</script>

<template>
  <ChatInputShell>
    <template #floating>
      <Transition name="cmd-popup">
        <div
          v-if="showCommandPopup && filteredCommands.length > 0"
          ref="commandPopupRef"
          class="command-popup"
        >
          <div
            v-for="(command, index) in filteredCommands"
            :key="command.name"
            class="command-item"
            :class="{ highlighted: index === commandHighlightIndex }"
            :ref="(el) => setCommandItemRef(index, el)"
            @mouseenter="commandHighlightIndex = index"
            @mousedown.prevent="executeCommandFromPopup(command)"
          >
            <div class="command-main">
              <div class="command-header">
                <span class="command-name">{{ command.name }}</span>
                <span v-if="command.argumentHint" class="command-hint-inline">{{ command.argumentHint }}</span>
                <span class="command-kind-badge">{{ commandTypeLabel(command) }}</span>
              </div>
              <span class="command-desc">{{ command.description }}</span>
            </div>
          </div>
        </div>
      </Transition>

      <Transition name="cmd-popup">
        <MentionPopup
          :visible="showMentionPopup"
          :mode="mentionMode"
          :entries="mentionDisplayList"
          :selected-index="mentionHighlightIndex"
          :breadcrumbs="mentionBreadcrumbs"
          :query="mentionMode === 'search' ? mentionQuery : mentionBrowseFilter"
          :loading="mentionLoading"
          @select="selectMentionEntry"
          @open-dir="browseMentionDirectory"
          @navigate-to="mentionNavigateTo"
          @navigate-root="mentionNavigateRoot"
          @update:selected-index="mentionHighlightIndex = $event"
        />
      </Transition>
    </template>

    <template #before-composer>
      <Transition name="paste-preview">
        <div v-if="pastedContent" class="paste-preview">
          <div
            class="paste-preview-body"
            :title="t('chat.paste.clickToEdit')"
            @click="openPasteEditor"
          >
            <div class="paste-preview-text">{{ pastedContent }}</div>
          </div>
          <div class="paste-preview-footer">
            <span class="paste-badge">PASTED</span>
            <span class="paste-char-count">{{ pastedContent.length }} chars</span>
            <button class="paste-remove ui-select-none" @click="removePastedContent">&times;</button>
          </div>
        </div>
      </Transition>
    </template>

    <ChatComposer
      ref="composerRef"
      :model-value="modelValue"
      :placeholder="placeholder"
      :disabled="disabled"
      :is-streaming="isStreaming"
      :can-send="canSend"
      :send-label="sendLabel"
      :cancel-label="cancelLabel"
      :submit-mode="chatInputSettings.submitMode"
      :compact="compact"
      :show-action="showAction"
      :show-header="hasHeaderContent"
      :extend-top="hasTopAttachments"
      @update:model-value="setInputValue"
      @keydown="handleKeydown"
      @paste="handlePaste"
      @click="handleTextareaInteraction"
      @keyup="handleTextareaKeyup"
      @mouseup="handleTextareaInteraction"
      @focus="handleTextareaInteraction"
      @send="handleSend"
      @cancel="emit('cancel')"
    >
      <template #overlay>
        <div v-if="hasTopAttachments" class="composer-attachment-list">
          <AssetChip
            v-for="(assetRef, index) in assetRefAttachments"
            :key="`${assetRef.kind}:${assetRef.path}`"
            :path="assetRef.path"
            :kind="assetRef.kind"
            removable
            @remove="removeAssetRef(index)"
          />
          <div
            v-for="(image, index) in imageAttachments"
            :key="`image:${index}`"
            class="image-attachment-item"
          >
            <button
              class="image-attachment-thumb-button ui-select-none"
              type="button"
              :aria-label="t('chat.paste.previewImage')"
              @click="openImagePreview(index)"
            >
              <img :src="imagePreviewUrl(image)" class="image-attachment-thumb" alt="" />
            </button>
            <button
              class="image-attachment-remove ui-select-none"
              type="button"
              :aria-label="t('chat.paste.remove')"
              @click="removeImage(index)"
            >
              &times;
            </button>
          </div>
        </div>
      </template>
      <template #header>
        <div class="composer-header-row">
          <div v-if="hasHeaderStart" class="composer-header-start">
            <slot name="header-start" />
            <button
              v-if="showTopPlanBadge && composerPlanBadge"
              type="button"
              class="composer-badge composer-top-badge plan ui-select-none"
              @click="removePlanBadge"
            >
              <span>{{ composerPlanBadge.label }}</span>
              <span class="composer-badge-remove">&times;</span>
            </button>
            <div v-if="showSkillBadges && composerSkillBadges.length > 0" class="composer-badge-row">
              <button
                v-for="badge in composerSkillBadges"
                :key="badge.key"
                type="button"
                class="composer-badge skill ui-select-none"
                @click="badge.skill ? removeSkillBadge(badge.skill) : undefined"
              >
                <span>{{ badge.label }}</span>
                <span class="composer-badge-remove">&times;</span>
              </button>
            </div>
          </div>
          <div v-if="hasHeaderEnd" class="composer-header-end">
            <slot name="header-end" />
          </div>
        </div>
      </template>
      <template v-if="hasFooterStart" #footer-start>
        <slot name="footer-start" />
        <slot name="top-start" />
      </template>
      <template v-if="hasFooterEnd" #footer-end>
        <slot name="footer-end" />
        <slot name="top-end" />
        <slot name="footer" />
      </template>
    </ChatComposer>
  </ChatInputShell>

  <Teleport to="body">
    <Transition name="paste-editor-overlay">
      <div v-if="showPasteEditor" class="paste-editor-overlay" @mousedown.self="closePasteEditor">
        <div class="paste-editor-modal">
          <div class="paste-editor-header">
            <span class="paste-editor-title">{{ t("chat.paste.editTitle") }}</span>
            <button class="paste-editor-close ui-select-none" @click="closePasteEditor">&times;</button>
          </div>
          <textarea
            v-model="pastedContent"
            class="paste-editor-textarea"
            spellcheck="false"
          />
          <div class="paste-editor-footer">
            <span class="paste-editor-info">{{ pastedContent.length }} chars</span>
            <div class="paste-editor-actions">
              <button class="paste-editor-btn paste-editor-btn-danger ui-select-none" @click="removePastedContent(); closePasteEditor()">
                {{ t("chat.paste.remove") }}
              </button>
              <button class="paste-editor-btn paste-editor-btn-primary ui-select-none" @click="closePasteEditor">
                {{ t("chat.paste.done") }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <Teleport to="body">
    <Transition name="image-preview-overlay">
      <div
        v-if="previewImageSrc"
        class="image-preview-overlay"
        @click.self="closeImagePreview"
      >
        <div class="image-preview-dialog" role="dialog" :aria-label="t('chat.paste.previewImage')">
          <button
            class="image-preview-close ui-select-none"
            type="button"
            :aria-label="t('common.close')"
            @click="closeImagePreview"
          >
            &times;
          </button>
          <img :src="previewImageSrc" class="image-preview-dialog-img" alt="" />
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.command-popup {
  position: absolute;
  bottom: 100%;
  left: 0;
  right: 0;
  margin-bottom: 4px;
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 4px;
  box-shadow: 0 -4px 16px rgba(0, 0, 0, 0.12);
  z-index: 10;
  max-height: 240px;
  overflow-y: auto;
}

.command-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: 7px;
  cursor: pointer;
  transition: background 0.12s, border-color 0.12s, box-shadow 0.12s;
}

.command-item:hover {
  background: var(--hover-bg);
}

.command-item.highlighted {
  background: color-mix(in srgb, var(--accent-soft) 86%, var(--hover-bg) 14%);
  border-color: color-mix(in srgb, var(--accent-color) 28%, var(--border-color));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 10%, transparent);
}

.command-main {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.command-header {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.command-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--accent-color);
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.command-kind-badge {
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  border-radius: var(--radius-badge);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  color: var(--text-secondary);
  background: var(--hover-bg);
}

.command-item.highlighted .command-kind-badge {
  color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-soft) 74%, var(--hover-bg) 26%);
}

.command-desc {
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.command-item.highlighted .command-desc {
  color: color-mix(in srgb, var(--text-color) 72%, var(--text-secondary) 28%);
}

.command-hint-inline {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.72;
  font-family: var(--font-mono-identifier);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.command-item.highlighted .command-hint-inline {
  color: color-mix(in srgb, var(--accent-color) 30%, var(--text-secondary) 70%);
  opacity: 0.92;
}

.cmd-popup-enter-active {
  transition: all 0.15s ease-out;
}

.cmd-popup-leave-active {
  transition: all 0.1s ease-in;
}

.cmd-popup-enter-from,
.cmd-popup-leave-to {
  opacity: 0;
  transform: translateY(6px);
}

:deep(.mention-popup) {
  position: absolute;
  bottom: 100%;
  left: 0;
  right: 0;
  margin-bottom: 4px;
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  padding: 4px;
  box-shadow: 0 -4px 16px rgba(0, 0, 0, 0.12);
  z-index: 10;
  max-height: 320px;
  overflow-y: auto;
}

:deep(.mention-breadcrumb) {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 6px 10px 4px;
  font-size: 12px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  border-bottom: 1px solid var(--border-color);
  margin-bottom: 2px;
  flex-wrap: wrap;
}

:deep(.mention-crumb) {
  cursor: pointer;
  padding: 1px 3px;
  border-radius: 3px;
  transition: background 0.1s, color 0.1s;
}

:deep(.mention-crumb:hover) {
  background: var(--hover-bg);
  color: var(--accent-color);
}

:deep(.mention-crumb.active) {
  color: var(--text-primary);
  font-weight: 600;
}

:deep(.mention-crumb-sep) {
  color: var(--text-secondary);
  opacity: 0.5;
}

:deep(.mention-item) {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px;
  border-radius: 7px;
  transition: background 0.12s ease, box-shadow 0.12s ease;
}

:deep(.mention-item.highlighted) {
  background: color-mix(in srgb, var(--accent-soft) 58%, var(--hover-bg) 42%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

:deep(.mention-item.is-current-path) {
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  margin-bottom: 2px;
  padding-bottom: 6px;
}

:deep(.mention-select) {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

:deep(.mention-copy) {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

:deep(.mention-icon) {
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  opacity: 0.95;
}

:deep(.mention-name) {
  font-size: 13px;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono-identifier);
}

:deep(.mention-path) {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--font-mono-identifier);
}

:deep(.mention-name-fragment),
:deep(.mention-path-fragment) {
  display: inline;
}

:deep(.mention-name-fragment.is-match),
:deep(.mention-path-fragment.is-match) {
  color: color-mix(in srgb, var(--accent-color) 86%, var(--text-color) 14%);
  background: color-mix(in srgb, var(--accent-soft) 62%, transparent);
  border-radius: 4px;
  padding: 0 1px;
  font-weight: 600;
}

:deep(.mention-item.highlighted .mention-name-fragment.is-match),
:deep(.mention-item.highlighted .mention-path-fragment.is-match) {
  background: color-mix(in srgb, var(--accent-soft) 78%, transparent);
}

:deep(.mention-search-header) {
  padding: 6px 10px 4px;
  border-bottom: 1px solid var(--border-color);
  margin-bottom: 2px;
}

:deep(.mention-search-label) {
  font-size: 11px;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  font-weight: 600;
}

:deep(.mention-open) {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.1s, border-color 0.1s, color 0.1s;
}

:deep(.mention-open:hover) {
  background: color-mix(in srgb, var(--hover-bg) 88%, transparent);
  border-color: color-mix(in srgb, var(--border-color) 82%, transparent);
  color: var(--text-color);
}

:deep(.mention-loading),
:deep(.mention-empty) {
  padding: 12px 10px;
  font-size: 12px;
  color: var(--text-secondary);
  text-align: center;
}

:deep(.mention-loading-inline) {
  padding-top: 6px;
  border-top: 1px solid var(--border-color);
  margin-top: 2px;
  font-size: 11px;
}

.composer-attachment-list {
  display: flex;
  align-items: center;
  flex-wrap: nowrap;
  gap: 6px;
  min-width: 0;
  max-width: 100%;
  overflow-x: auto;
  overflow-y: visible;
  padding: 1px 8px 7px 0;
  pointer-events: auto;
  scrollbar-width: thin;
}

.composer-attachment-list::-webkit-scrollbar {
  height: 4px;
}

.composer-attachment-list::-webkit-scrollbar-thumb {
  background: color-mix(in srgb, var(--border-color) 72%, transparent);
  border-radius: 999px;
}

.composer-attachment-list :deep(.asset-chip) {
  flex: 0 0 auto;
  height: 28px;
  min-height: 28px;
  max-width: min(280px, calc(100vw - 96px));
  padding-top: 0;
  padding-bottom: 0;
  background: color-mix(in srgb, var(--panel-bg) 70%, var(--input-bg) 30%);
  border-color: color-mix(in srgb, var(--border-color) 88%, transparent);
  font-size: 12px;
  line-height: 1;
}

.composer-attachment-list :deep(.asset-chip:hover) {
  background: color-mix(in srgb, var(--hover-bg) 82%, var(--panel-bg) 18%);
  border-color: color-mix(in srgb, var(--accent-color) 35%, var(--border-color));
}

.composer-attachment-list :deep(.asset-chip-remove:hover) {
  background: color-mix(in srgb, var(--status-error-bg, var(--hover-bg)) 76%, transparent);
  color: var(--status-error-fg, var(--text-color));
}

.image-attachment-item {
  position: relative;
  flex: 0 0 auto;
  width: 28px;
  height: 28px;
  border: 1px solid var(--border-color);
  border-radius: 7px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--input-bg) 28%);
}

.image-attachment-thumb-button {
  display: block;
  width: 100%;
  height: 100%;
  padding: 0;
  border: none;
  border-radius: 6px;
  overflow: hidden;
  background: color-mix(in srgb, var(--input-bg) 80%, var(--panel-bg) 20%);
  cursor: zoom-in;
}

.image-attachment-thumb-button:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 1px;
}

.image-attachment-thumb {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.image-attachment-remove {
  position: absolute;
  top: -1px;
  right: -1px;
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 50%;
  background: var(--panel-bg);
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease, color 0.12s ease, background 0.12s ease, border-color 0.12s ease;
}

.image-attachment-item:hover .image-attachment-remove,
.image-attachment-remove:focus-visible {
  opacity: 1;
}

.image-attachment-remove:hover,
.image-attachment-remove:focus-visible {
  color: var(--text-color);
  background: var(--hover-bg);
  border-color: color-mix(in srgb, var(--border-color) 82%, transparent);
}

.image-preview-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  background: rgba(0, 0, 0, 0.46);
}

.image-preview-dialog {
  position: relative;
  max-width: min(76vw, 920px);
  max-height: min(78vh, 720px);
  padding: 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--surface-elevated, var(--panel-bg));
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.34);
}

.image-preview-dialog-img {
  display: block;
  max-width: calc(min(76vw, 920px) - 20px);
  max-height: calc(min(78vh, 720px) - 20px);
  border-radius: 5px;
  object-fit: contain;
}

.image-preview-close {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 6px;
  background: var(--surface-elevated, var(--panel-bg));
  color: var(--text-secondary);
  font-size: 16px;
  line-height: 1;
  cursor: pointer;
}

.image-preview-close:hover,
.image-preview-close:focus-visible {
  color: var(--text-color);
  background: var(--hover-bg);
}

.image-preview-overlay-enter-active,
.image-preview-overlay-leave-active {
  transition: opacity 0.12s ease;
}

.image-preview-overlay-enter-from,
.image-preview-overlay-leave-to {
  opacity: 0;
}

.paste-preview {
  background: var(--input-bg);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  overflow: hidden;
}

.paste-preview-body {
  max-height: 120px;
  overflow-y: auto;
  padding: 10px 14px;
  cursor: pointer;
}

.paste-preview-text {
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-word;
  opacity: 0.8;
  -webkit-mask-image: linear-gradient(to bottom, #000 70%, transparent 100%);
  mask-image: linear-gradient(to bottom, #000 70%, transparent 100%);
}

.paste-preview-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 14px 8px;
}

.paste-badge {
  display: inline-block;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.5px;
  padding: 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  color: var(--text-secondary);
  background: var(--bg-color);
}

.paste-char-count {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.7;
  margin-left: auto;
  margin-right: 8px;
}

.paste-remove {
  background: none;
  border: none;
  font-size: 18px;
  line-height: 1;
  color: var(--text-secondary);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  transition: all 0.12s;
}

.paste-remove:hover {
  color: var(--text-color);
  background: var(--hover-bg);
}

.paste-preview-enter-active {
  transition: all 0.2s ease-out;
}

.paste-preview-leave-active {
  transition: all 0.15s ease-in;
}

.paste-preview-enter-from,
.paste-preview-leave-to {
  opacity: 0;
  max-height: 0;
  margin-bottom: 0;
  transform: translateY(8px);
}

.paste-editor-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  backdrop-filter: blur(2px);
}

.paste-editor-modal {
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 14px;
  width: min(700px, 90vw);
  height: min(500px, 75vh);
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
  overflow: hidden;
}

.paste-editor-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.paste-editor-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.paste-editor-close {
  background: none;
  border: none;
  font-size: 20px;
  line-height: 1;
  color: var(--text-secondary);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 6px;
  transition: all 0.12s;
}

.paste-editor-close:hover {
  color: var(--text-color);
  background: var(--hover-bg);
}

.paste-editor-textarea {
  flex: 1;
  border: none;
  outline: none;
  resize: none;
  padding: 16px 18px;
  font-family: var(--font-mono-editor);
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-color);
  background: var(--input-bg);
  tab-size: 2;
}

.paste-editor-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 18px;
  border-top: 1px solid var(--border-color);
  flex-shrink: 0;
}

.paste-editor-info {
  font-size: 12px;
  color: var(--text-secondary);
}

.paste-editor-actions {
  display: flex;
  gap: 8px;
}

.paste-editor-btn {
  padding: 6px 16px;
  border-radius: 8px;
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  border: 1px solid var(--border-color);
  transition: all 0.12s;
}

.paste-editor-btn-danger {
  background: none;
  color: var(--status-danger-fg);
  border-color: color-mix(in srgb, var(--status-danger-fg) 28%, var(--border-color));
}

.paste-editor-btn-danger:hover {
  background: color-mix(in srgb, var(--status-danger-fg) 10%, transparent);
}

.paste-editor-btn-primary {
  background: var(--accent-color);
  color: var(--text-on-accent, #fff);
  border-color: transparent;
}

.paste-editor-btn-primary:hover {
  filter: brightness(1.05);
}

.paste-editor-overlay-enter-active {
  transition: opacity 0.2s ease-out;
}

.paste-editor-overlay-enter-active .paste-editor-modal {
  transition: transform 0.2s ease-out, opacity 0.2s ease-out;
}

.paste-editor-overlay-leave-active {
  transition: opacity 0.15s ease-in;
}

.paste-editor-overlay-leave-active .paste-editor-modal {
  transition: transform 0.15s ease-in, opacity 0.15s ease-in;
}

.paste-editor-overlay-enter-from,
.paste-editor-overlay-leave-to {
  opacity: 0;
}

.paste-editor-overlay-enter-from .paste-editor-modal,
.paste-editor-overlay-leave-to .paste-editor-modal {
  opacity: 0;
  transform: scale(0.95) translateY(10px);
}

.composer-header-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  width: 100%;
  min-width: 0;
}

.composer-header-start {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.composer-header-end {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}

.composer-badge-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.composer-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: var(--radius-badge);
  border: 1px solid transparent;
  background: var(--hover-bg);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: none;
}

.composer-badge.plan {
  color: var(--text-secondary);
  border-color: color-mix(in srgb, var(--accent-color) 18%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--input-bg) 28%);
}

.composer-badge.plan:hover {
  color: var(--text-color);
  border-color: color-mix(in srgb, var(--accent-color) 30%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 56%, var(--hover-bg) 44%);
}

.composer-badge.skill {
  color: var(--accent-color);
  border-color: color-mix(in srgb, var(--accent-color) 24%, transparent);
  background: color-mix(in srgb, var(--accent-color) 14%, transparent);
}

.composer-badge-remove {
  font-size: 14px;
  line-height: 1;
  opacity: 0.7;
}

.composer-top-badge {
  flex-shrink: 0;
}
</style>
