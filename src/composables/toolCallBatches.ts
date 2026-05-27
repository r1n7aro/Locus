import type { AssistantRenderPart, ChatMessage, ImageAttachment, ToolCallDisplay, ToolCallInfo } from "../types";

const INTERRUPTED_TOOL_RESULT = "工具执行被用户中止，未返回结果。";
const GENERIC_ARGUMENT_ALIAS_GROUPS: Array<readonly [string, readonly string[]]> = [
  ["filePath", ["filePath", "file_path"]],
  ["oldString", ["oldString", "old_string"]],
  ["newString", ["newString", "new_string"]],
  ["replaceAll", ["replaceAll", "replace_all"]],
  ["editorStatus", ["editorStatus", "editor_status"]],
  ["requestEditorStatus", ["requestEditorStatus", "request_editor_status"]],
  ["assetPath", ["assetPath", "asset_path"]],
  ["maxDepth", ["maxDepth", "max_depth"]],
  ["typeFilter", ["typeFilter", "type_filter"]],
  ["objectPath", ["objectPath", "object_path"]],
  ["includeFiles", ["includeFiles", "include_files"]],
  ["maxItems", ["maxItems", "max_items"]],
  ["maxTotal", ["maxTotal", "max_total"]],
  ["scenePath", ["scenePath", "scene_path"]],
  ["sourceField", ["sourceField", "source_field"]],
  ["subagentType", ["subagentType", "subagent_type"]],
];

const TOOL_SPECIFIC_ARGUMENT_ALIAS_GROUPS: Partial<
  Record<string, Array<readonly [string, readonly string[]]>>
> = {
  read: [["filePath", ["filePath", "file_path", "path"]]],
  write: [["filePath", ["filePath", "file_path", "path"]]],
  edit: [["filePath", ["filePath", "file_path", "path"]]],
  list: [["path", ["path", "filePath", "file_path"]]],
  grep: [["path", ["path", "filePath", "file_path"]]],
};

const PATH_LIKE_ARGUMENT_KEYS = new Set([
  "filePath",
  "path",
  "assetPath",
  "objectPath",
  "scenePath",
]);

export interface ToolCallMatchState {
  ids: Set<string>;
  fingerprintCounts: Map<string, number>;
  idFingerprints: Map<string, string>;
}

export interface ToolCallBatchState {
  total: number;
  doneCount: number;
  runningCount: number;
  errorCount: number;
  interruptedCount: number;
  canCollapse: boolean;
}

export interface AssistantToolMergeCandidate {
  id: string;
  content: string;
  thinkingContent?: string;
  renderParts?: unknown[];
  toolCalls?: ToolCallInfo[];
  attachedKnowledgeProposalCount?: number;
  isKnowledgeProposal?: boolean;
}

export type AssistantToolMergeResult<T> = T & {
  displayToolCalls?: ToolCallInfo[];
  displayToolCallsBeforeContent?: ToolCallInfo[];
  displayToolCallsAfterContent?: ToolCallInfo[];
};

export interface ToolCallInfoRenderSource {
  messageToolCalls?: ToolCallInfo[];
  displayToolCalls?: ToolCallInfo[];
}

export interface ToolCallDisplayShape {
  name: string;
  arguments: string;
}

interface OrderedToolCallLike {
  order?: number;
  nestedToolCalls?: readonly OrderedToolCallLike[];
}

export interface ToolCallRenderOrderSegment<T> {
  order: number;
  toolCalls: T[];
}

