import { prepareSearchText, scorePreparedSearchText } from "../../composables/searchMatcher";

export type MentionSearchEntryKind = "asset" | "knowledge" | "sceneObject";

export interface MentionSearchRankable {
  name: string;
  relPath: string;
  parentPath: string;
  meta?: string;
  matchScore: number;
  entryKind: MentionSearchEntryKind;
  source?: "tab" | "tree";
}

export const MAX_MENTION_SEARCH_RESULTS = 120;

const preparedFields = new WeakMap<MentionSearchRankable, {
  name: string; relPath: string; parentPath: string; meta?: string;
  matchScore: number; entryKind: MentionSearchEntryKind;
  fields: ReturnType<typeof searchFields>;
}>();

function sourcePriority(result: MentionSearchRankable): number {
  return result.source === "tab" ? 0 : result.source === "tree" ? 1 : 2;
}

export function mentionResultKey(result: Pick<MentionSearchRankable, "relPath"> & { entryKind?: MentionSearchEntryKind }): string {
  const path = result.relPath.replace(/\\/g, "/").replace(/\/+$/, "");
  if (result.entryKind === "sceneObject") return `scene:${path}`;
  // Knowledge refs use type-relative paths; workspace search returns the same
  // files under Locus/knowledge. Compare their physical workspace paths.
  const workspacePath = result.entryKind === "knowledge"
    && /^(?:design|plan|memory|skill|reference)\//i.test(path)
    ? `Locus/knowledge/${path}`
    : path;
  return workspacePath.toLowerCase();
}

function searchFields(result: MentionSearchRankable) {
  return [
    {
      text: result.name,
      weight: result.entryKind === "knowledge"
        ? 210 + Math.min(Math.floor(result.matchScore / 12), 60)
        : result.entryKind === "sceneObject"
          ? 190
          : 180 + Math.min(Math.floor(result.matchScore / 12), 90),
    },
    {
      text: result.relPath,
      weight: result.entryKind === "knowledge"
        ? 145 + Math.min(Math.floor(result.matchScore / 24), 35)
        : result.entryKind === "sceneObject"
          ? 135
          : 90 + Math.min(Math.floor(result.matchScore / 24), 45),
    },
    { text: result.parentPath, weight: 30 },
    { text: result.meta || "", weight: 50 },
  ].filter((field) => field.text).map((field) => ({
    text: prepareSearchText(field.text), weight: field.weight,
  }));
}

export function rankMentionSearchResults<T extends MentionSearchRankable>(
  results: T[],
  query: string,
  limit = MAX_MENTION_SEARCH_RESULTS,
): T[] {
  const unique = new Map<string, T>();
  for (const result of results) {
    const key = mentionResultKey(result);
    const previous = unique.get(key);
    if (!previous || sourcePriority(result) < sourcePriority(previous)
      || (sourcePriority(result) === sourcePriority(previous)
        && result.entryKind === "knowledge" && previous.entryKind !== "knowledge")) {
      unique.set(key, result);
    }
  }
  const preparedQuery = prepareSearchText(query);
  const matches: Array<{ item: T; score: number }> = [];
  for (const item of unique.values()) {
    if (!preparedQuery.compact) {
      matches.push({ item, score: 0 });
      continue;
    }
    let cached = preparedFields.get(item);
    if (!cached || cached.name !== item.name || cached.relPath !== item.relPath
      || cached.parentPath !== item.parentPath || cached.meta !== item.meta
      || cached.matchScore !== item.matchScore || cached.entryKind !== item.entryKind) {
      cached = {
        name: item.name, relPath: item.relPath, parentPath: item.parentPath, meta: item.meta,
        matchScore: item.matchScore, entryKind: item.entryKind, fields: searchFields(item),
      };
      preparedFields.set(item, cached);
    }
    let best = -Infinity;
    for (const field of cached.fields) {
      const score = scorePreparedSearchText(preparedQuery, field.text);
      if (score !== null) best = Math.max(best, score + field.weight);
    }
    if (best !== -Infinity) matches.push({ item, score: best });
  }
  return matches.sort((a, b) => sourcePriority(a.item) - sourcePriority(b.item) || b.score - a.score)
    .slice(0, limit).map((match) => match.item);
}
