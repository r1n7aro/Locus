import { computed, shallowRef } from "vue";
import { normalizeAppError } from "../services/errors";
import { globalSearchHitKey, searchGlobalPage, type GlobalSearchResult, type GlobalSearchTarget, type GlobalSearchRequest, type GlobalSearchSource } from "../services/globalSearch";
import type { GlobalSearchSettings } from "./useGlobalSearchSettings";

type Family = "knowledge" | "session";
interface Lane { source: GlobalSearchSource; archived: boolean; cursor: string | null; target: GlobalSearchTarget }
interface Run {
  query: string;
  lanes: Record<Family, Lane[]>;
  hits: Record<Family, GlobalSearchResult[]>;
  ordered: GlobalSearchResult[];
  keys: Set<string>;
  limit: number;
}

export function useGlobalSearch(fetchPage = searchGlobalPage) {
  const results = shallowRef<GlobalSearchResult[]>([]);
  const searching = shallowRef(false);
  const hasMore = shallowRef(false);
  const errors = shallowRef<string[]>([]);
  let run: Run | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  // Serialize each family even across query changes. A burst of typing cannot
  // fill the Rust worker queue with obsolete searches.
  const tails: Record<Family, Promise<void>> = { knowledge: Promise.resolve(), session: Promise.resolve() };

  function cancel() {
    clearTimeout(timer);
    run = null;
    searching.value = false;
    hasMore.value = false;
  }
  function publish(current: Run) {
    if (run !== current) return;
    results.value = [...current.ordered];
    hasMore.value = Object.values(current.lanes).some((lanes) => lanes.length > 0);
  }
  async function scan(current: Run, family: Family) {
    while (run === current && current.lanes[family].length && current.hits[family].length < current.limit) {
      const lane = current.lanes[family][0]!;
      const request: GlobalSearchRequest = { source: lane.source, archived: lane.archived, cursor: lane.cursor, query: current.query, workspaceRef: lane.target.workspaceRef };
      try {
        const page = await fetchPage(request);
        if (run !== current) return;
        for (const hit of page.matches) {
          const key = lane.target.projectId + globalSearchHitKey(hit);
          if (current.keys.has(key)) {
            if (hit.field === "content") {
              const index = current.hits[family].findIndex((item) => item.target.projectId + globalSearchHitKey(item) === key);
              // Enrich a title match with its first matching content excerpt.
              if (index >= 0 && current.hits[family][index]!.field === "title") {
                const enriched = { ...hit, target: lane.target };
                const orderedIndex = current.ordered.indexOf(current.hits[family][index]!);
                current.hits[family][index] = enriched;
                if (orderedIndex >= 0) current.ordered[orderedIndex] = enriched;
              }
            }
            continue;
          }
          current.keys.add(key);
          const result = { ...hit, target: lane.target };
          current.hits[family].push(result);
          current.ordered.push(result);
        }
        current.lanes[family].shift();
        if (page.nextCursor) {
          lane.cursor = page.nextCursor;
          current.lanes[family].push(lane);
        }
        publish(current);
      } catch (error) {
        if (run !== current) return;
        errors.value = [...errors.value, `${lane.target.projectName}: ${normalizeAppError(error).message}`];
        current.lanes[family].shift();
        publish(current);
      }
    }
  }
  function resume(current: Run) {
    searching.value = true;
    const tasks = (["knowledge", "session"] as const).map((family) => {
      tails[family] = tails[family].then(() => scan(current, family));
      return tails[family];
    });
    void Promise.all(tasks).finally(() => {
      if (run === current) searching.value = false;
    });
  }
  function search(query: string, targets: GlobalSearchTarget[], settings: GlobalSearchSettings) {
    cancel();
    results.value = [];
    errors.value = [];
    const text = query.trim();
    if (!settings.enabled || !targets.length || !text || text.length > 200) return;
    const lanes: Run["lanes"] = { knowledge: [], session: [] };
    for (const source of ["knowledgeTitle", "knowledgeContent", "sessionTitle", "sessionContent"] as const) {
      if (!settings[source]) continue;
      const family = source.startsWith("knowledge") ? "knowledge" : "session";
      for (const value of targets) {
        const target = { ...value, workspaceRef: { ...value.workspaceRef } };
        lanes[family].push({ source, archived: false, cursor: null, target });
        if (family === "session") lanes[family].push({ source, archived: true, cursor: null, target });
      }
    }
    if (!lanes.knowledge.length && !lanes.session.length) return;
    const current: Run = { query: text, lanes, hits: { knowledge: [], session: [] }, ordered: [], keys: new Set(), limit: 40 };
    run = current;
    searching.value = true;
    timer = setTimeout(() => resume(current), 120);
  }
  function loadMore() {
    if (!run || searching.value || !hasMore.value) return;
    run.limit += 40;
    resume(run);
  }
  return { results, searching, hasMore, errors, search, cancel, loadMore,
    empty: computed(() => !searching.value && !results.value.length && !errors.value.length) };
}
