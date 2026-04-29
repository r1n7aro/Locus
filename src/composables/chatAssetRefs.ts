import {
  findUnityAssetPathEnd,
  findUnitySceneObjectPathEnd,
} from "./markdownInject";

export interface ChatAssetRefSegment {
  type: "text" | "asset";
  value: string;
}

const UNITY_ASSET_REF_START_RE = /`@?(?:Assets|Packages|ProjectSettings)\/|\{@(?:Assets|Packages|ProjectSettings)\/|@(?:Assets|Packages|ProjectSettings)\//g;
const UNITY_ASSET_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)\//;

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
    const backticked = match[0].startsWith("`");
    const braced = match[0].startsWith("{@");
    const pathStart = markerStart + (backticked ? (match[0].startsWith("`@") ? 2 : 1) : braced ? 2 : 1);
    const end = backticked ? text.indexOf("`", pathStart) : findAssetMentionEnd(text, pathStart);
    if (end < 0) continue;
    const pathValue = normalizeAssetSegmentValue(text.slice(pathStart, end).replace(/\\/g, "/"));
    if (!UNITY_ASSET_ROOT_RE.test(pathValue)) continue;
    const tokenEnd = backticked ? end + 1 : braced && text[end] === "}" ? end + 1 : end;

    if (markerStart > cursor) {
      segments.push({ type: "text", value: text.slice(cursor, markerStart) });
    }
    segments.push({ type: "asset", value: pathValue });
    cursor = tokenEnd;
    UNITY_ASSET_REF_START_RE.lastIndex = tokenEnd;
  }

  if (cursor < text.length) {
    segments.push({ type: "text", value: text.slice(cursor) });
  }

  return segments;
}
