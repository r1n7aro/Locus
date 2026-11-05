import { reactive } from "vue";
import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";

/** A native subagent's card changed (`claude_native_agent_hub.rs`). */
export const CLAUDE_AGENT_CARD_EVENT = "claude-agent-card";

/** Session type of a native subagent's record: Claude Code runs the agent,
 *  so its child session is read-only (the backend refuses input there). */
export const NATIVE_AGENT_SESSION_TYPE = "claude_agent";

export function isNativeAgentRecordType(sessionType: string | null | undefined): boolean {
  return sessionType === NATIVE_AGENT_SESSION_TYPE;
}

/** What a native subagent's Agent card shows beyond its call. */
export interface ClaudeAgentCard {
  /** The model Claude resolved for the subagent. */
  model?: string;
  /** The subagent's read-only record (a child session of the conversation). */
  recordSessionId?: string;
}

export interface ClaudeAgentCardEvent extends ClaudeAgentCard {
  sessionId: string;
  /** The Agent card: the call that started the subagent. */
  toolCallId: string;
}

// Session → Agent card id → its model and record. Nothing loads or listens
// until a native Agent card renders; each session then loads its stored
// cards once and follows live changes.
const cards = reactive(new Map<string, Map<string, ClaudeAgentCard>>());
const loaded = new Set<string>();
const pending = new Set<string>();
let subscription: Promise<void> | null = null;

function clean(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

/** Merges a card's known fields; a stored value never replaces a live one. */
function remember(sessionId: string, toolCallId: string, card: ClaudeAgentCard, live: boolean) {
  const model = clean(card.model);
  const recordSessionId = clean(card.recordSessionId);
  if (!sessionId || !toolCallId || (!model && !recordSessionId)) return;
  if (!cards.has(sessionId)) cards.set(sessionId, new Map());
  const sessionCards = cards.get(sessionId)!;
  const known = sessionCards.get(toolCallId) ?? {};
  sessionCards.set(toolCallId, {
    model: (live ? model ?? known.model : known.model ?? model),
    recordSessionId: (live ? recordSessionId ?? known.recordSessionId : known.recordSessionId ?? recordSessionId),
  });
}

// Listening first: a change stored after the load read it still arrives.
function ensureSubscription(): Promise<void> {
  subscription ??= getLocusRuntime()
    .subscribe<ClaudeAgentCardEvent>(CLAUDE_AGENT_CARD_EVENT, (event) => {
      remember(event.sessionId, event.toolCallId, event, true);
    })
    .then(() => {})
    .catch(() => {
      // The next card that renders subscribes again.
      subscription = null;
    });
  return subscription;
}

function load(sessionId: string) {
  if (loaded.has(sessionId) || pending.has(sessionId)) return;
  pending.add(sessionId);
  void ensureSubscription()
    .then(() => ipcInvoke<Record<string, ClaudeAgentCard> | null>("get_claude_agent_cards", { sessionId }))
    .then((stored) => {
      loaded.add(sessionId);
      for (const [card, info] of Object.entries(stored ?? {})) remember(sessionId, card, info, false);
    })
    // A failed query is asked again by the next card that renders.
    .catch(() => {})
    .finally(() => { pending.delete(sessionId); });
}

/** What the native subagent of an Agent card ran on and where its record
 *  is; `null` until known. Reading it from a computed re-evaluates once it
 *  loads or changes. */
export function claudeAgentCard(sessionId: string | null | undefined, toolCallId: string): ClaudeAgentCard | null {
  const id = sessionId?.trim();
  if (!id || !toolCallId) return null;
  const card = cards.get(id)?.get(toolCallId) ?? null;
  if (!loaded.has(id)) load(id);
  return card;
}