export function firstToolCallRenderOrder(toolCalls: readonly OrderedToolCallLike[]) {
  let order = Number.POSITIVE_INFINITY;
  const visit = (items: readonly OrderedToolCallLike[]) => {
    for (const toolCall of items) {
      if (typeof toolCall.order === "number" && toolCall.order > 0) {
        order = Math.min(order, toolCall.order);
      }
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };
  visit(toolCalls);
  return Number.isFinite(order) ? order : 0;
}

export function lastToolCallRenderOrder(toolCalls: readonly OrderedToolCallLike[]) {
  let order = 0;
  const visit = (items: readonly OrderedToolCallLike[]) => {
    for (const toolCall of items) {
      if (typeof toolCall.order === "number" && toolCall.order > 0) {
        order = Math.max(order, toolCall.order);
      }
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };
  visit(toolCalls);
  return order;
}

export function hasVisibleTextPartAfterToolCalls(
  parts: readonly AssistantRenderPart[],
  toolCalls: readonly OrderedToolCallLike[],
) {
  const lastToolOrder = lastToolCallRenderOrder(toolCalls);
  if (lastToolOrder <= 0) return false;
  return parts.some((part) =>
    part.kind === "text"
    && part.content.trim().length > 0
    && part.order.seq > lastToolOrder,
  );
}

export function splitToolCallsByRenderOrder<T extends OrderedToolCallLike>(
  toolCalls: readonly T[],
  options: { fallbackOrder: number; boundaryOrders?: readonly number[] },
): Array<ToolCallRenderOrderSegment<T>> {
  const boundaryOrders = [...(options.boundaryOrders ?? [])]
    .filter((order) => Number.isFinite(order) && order > 0)
    .sort((left, right) => left - right);
  const entries = toolCalls
    .map((toolCall, index) => ({
      toolCall,
      index,
      order: firstToolCallRenderOrder([toolCall]) || options.fallbackOrder,
    }))
    .sort((left, right) => left.order - right.order || left.index - right.index);
  const segments: Array<ToolCallRenderOrderSegment<T>> = [];

  const hasBoundaryBetween = (leftOrder: number, rightOrder: number) =>
    boundaryOrders.some((boundaryOrder) =>
      boundaryOrder > leftOrder && boundaryOrder <= rightOrder,
    );

  for (const entry of entries) {
    const current = segments[segments.length - 1];
    if (!current || hasBoundaryBetween(current.order, entry.order)) {
      segments.push({ order: entry.order, toolCalls: [entry.toolCall] });
      continue;
    }
    current.toolCalls.push(entry.toolCall);
  }

  return segments;
}

function stableSerialize(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableSerialize(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, nestedValue]) => `${JSON.stringify(key)}:${stableSerialize(nestedValue)}`);
    return `{${entries.join(",")}}`;
  }
  return JSON.stringify(value);
}

function normalizePathLikeArgument(value: string): string {
  return value.replace(/\\/g, "/");
}

function normalizeArgumentValue(key: string, value: unknown): unknown {
  if (typeof value === "string" && PATH_LIKE_ARGUMENT_KEYS.has(key)) {
    return normalizePathLikeArgument(value);
  }
  return value;
}

function resolveAliasValue(
  source: Record<string, unknown>,
  aliases: readonly string[],
): unknown {
  for (const alias of aliases) {
    const value = source[alias];
    if (value !== undefined) {
      return value;
    }
  }
  return undefined;
}

function canonicalizeArgumentObject(
  toolName: string,
  value: Record<string, unknown>,
  isRoot: boolean,
): Record<string, unknown> {
  const canonical: Record<string, unknown> = {};

  for (const [key, nestedValue] of Object.entries(value)) {
    canonical[key] = canonicalizeArgumentValue(toolName, nestedValue, false);
  }

  const aliasGroups = [
    ...GENERIC_ARGUMENT_ALIAS_GROUPS,
    ...(isRoot ? (TOOL_SPECIFIC_ARGUMENT_ALIAS_GROUPS[toolName] ?? []) : []),
  ];

  for (const [canonicalKey, aliases] of aliasGroups) {
    const resolved = resolveAliasValue(canonical, aliases);
    for (const alias of aliases) {
      delete canonical[alias];
    }
    if (resolved !== undefined) {
      canonical[canonicalKey] = normalizeArgumentValue(canonicalKey, resolved);
    }
  }

  const normalized: Record<string, unknown> = {};
  for (const [key, nestedValue] of Object.entries(canonical)) {
    normalized[key] = normalizeArgumentValue(key, nestedValue);
  }
  return normalized;
}

function canonicalizeArgumentValue(
  toolName: string,
  value: unknown,
  isRoot: boolean,
): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => canonicalizeArgumentValue(toolName, item, false));
  }
  if (value && typeof value === "object") {
    return canonicalizeArgumentObject(toolName, value as Record<string, unknown>, isRoot);
  }
  return value;
}

