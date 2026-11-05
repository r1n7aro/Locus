<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { normalizeAppError } from "../services/errors";
import { knowledgeListDirectories, knowledgeListDirectoryDocumentsPage, knowledgeQuery } from "../services/knowledge";
import type { WorkspaceRef } from "../services/project";
import { watchWindowEventInterests } from "../services/windowEventInterests";
import type {
  KnowledgeChangedEvent,
  KnowledgeDocumentSummary,
  KnowledgeDocumentType,
  KnowledgeSearchResult,
} from "../types";
import { t } from "../i18n";
import { onWorkspaceEvent } from "./events";
import { KNOWLEDGE_TYPES } from "./references";

// Knowledge of one workspace: the five document types, then folders and
// documents of one folder per screen. Search covers every type at the top
// and one type inside it.
const props = defineProps<{
  workspaceRef: WorkspaceRef;
  projectName: string;
  docType: KnowledgeDocumentType | null;
  /** Folder relative to the type root, "" for the root. */
  path: string;
  connected: boolean;
  /** The screen is on top of the stack. */
  active: boolean;
  /** Bumped after a reconnect or dropped events. */
  refreshToken: number;
}>();

const emit = defineEmits<{
  back: [];
  openFolder: [docType: KnowledgeDocumentType, path: string];
  openDocument: [docType: KnowledgeDocumentType, path: string, title: string];
}>();

const PAGE_SIZE = 100;
const SEARCH_LIMIT = 50;

const folders = ref<string[]>([]);
const documents = ref<KnowledgeDocumentSummary[]>([]);
const cursor = ref<string | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);
const stale = ref(false);
const query = ref("");
const results = ref<KnowledgeSearchResult[] | null>(null);
const searching = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;
let searchRun = 0;

const title = computed(() => {
  if (!props.docType) return t("mobile.knowledge.title");
  return props.path ? props.path.split("/").pop()! : t(`knowledge.type.${props.docType}`);
});
const subtitle = computed(() => (props.docType
  ? [props.projectName, t(`knowledge.type.${props.docType}`), props.path].filter(Boolean).join(" / ")
  : props.projectName));
/** Search sits at the top of each tree (all types, or one type's root). */
const showSearch = computed(() => !props.path);

/** Directory paths may or may not carry the type root. */
function relativeToType(directory: string, docType: KnowledgeDocumentType): string {
  const normalized = directory.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  if (normalized === docType) return "";
  return normalized.startsWith(`${docType}/`) ? normalized.slice(docType.length + 1) : normalized;
}

function parentOf(path: string): string {
  const index = path.lastIndexOf("/");
  return index < 0 ? "" : path.slice(0, index);
}

async function load(reset: boolean) {
  const docType = props.docType;
  if (!docType || loading.value) return;
  loading.value = true;
  error.value = null;
  stale.value = false;
  try {
    const [directories, page] = await Promise.all([
      reset ? knowledgeListDirectories(docType, props.workspaceRef) : Promise.resolve(null),
      knowledgeListDirectoryDocumentsPage(docType, props.path, props.workspaceRef, {
        cursor: reset ? null : cursor.value,
        limit: reset ? Math.max(PAGE_SIZE, documents.value.length) : PAGE_SIZE,
      }),
    ]);
    if (directories) {
      folders.value = [...new Set(directories.map((directory) => relativeToType(directory, docType)))]
        .filter((directory) => directory && parentOf(directory) === props.path)
        .sort((a, b) => a.localeCompare(b));
    }
    documents.value = reset ? page.items : [...documents.value, ...page.items];
    cursor.value = page.nextCursor ?? null;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  } finally {
    loading.value = false;
  }
}

function folderName(path: string): string {
  return path.split("/").pop() ?? path;
}

function documentMeta(document: KnowledgeDocumentSummary): string {
  const parts = [document.summary?.trim() || document.path];
  if (document.storageSource === "app") parts.push(t("mobile.document.appSource"));
  return parts.join(" · ");
}

function resultMeta(result: KnowledgeSearchResult): string {
  const path = result.path.replace(/^\/+/, "");
  const relative = path.toLowerCase().startsWith(`${result.type}/`) ? path.slice(result.type.length + 1) : path;
  const parts = [t(`knowledge.type.${result.type}`), relative];
  if (result.storageSource === "app") parts.push(t("mobile.document.appSource"));
  return parts.join(" · ");
}

