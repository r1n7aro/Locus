import {
  findUnityAssetPathEnd,
  findUnitySceneObjectPathEnd,
} from "./markdownInject";

export interface ChatAssetRefSegment {
  type: "text" | "asset";
  value: string;
}

const UNITY_ASSET_REF_START_RE = /@(?:Assets|Packages)\//g;

function findSimpleAssetMentionEnd(text: string, start: number): number {
  let end = start;
  while (end < text.length && !/[\s@<>"'`，。；、？！,;:\])}）】》」』]/.test(text[end])) {
    end++;
  }
  return end > start && text.slice(start, end).includes("/") ? end : -1;
}

function findAssetMentionEnd(text: string, start: number): number {
  const sceneObjectEnd = findUnitySceneObjectPathEnd(text, start);
  if (sceneObjectEnd >= 0) return sceneObjectEnd;

  const assetEnd = findUnityAssetPathEnd(text, start);
  if (assetEnd >= 0) return assetEnd;

  return findSimpleAssetMentionEnd(text, start);
}

function normalizeAssetSegmentValue(value: string): string {
  const trimmed = value.trimEnd();
  return trimmed.replace(/\/+$/, "") || trimmed;
}

export function parseChatAssetRefs(text: string): ChatAssetRefSegment[] {
  const segments: ChatAssetRefSegment[] = [];
  let cursor = 0;
  UNITY_ASSET_REF_START_RE.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = UNITY_ASSET_REF_START_RE.exec(text)) !== null) {
    const markerStart = match.index;
    const pathStart = markerStart + 1;
    const end = findAssetMentionEnd(text, pathStart);
    if (end < 0) continue;

    if (markerStart > cursor) {
      segments.push({ type: "text", value: text.slice(cursor, markerStart) });
    }
    segments.push({ type: "asset", value: normalizeAssetSegmentValue(text.slice(pathStart, end)) });
    cursor = end;
    UNITY_ASSET_REF_START_RE.lastIndex = end;
  }

  if (cursor < text.length) {
    segments.push({ type: "text", value: text.slice(cursor) });
  }

  return segments;
}