function normalizeToolCallArguments(argumentsText: string): string {
  try {
    return stableSerialize(canonicalizeArgumentValue("", JSON.parse(argumentsText), true));
  } catch {
    return argumentsText.trim();
  }
}

export function resolveToolCallDisplayShape(
  toolCall: ToolCallDisplayShape,
): ToolCallDisplayShape {
  if (toolCall.name !== "tool_call") {
    return toolCall;
  }
  try {
    const parsed = JSON.parse(toolCall.arguments) as {
      toolName?: unknown;
      arguments?: unknown;
    };
    const targetName = typeof parsed.toolName === "string" ? parsed.toolName.trim() : "";
    if (!targetName) {
      return toolCall;
    }
    const targetArguments =
      parsed.arguments && typeof parsed.arguments === "object" && !Array.isArray(parsed.arguments)
        ? parsed.arguments
        : {};
    return {
      name: targetName,
      arguments: stableSerialize(canonicalizeArgumentValue(targetName, targetArguments, true)),
    };
  } catch {
    return toolCall;
  }
}

export function getToolCallInfoFingerprint(toolCall: Pick<ToolCallInfo, "name" | "arguments" | "nestedToolCalls">): string {
  const nestedFingerprints = toolCall.nestedToolCalls?.map((nestedToolCall) => getToolCallInfoFingerprint(nestedToolCall)) ?? [];
  const display = resolveToolCallDisplayShape(toolCall);
  return `${display.name}\u241f${normalizeToolCallArgumentsForTool(display.name, display.arguments)}\u241f${nestedFingerprints.join("\u241e")}`;
}

export function getToolCallDisplayFingerprint(toolCall: Pick<ToolCallDisplay, "name" | "arguments" | "nestedToolCalls">): string {
  const nestedFingerprints =
    toolCall.nestedToolCalls?.map((nestedToolCall) => getToolCallDisplayFingerprint(nestedToolCall)) ?? [];
  const display = resolveToolCallDisplayShape(toolCall);
  return `${display.name}\u241f${normalizeToolCallArgumentsForTool(display.name, display.arguments)}\u241f${nestedFingerprints.join("\u241e")}`;
}

function normalizeToolCallArgumentsForTool(toolName: string, argumentsText: string): string {
  try {
    return stableSerialize(canonicalizeArgumentValue(toolName, JSON.parse(argumentsText), true));
  } catch {
    return normalizeToolCallArguments(argumentsText);
  }
}

function incrementCount(map: Map<string, number>, key: string) {
  map.set(key, (map.get(key) ?? 0) + 1);
}

