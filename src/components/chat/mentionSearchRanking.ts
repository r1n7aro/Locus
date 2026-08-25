import { rankSearchResults } from "../../composables/searchMatcher";

export type MentionSearchEntryKind = "asset" | "knowledge" | "sceneObject";

export interface MentionSearchRankable {
  name: string;
  relPath: string;
  parentPath: string;
  meta?: string;
  matchScore: number;
  entryKind: MentionSearchEntryKind;
}

export function rankMentionSearchResults<T extends MentionSearchRankable>(
  results: T[],
  query: string,
): T[] {
  return rankSearchResults(results, query, (result) => [
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
  ]);
}
