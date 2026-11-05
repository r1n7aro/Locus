<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { workspaceFileChangeInScope } from "../composables/useFileChangeRevalidation";
import { normalizeAppError } from "../services/errors";
import {
  listDirEntriesPage,
  searchWorkspaceEntries,
  type DirEntry,
  type WorkspaceRef,
  type WorkspaceSearchEntry,
} from "../services/project";
import { subscribeWorkspaceFileChanges } from "../services/workspaceExplorer";
import { retainFileWatch } from "./fileWatches";
import { t } from "../i18n";

// One directory of an open workspace; subdirectories open as new screens so
// the back gesture walks up the tree.
const props = defineProps<{
  workspaceRef: WorkspaceRef;
  projectName: string;
  /** Workspace-relative directory, "" for the root. */
  path: string;
  connected: boolean;
  /** The screen is on top of the stack. */
  active: boolean;
  /** Bumped after a reconnect or dropped events. */
  refreshToken: number;
}>();

const emit = defineEmits<{
  back: [];
  openDirectory: [path: string];
  openFile: [path: string, name: string];
}>();

const PAGE_SIZE = 200;
const entries = ref<DirEntry[]>([]);
const nextOffset = ref(0);
const hasMore = ref(false);
const loading = ref(false);
const error = ref<string | null>(null);
const query = ref("");
const results = ref<WorkspaceSearchEntry[] | null>(null);
const searching = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let searchRun = 0;

const title = computed(() => (props.path ? props.path.split("/").pop() : props.projectName) || props.projectName);
const subtitle = computed(() => (props.path ? `${props.projectName}/${props.path}` : t("mobile.files.title")));

function sortEntries(list: DirEntry[]): DirEntry[] {
  return [...list].sort((a, b) => Number(b.isDir) - Number(a.isDir) || a.name.localeCompare(b.name));
}

/** `reset` reloads from the top; `keep` asks for at least as many entries as
 * are listed now, so a refresh does not collapse loaded pages. */
async function load(reset: boolean, keep = false) {
  if (loading.value) return;
  loading.value = true;
  error.value = null;
  stale.value = false;
  try {
    const limit = keep ? Math.min(2000, Math.max(PAGE_SIZE, entries.value.length)) : PAGE_SIZE;
    const page = await listDirEntriesPage(props.path, props.workspaceRef, reset ? 0 : nextOffset.value, limit, true);
    entries.value = sortEntries(reset ? page.entries : [...entries.value, ...page.entries]);
    nextOffset.value = page.nextOffset;
    hasMore.value = page.hasMore;
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  } finally {
    loading.value = false;
  }
}

watch(query, (value) => {
  if (searchTimer) clearTimeout(searchTimer);
  const text = value.trim();
  if (!text) {
    results.value = null;
    searching.value = false;
    return;
  }
  searching.value = true;
  searchTimer = setTimeout(async () => {
    const run = ++searchRun;
    try {
      const found = await searchWorkspaceEntries(text, props.workspaceRef, 100);
      if (run === searchRun) results.value = found;
    } catch (cause) {
      if (run === searchRun) error.value = normalizeAppError(cause).message;
    } finally {
      if (run === searchRun) searching.value = false;
    }
  }, 250);
});

function open(entry: { relPath: string; name: string; isDir: boolean }) {
  if (entry.isDir) emit("openDirectory", entry.relPath);
  else emit("openFile", entry.relPath, entry.name);
}

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif"]);
const DOC_EXTENSIONS = new Set(["md", "markdown", "mdx", "txt"]);
const TABLE_EXTENSIONS = new Set(["csv", "tsv"]);

function entryKind(entry: { name: string; isDir: boolean }): string {
  if (entry.isDir) return "dir";
  const dot = entry.name.lastIndexOf(".");
  const extension = dot > 0 ? entry.name.slice(dot + 1).toLowerCase() : "";
  if (IMAGE_EXTENSIONS.has(extension)) return "image";
  if (DOC_EXTENSIONS.has(extension)) return "doc";
  if (TABLE_EXTENSIONS.has(extension)) return "table";
  return "file";
}

// ----- following changes on the desktop: entries of this folder.
const stale = ref(false);
let refreshTimer: ReturnType<typeof setTimeout> | null = null;
let releaseChanges: (() => void) | null = null;
let unmounted = false;

