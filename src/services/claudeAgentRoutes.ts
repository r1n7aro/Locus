import { reactive, type InjectionKey, type Ref } from "vue";
import { ipcInvoke } from "./ipc";

/** Where one published Agent type of a Claude Code conversation runs. */
export interface ClaudeAgentRoute {
  /** The `subagent_type` Claude calls, `locus:<name>`. */
  subagentType: string;
  /** The Locus agent definition it runs. */
  agentType: string;
  backend: "claudeNative" | "locus";
  /** Full model id the type is bound to, and its short label. */
  model: string;
  modelLabel: string;
}

export interface ClaudeAgentRoutes {
  types: ClaudeAgentRoute[];
  /** Offered Claude Code builtin types; Claude Code picks their model. */
  builtins: string[];
}

/** The session whose tool cards render below (provided by ChatTranscript). */
export const AGENT_ROUTES_SESSION_KEY: InjectionKey<Readonly<Ref<string | null>>> = Symbol("agentRoutesSession");

// A conversation pins its directory before its first Agent call and never
// changes it, so one answer per session is kept, including "none".
const loaded = reactive(new Map<string, ClaudeAgentRoutes | null>());
const pending = new Set<string>();

/** The session's pinned Agent routes; `null` until loaded or without a
 *  native route. Reading it from a computed re-evaluates once it loads. */
export function claudeAgentRoutes(sessionId: string | null | undefined): ClaudeAgentRoutes | null {
  const id = sessionId?.trim();
  if (!id) return null;
  if (loaded.has(id)) return loaded.get(id) ?? null;
  if (!pending.has(id)) {
    pending.add(id);
    ipcInvoke<ClaudeAgentRoutes | null>("get_claude_agent_routes", { sessionId: id })
      .then((routes) => { loaded.set(id, routes ?? null); })
      // A failed query is asked again by the next card that renders.
      .catch(() => {})
      .finally(() => { pending.delete(id); });
  }
  return null;
}
