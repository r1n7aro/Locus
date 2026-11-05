import { vi } from "vitest";
import { installLocusRuntime } from "../../services/locusRuntime";
import {
  applySessionAttentionChange,
  type SessionAttentionEntry,
  type SessionAttentionKind,
  type SessionAttentionSnapshot,
} from "../../services/sessionAttention";

/** Stand-in for the desktop's `session::attention` store: records results,
 * acknowledges reads, and installs a runtime the frontend service talks to. */
export function createSessionAttentionServer(options: {
  /** Answer to `get_session_attention` (default: no results). */
  snapshot?: () => Promise<SessionAttentionSnapshot>;
} = {}) {
  let revision = 0;
  let sequence = 0;
  const sessions = new Map<string, SessionAttentionEntry>();
  const markRead = vi.fn(async (sessionId: string, read: number): Promise<SessionAttentionEntry | null> => {
    const entry = sessions.get(sessionId);
    if (!entry) return null;
    const value = Math.min(read, entry.sequence);
    if (value <= entry.readSequence) return null;
    revision += 1;
    const updated = { ...entry, readSequence: value, revision };
    sessions.set(sessionId, updated);
    return updated;
  });
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    if (command === "get_session_attention") {
      return options.snapshot ? options.snapshot() : { revision: 0, sequence: 0, sessions: {} };
    }
    if (command === "mark_session_attention_read") {
      return markRead(String(args?.sessionId), Number(args?.sequence));
    }
    throw new Error(`unexpected command ${command}`);
  });
  const uninstall = installLocusRuntime({
    kind: "tauri",
    invoke: invoke as never,
    subscribe: async () => () => {},
  });

  /** Records a new result and delivers its change, like the event would. */
  function record(sessionId: string, kind: SessionAttentionKind = "done"): SessionAttentionEntry {
    sequence += 1;
    revision += 1;
    const previous = sessions.get(sessionId);
    const entry: SessionAttentionEntry = {
      sequence,
      readSequence: previous?.readSequence ?? 0,
      kind,
      runId: `run-${sequence}`,
      targetId: null,
      completionSequence: kind === "done" ? sequence : previous?.completionSequence ?? 0,
      revision,
      updatedAt: 0,
    };
    sessions.set(sessionId, entry);
    applySessionAttentionChange({ sessionId, entry });
    return entry;
  }

  return { invoke, markRead, record, uninstall };
}