export function collectToolCallDisplayIds(toolCalls: ToolCallDisplay[]): Set<string> {
  const ids = new Set<string>();

  const visit = (items: ToolCallDisplay[]) => {
    for (const toolCall of items) {
      ids.add(toolCall.id);
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };

  visit(toolCalls);
  return ids;
}

export function collectToolCallDisplayMatchState(toolCalls: ToolCallDisplay[]): ToolCallMatchState {
  const ids = new Set<string>();
  const fingerprintCounts = new Map<string, number>();
  const idFingerprints = new Map<string, string>();

  const visit = (items: ToolCallDisplay[]) => {
    for (const toolCall of items) {
      const fingerprint = getToolCallDisplayFingerprint(toolCall);
      ids.add(toolCall.id);
      idFingerprints.set(toolCall.id, fingerprint);
      incrementCount(fingerprintCounts, fingerprint);
      if (toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0) {
        visit(toolCall.nestedToolCalls);
      }
    }
  };

  visit(toolCalls);
  return { ids, fingerprintCounts, idFingerprints };
}

export function collectToolCallDisplayIdMatchState(toolCalls: ToolCallDisplay[]): ToolCallMatchState {
  return {
    ids: collectToolCallDisplayIds(toolCalls),
    fingerprintCounts: new Map<string, number>(),
    idFingerprints: new Map<string, string>(),
  };
}

export function mergeToolCallMatchStates(...states: ToolCallMatchState[]): ToolCallMatchState {
  const ids = new Set<string>();
  const fingerprintCounts = new Map<string, number>();
  const idFingerprints = new Map<string, string>();

  for (const state of states) {
    for (const id of state.ids) {
      ids.add(id);
    }
    for (const [id, fingerprint] of state.idFingerprints) {
      idFingerprints.set(id, fingerprint);
    }
    for (const [fingerprint, count] of state.fingerprintCounts) {
      fingerprintCounts.set(fingerprint, (fingerprintCounts.get(fingerprint) ?? 0) + count);
    }
  }

  return { ids, fingerprintCounts, idFingerprints };
}

export function cloneToolCallMatchState(state: ToolCallMatchState): ToolCallMatchState {
  return {
    ids: new Set(state.ids),
    fingerprintCounts: new Map(state.fingerprintCounts),
    idFingerprints: new Map(state.idFingerprints),
  };
}

export function areToolCallDisplaysCoveredByMatchState(
  toolCalls: ToolCallDisplay[],
  state: ToolCallMatchState,
): boolean {
  if (toolCalls.length === 0) return false;
  if (state.ids.size === 0 && state.fingerprintCounts.size === 0) return false;

  const remainingState = cloneToolCallMatchState(state);
  return toolCalls.every((toolCall) => consumeDisplayMatch(toolCall, remainingState));
}

function consumeFingerprintMatch(state: ToolCallMatchState, fingerprint: string): boolean {
  const remaining = state.fingerprintCounts.get(fingerprint) ?? 0;
  if (remaining <= 0) return false;
  if (remaining === 1) {
    state.fingerprintCounts.delete(fingerprint);
  } else {
    state.fingerprintCounts.set(fingerprint, remaining - 1);
  }
  return true;
}

function consumeIdMatch(state: ToolCallMatchState, id: string, fallbackFingerprint: string): boolean {
  if (!state.ids.has(id)) return false;
  const fingerprint = state.idFingerprints.get(id) ?? fallbackFingerprint;
  state.ids.delete(id);
  state.idFingerprints.delete(id);
  consumeFingerprintMatch(state, fingerprint);
  return true;
}

function consumeFingerprintAndOneId(state: ToolCallMatchState, fingerprint: string): boolean {
  if (!consumeFingerprintMatch(state, fingerprint)) return false;
  for (const [id, storedFingerprint] of state.idFingerprints) {
    if (storedFingerprint !== fingerprint) continue;
    state.ids.delete(id);
    state.idFingerprints.delete(id);
    break;
  }
  return true;
}

function consumeInfoTreeMatchState(toolCall: ToolCallInfo, state: ToolCallMatchState) {
  const fingerprint = getToolCallInfoFingerprint(toolCall);
  if (!consumeIdMatch(state, toolCall.id, fingerprint)) {
    consumeFingerprintAndOneId(state, fingerprint);
  }
  for (const nestedToolCall of toolCall.nestedToolCalls ?? []) {
    consumeInfoTreeMatchState(nestedToolCall, state);
  }
}

function consumeDisplayTreeMatchState(toolCall: ToolCallDisplay, state: ToolCallMatchState) {
  const fingerprint = getToolCallDisplayFingerprint(toolCall);
  if (!consumeIdMatch(state, toolCall.id, fingerprint)) {
    consumeFingerprintAndOneId(state, fingerprint);
  }
  for (const nestedToolCall of toolCall.nestedToolCalls ?? []) {
    consumeDisplayTreeMatchState(nestedToolCall, state);
  }
}

function consumeInfoMatch(
  toolCall: ToolCallInfo,
  state: ToolCallMatchState,
): boolean {
  const fingerprint = getToolCallInfoFingerprint(toolCall);
  if (state.ids.has(toolCall.id) || (state.fingerprintCounts.get(fingerprint) ?? 0) > 0) {
    consumeInfoTreeMatchState(toolCall, state);
    return true;
  }
  return false;
}

function consumeDisplayMatch(
  toolCall: ToolCallDisplay,
  state: ToolCallMatchState,
): boolean {
  const fingerprint = getToolCallDisplayFingerprint(toolCall);
  if (state.ids.has(toolCall.id) || (state.fingerprintCounts.get(fingerprint) ?? 0) > 0) {
    consumeDisplayTreeMatchState(toolCall, state);
    return true;
  }
  return false;
}

function filterToolCallInfoArray(
  toolCalls: ToolCallInfo[],
  state: ToolCallMatchState,
): ToolCallInfo[] {
  const filtered: ToolCallInfo[] = [];
  for (const toolCall of toolCalls) {
    if (consumeInfoMatch(toolCall, state)) continue;
    const nestedToolCalls =
      toolCall.nestedToolCalls && toolCall.nestedToolCalls.length > 0
        ? filterToolCallInfoArray(toolCall.nestedToolCalls, state)
        : toolCall.nestedToolCalls;
    filtered.push(
      nestedToolCalls !== toolCall.nestedToolCalls
        ? { ...toolCall, nestedToolCalls }
        : toolCall,
    );
  }
  return filtered;
}

export function filterToolCallsByActiveIds(
  toolCalls: ToolCallInfo[] | undefined,
  activeIds: Set<string>,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (activeIds.size === 0) return [...toolCalls];

  const filtered = toolCalls.filter((toolCall) => !activeIds.has(toolCall.id));
  return filtered.length > 0 ? filtered : undefined;
}

export function filterToolCallsByMatchState(
  toolCalls: ToolCallInfo[] | undefined,
  hiddenState: ToolCallMatchState,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (hiddenState.ids.size === 0 && hiddenState.fingerprintCounts.size === 0) return [...toolCalls];

  const filtered = filterToolCallInfoArray(toolCalls, cloneToolCallMatchState(hiddenState));
  return filtered.length > 0 ? filtered : undefined;
}

export function filterToolCallsByConsumableMatchState(
  toolCalls: ToolCallInfo[] | undefined,
  hiddenState: ToolCallMatchState,
): ToolCallInfo[] | undefined {
  if (!toolCalls || toolCalls.length === 0) return undefined;
  if (hiddenState.ids.size === 0 && hiddenState.fingerprintCounts.size === 0) return [...toolCalls];

  const filtered = filterToolCallInfoArray(toolCalls, hiddenState);
  return filtered.length > 0 ? filtered : undefined;
}

export function mergeToolCallDisplaysWithoutDuplicates(
  primary: ToolCallDisplay[],
  secondary: ToolCallDisplay[],
): ToolCallDisplay[] {
  if (primary.length === 0) return [...secondary];
  if (secondary.length === 0) return [...primary];

  const result = [...primary];
  const primaryState = collectToolCallDisplayMatchState(primary);
  const remainingState = cloneToolCallMatchState(primaryState);

  for (const toolCall of secondary) {
    if (consumeDisplayMatch(toolCall, remainingState)) continue;
    result.push(toolCall);
  }

  return result;
}

export function resolveToolCallInfosForRender(
  source: ToolCallInfoRenderSource,
): ToolCallInfo[] | undefined {
  if (Object.prototype.hasOwnProperty.call(source, "displayToolCalls")) {
    return source.displayToolCalls;
  }
  return source.messageToolCalls;
}

export function buildMessageToolCalls(
  message: Pick<ChatMessage, "toolCalls">,
  toolOutputMap: Record<string, string>,
  toolOutputImageMap: Record<string, ImageAttachment[]> = {},
): ToolCallDisplay[] {
  return (message.toolCalls ?? []).map((toolCall) =>
    buildMessageToolCall(toolCall, toolOutputMap, toolOutputImageMap),
  );
}

export function buildMessageToolCall(
  toolCall: ToolCallInfo,
  toolOutputMap: Record<string, string>,
  toolOutputImageMap: Record<string, ImageAttachment[]> = {},
): ToolCallDisplay {
  const output =
    toolCall.recordedOutput
    ?? toolCall.serverToolOutput
    ?? toolOutputMap[toolCall.id];
  const images = toolOutputImageMap[toolCall.id];
  const displayShape = resolveToolCallDisplayShape(toolCall);

  const display: ToolCallDisplay = {
    id: toolCall.id,
    name: displayShape.name,
    arguments: displayShape.arguments,
    order: toolCall.order,
    status: inferToolCallStatus(toolCall, output),
    output,
    nestedToolCalls: toolCall.nestedToolCalls?.map((nestedToolCall) =>
      buildMessageToolCall(nestedToolCall, toolOutputMap, toolOutputImageMap),
    ),
  };
  if (images && images.length > 0) {
    display.images = images;
  }
  return display;
}

function inferToolCallStatus(
  toolCall: ToolCallInfo,
  output: string | undefined,
): ToolCallDisplay["status"] {
  if (toolCall.outcome) {
    return toolCall.outcome;
  }
  if (output === INTERRUPTED_TOOL_RESULT) {
    return "interrupted";
  }
  return "done";
}

export function mergeSequentialAssistantToolCalls<T extends AssistantToolMergeCandidate>(
  items: T[],
): Array<AssistantToolMergeResult<T>> {
  const merged: Array<AssistantToolMergeResult<T>> = [];
  let pendingToolOnlyItem: T | null = null;
  let pendingToolCalls: ToolCallInfo[] = [];

  const clearPendingToolOnlyItem = () => {
    pendingToolOnlyItem = null;
    pendingToolCalls = [];
  };

  const flushPendingToolOnlyItem = () => {
    if (!pendingToolOnlyItem) return;
    const displayToolCalls = pendingToolCalls.length > 0 ? [...pendingToolCalls] : undefined;
    merged.push({
      ...pendingToolOnlyItem,
      displayToolCalls,
      displayToolCallsBeforeContent: displayToolCalls,
    });
    pendingToolOnlyItem = null;
    pendingToolCalls = [];
  };

  for (const item of items) {
    if (item.renderParts && item.renderParts.length > 0) {
      const hasToolCallsProperty = Object.prototype.hasOwnProperty.call(item, "toolCalls");
      const displayToolCalls = hasToolCallsProperty ? [...(item.toolCalls ?? [])] : undefined;
      flushPendingToolOnlyItem();
      merged.push({
        ...item,
        ...(hasToolCallsProperty ? { displayToolCalls } : {}),
      });
      continue;
    }

    const currentToolCalls = item.toolCalls ?? [];
    const hasResponseText = !item.isKnowledgeProposal && item.content.trim().length > 0;
    const hasThinkingContent = !item.isKnowledgeProposal && !!item.thinkingContent?.trim();
    const isToolOnlyRound =
      !item.isKnowledgeProposal
      && !hasResponseText
      && !hasThinkingContent
      && (item.attachedKnowledgeProposalCount ?? 0) === 0
      && currentToolCalls.length > 0;
    const canAbsorbPendingRounds = !item.isKnowledgeProposal && hasResponseText;

    if (isToolOnlyRound) {
      pendingToolOnlyItem ??= item;
      pendingToolCalls.push(...currentToolCalls);
      continue;
    }

    if (pendingToolCalls.length > 0 && canAbsorbPendingRounds) {
      const beforeContentToolCalls = [...pendingToolCalls];
      const afterContentToolCalls = currentToolCalls.length > 0 ? [...currentToolCalls] : undefined;
      merged.push({
        ...item,
        displayToolCalls: [...beforeContentToolCalls, ...currentToolCalls],
        displayToolCallsBeforeContent: beforeContentToolCalls,
        displayToolCallsAfterContent: afterContentToolCalls,
      });
      clearPendingToolOnlyItem();
      continue;
    }

    flushPendingToolOnlyItem();

    merged.push({
      ...item,
      displayToolCalls: currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
      displayToolCallsBeforeContent:
        !hasResponseText && currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
      displayToolCallsAfterContent:
        hasResponseText && currentToolCalls.length > 0 ? [...currentToolCalls] : undefined,
    });
  }

  flushPendingToolOnlyItem();

  return merged;
}

export function summarizeToolCallBatch(
  toolCalls: ToolCallDisplay[],
  compactEnabled: boolean,
): ToolCallBatchState {
  const total = toolCalls.length;
  let doneCount = 0;
  let runningCount = 0;
  let errorCount = 0;
  let interruptedCount = 0;

  for (const toolCall of toolCalls) {
    switch (toolCall.status) {
      case "done":
        doneCount += 1;
        break;
      case "running":
        runningCount += 1;
        break;
      case "error":
        errorCount += 1;
        break;
      case "interrupted":
        interruptedCount += 1;
        break;
    }
  }

  return {
    total,
    doneCount,
    runningCount,
    errorCount,
    interruptedCount,
    canCollapse:
      compactEnabled
      && total >= 2
      && runningCount === 0,
  };
}
