import { reactive, readonly, watch } from "vue";
import type { AgentInfo } from "../types";
import type { NewSessionAgentMode } from "../stores/newSessionSelection";
import { LEGACY_STORAGE_KEYS, readLegacyNewSessionAgentId } from "./settingsLegacy";
import { saveSettings, settingValue } from "./useSettingsStore";

const AGENT_MODE_KEY = "agent.newSession.agentMode";
const DEFAULT_AGENT_ID_KEY = "agent.newSession.defaultAgentId";

interface NewSessionSettings {
  agentMode: NewSessionAgentMode;
  /** The Agent used by "fixed" mode. */
  defaultAgentId: string;
  /** A pre-migration global default; seeds projects without remembered choices. */
  legacyAgentId: string;
}

function trimmedString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

// Migration state of older builds, not a setting: read once.
let legacyAgentId = readLegacyNewSessionAgentId();

function project(): NewSessionSettings {
  const defaultAgentId = trimmedString(settingValue(DEFAULT_AGENT_ID_KEY));
  const mode = settingValue(AGENT_MODE_KEY);
  const agentMode: NewSessionAgentMode = mode === "workspace" || mode === "fixed" ? mode : "remember";
  return {
    agentMode: agentMode === "fixed" && !defaultAgentId ? "remember" : agentMode,
    defaultAgentId,
    legacyAgentId,
  };
}

const state = reactive(project());

watch(project, (next) => Object.assign(state, next), { flush: "sync" });

function persist(agentMode: NewSessionAgentMode, defaultAgentId: string) {
  saveSettings([
    { key: AGENT_MODE_KEY, value: agentMode },
    { key: DEFAULT_AGENT_ID_KEY, value: defaultAgentId },
  ]);
}

/** Plain catalog default: the preferred Agent when available, else the workspace default. */
export function resolveDefaultAgentId(
  agents: readonly Pick<AgentInfo, "id" | "isDefault">[],
  preferredId = "",
): string {
  return agents.find((agent) => agent.id === preferredId)?.id
    ?? agents.find((agent) => agent.isDefault)?.id
    ?? agents[0]?.id
    ?? "";
}

export function useNewSessionSettings() {
  return {
    state: readonly(state),
    setAgentMode(mode: NewSessionAgentMode) {
      persist(mode === "fixed" && !state.defaultAgentId ? "remember" : mode, state.defaultAgentId);
    },
    /** Pins an Agent; an empty id follows the workspace default. */
    setDefaultAgentId(id: string) {
      const agentId = id.trim();
      persist(agentId ? "fixed" : "workspace", agentId);
    },
    reset() {
      legacyAgentId = "";
      state.legacyAgentId = "";
      try { localStorage.removeItem(LEGACY_STORAGE_KEYS.newSession); } catch { /* optional */ }
      persist("remember", "");
    },
  };
}