/** Snippets may carry Markdown and highlight markers; shown as plain text. */
function plainSnippet(snippet: string): string {
  return snippet.replace(/<\/?mark>/g, "").replace(/[*_`#>]+/g, "").replace(/\s+/g, " ").trim();
}

// ----- search.
watch(query, (value) => {
  if (searchTimer) clearTimeout(searchTimer);
  const text = value.trim();
  if (!text) {
    searchRun += 1;
    results.value = null;
    searching.value = false;
    return;
  }
  searching.value = true;
  searchTimer = setTimeout(async () => {
    const run = ++searchRun;
    try {
      const found = await knowledgeQuery(
        { query: text, limit: SEARCH_LIMIT, types: props.docType ? [props.docType] : undefined },
        props.workspaceRef,
      );
      if (run === searchRun) results.value = found;
    } catch (cause) {
      if (run === searchRun) {
        results.value = [];
        error.value = normalizeAppError(cause).message;
      }
    } finally {
      if (run === searchRun) searching.value = false;
    }
  }, 300);
});

// ----- following changes on the desktop.
const stopInterests = watchWindowEventInterests(() => [{ workspaceRef: props.workspaceRef }]);

function scheduleRefresh() {
  if (!props.docType) return;
  if (!props.active || !props.connected) {
    stale.value = true;
    return;
  }
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void load(true);
  }, 400);
}

function affectsThisFolder(change: KnowledgeChangedEvent): boolean {
  if (change.docType && change.docType !== props.docType) return false;
  // Content edits only matter for titles and summaries; structure and
  // config changes may add, move or remove entries.
  return change.changeKind !== "content" || change.targetKind === "document";
}

const stopEvents = onWorkspaceEvent((event) => {
  if (event.eventName !== "knowledge-changed" || event.checkoutId !== props.workspaceRef.checkoutId) return;
  if (affectsThisFolder(event.payload as KnowledgeChangedEvent)) scheduleRefresh();
});

watch(() => props.refreshToken, scheduleRefresh);
watch(() => [props.active, props.connected] as const, ([active, connected], previous) => {
  if (!active || !connected) return;
  if (stale.value || (previous && !previous[1])) void load(true);
});

onMounted(() => void load(true));
onBeforeUnmount(() => {
  stopInterests();
  stopEvents();
  if (searchTimer) clearTimeout(searchTimer);
  if (refreshTimer) clearTimeout(refreshTimer);
});
</script>

<template>
  <section class="m-screen m-knowledge">
    <header class="m-topbar">
      <button class="m-icon-button" type="button" :aria-label="t('mobile.common.back')" @click="emit('back')">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="m-topbar-text">
        <span class="m-topbar-title">{{ title }}</span>
        <span class="m-topbar-sub">{{ subtitle }}</span>
      </div>
      <button
        v-if="docType"
        class="m-icon-button"
        type="button"
        :aria-label="t('mobile.common.refresh')"
        :disabled="loading || !connected"
        @click="load(true)"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true" :class="{ 'is-spinning': loading }">
          <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5" />
        </svg>
      </button>
    </header>

    <div v-if="showSearch" class="m-searchbar">
      <input
        v-model="query"
        class="m-search-input"
        type="search"
        :placeholder="docType ? t('mobile.knowledge.searchType', t(`knowledge.type.${docType}`)) : t('mobile.knowledge.search')"
        :aria-label="t('mobile.knowledge.search')"
        autocapitalize="off"
        spellcheck="false"
      >
    </div>

    <div class="m-scroll m-list-scroll">
      <div v-if="error" class="m-notice" role="alert">{{ error }}</div>

      <template v-if="results !== null">
        <div v-if="searching && results.length === 0" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
        <div v-else-if="results.length === 0" class="m-placeholder">{{ t("mobile.knowledge.noResults") }}</div>
        <ul v-else class="m-entry-list">
          <li v-for="result in results" :key="result.id || `${result.type}:${result.path}`">
            <button class="m-entry" type="button" @click="emit('openDocument', result.type, result.path, result.title)">
              <span class="m-entry-icon" data-kind="doc" aria-hidden="true" />
              <span class="m-entry-text">
                <span class="m-entry-name">{{ result.title || folderName(result.path) }}</span>
                <span v-if="result.snippet" class="m-entry-snippet">{{ plainSnippet(result.snippet) }}</span>
                <span class="m-entry-meta">{{ resultMeta(result) }}</span>
              </span>
            </button>
          </li>
        </ul>
      </template>

      <ul v-else-if="!docType" class="m-entry-list">
        <li v-for="type in KNOWLEDGE_TYPES" :key="type">
          <button class="m-entry" type="button" @click="emit('openFolder', type, '')">
            <span class="m-entry-icon" data-kind="knowledge" aria-hidden="true" />
            <span class="m-entry-text">
              <span class="m-entry-name">{{ t(`knowledge.type.${type}`) }}</span>
              <span class="m-entry-meta">{{ t(`knowledge.type.${type}Desc`) }}</span>
            </span>
            <svg class="m-entry-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5l7 7-7 7" /></svg>
          </button>
        </li>
      </ul>

      <template v-else>
        <div v-if="loading && folders.length === 0 && documents.length === 0" class="m-placeholder">
          {{ t("mobile.common.loading") }}
        </div>
        <div v-else-if="!error && folders.length === 0 && documents.length === 0" class="m-placeholder">
          {{ t("mobile.knowledge.empty") }}
        </div>
        <ul v-else class="m-entry-list">
          <li v-for="folder in folders" :key="`dir:${folder}`">
            <button class="m-entry" type="button" @click="emit('openFolder', docType, folder)">
              <span class="m-entry-icon" data-kind="dir" aria-hidden="true" />
              <span class="m-entry-text">
                <span class="m-entry-name">{{ folderName(folder) }}</span>
              </span>
              <svg class="m-entry-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5l7 7-7 7" /></svg>
            </button>
          </li>
          <li v-for="document in documents" :key="document.id || document.path">
            <button class="m-entry" type="button" @click="emit('openDocument', docType, document.path, document.title)">
              <span class="m-entry-icon" :data-kind="document.path.endsWith('.csv') ? 'table' : 'doc'" aria-hidden="true" />
              <span class="m-entry-text">
                <span class="m-entry-name">{{ document.title || folderName(document.path) }}</span>
                <span class="m-entry-meta">{{ documentMeta(document) }}</span>
              </span>
            </button>
          </li>
        </ul>
        <button v-if="cursor" class="m-text-button m-more" type="button" :disabled="loading" @click="load(false)">
          {{ loading ? t("mobile.common.loading") : t("mobile.common.loadMore") }}
        </button>
      </template>
    </div>
  </section>
</template>