function normalizeDir(path: string): string {
  return path.replace(/\\/g, "/").replace(/^(?:\.\/)+/, "").replace(/^\/+|\/+$/g, "").toLowerCase();
}

function scheduleRefresh() {
  if (!props.active || !props.connected) {
    stale.value = true;
    return;
  }
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void load(true, true);
  }, 400);
}

function affectsThisFolder(changedPath: string, rescan: boolean): boolean {
  const changed = normalizeDir(changedPath);
  const folder = normalizeDir(props.path);
  if (!changed || changed === ".") return true;
  const parent = changed.includes("/") ? changed.slice(0, changed.lastIndexOf("/")) : "";
  if (parent === folder) return true;
  // A rescanned ancestor may have replaced this folder's contents.
  return rescan && (folder === changed || folder.startsWith(`${changed}/`));
}

watch(() => props.refreshToken, scheduleRefresh);
watch(() => [props.active, props.connected] as const, ([active, connected], previous) => {
  if (!active || !connected) return;
  if (stale.value || (previous && !previous[1])) void load(true, true);
});

// The desktop forwards file changes only for watched paths.
const releaseWatch = retainFileWatch(props.workspaceRef.checkoutId, props.path);

onMounted(() => {
  void load(true);
  void subscribeWorkspaceFileChanges((event) => {
    if (!workspaceFileChangeInScope(event, props.workspaceRef)) return;
    if (affectsThisFolder(event.payload.path, !!event.payload.rescan)) scheduleRefresh();
  }, () => props.workspaceRef).then((release) => {
    if (unmounted) release();
    else releaseChanges = release;
  }).catch(() => {});
});
onBeforeUnmount(() => {
  unmounted = true;
  releaseWatch();
  if (searchTimer) clearTimeout(searchTimer);
  if (refreshTimer) clearTimeout(refreshTimer);
  releaseChanges?.();
});
</script>

<template>
  <section class="m-screen m-files">
    <header class="m-topbar">
      <button class="m-icon-button" type="button" :aria-label="t('mobile.common.back')" @click="emit('back')">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="m-topbar-text">
        <span class="m-topbar-title">{{ title }}</span>
        <span class="m-topbar-sub">{{ subtitle }}</span>
      </div>
      <button
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

    <div v-if="!path" class="m-searchbar">
      <input
        v-model="query"
        class="m-search-input"
        type="search"
        :placeholder="t('mobile.files.search')"
        :aria-label="t('mobile.files.search')"
        autocapitalize="off"
        spellcheck="false"
      >
    </div>

    <div class="m-scroll m-list-scroll">
      <div v-if="error" class="m-notice" role="alert">{{ error }}</div>
      <template v-if="results !== null">
        <div v-if="searching && results.length === 0" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
        <div v-else-if="results.length === 0" class="m-placeholder">{{ t("mobile.files.noResults") }}</div>
        <ul v-else class="m-entry-list">
          <li v-for="entry in results" :key="entry.relPath">
            <button class="m-entry" type="button" @click="open(entry)">
              <span class="m-entry-icon" :data-kind="entryKind(entry)" aria-hidden="true" />
              <span class="m-entry-text">
                <span class="m-entry-name">{{ entry.name }}</span>
                <span class="m-entry-meta">{{ entry.parentPath || "/" }}</span>
              </span>
            </button>
          </li>
        </ul>
      </template>
      <template v-else>
        <div v-if="loading && entries.length === 0" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
        <div v-else-if="!error && entries.length === 0" class="m-placeholder">{{ t("mobile.files.empty") }}</div>
        <ul v-else class="m-entry-list">
          <li v-for="entry in entries" :key="entry.relPath">
            <button class="m-entry" type="button" @click="open(entry)">
              <span class="m-entry-icon" :data-kind="entryKind(entry)" aria-hidden="true" />
              <span class="m-entry-text">
                <span class="m-entry-name">{{ entry.name }}</span>
              </span>
              <svg v-if="entry.isDir" class="m-entry-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5l7 7-7 7" /></svg>
            </button>
          </li>
        </ul>
        <button v-if="hasMore" class="m-text-button m-more" type="button" :disabled="loading" @click="load(false)">
          {{ loading ? t("mobile.common.loading") : t("mobile.common.loadMore") }}
        </button>
      </template>
    </div>
  </section>
</template>
