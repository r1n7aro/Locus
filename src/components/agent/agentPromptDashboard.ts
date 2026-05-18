import type {
  AgentSystemPromptStats,
  InjectedPromptItem,
  InjectedToolLoadMode,
  RuleItem,
} from "../../types";

const TOOL_SCHEMA_OVERHEAD_TOKENS = 32;

export interface ToolPromptEstimate {
  chars: number;
  tokens: number;
}

export type AgentPromptPartKey = "base" | "env" | "rules" | "knowledge" | "tools";
export type AgentPromptHealthLevel = "healthy" | "watch" | "heavy";

export interface AgentPromptDashboardPart {
  key: AgentPromptPartKey;
  chars: number;
  tokens: number;
  share: number;
}

export interface AgentPromptDashboardHealth {
  score: number;
  level: AgentPromptHealthLevel;
  dominantPartKey: AgentPromptPartKey;
  dominantShare: number;
}

export interface AgentPromptDashboardSummary {
  totalChars: number;
  totalTokens: number;
  parts: AgentPromptDashboardPart[];
  enabledRuleCount: number;
  totalRuleCount: number;
  injectedContextCount: number;
  toolCount: number;
  directToolCount: number;
  lazyToolCount: number;
  skillToolCount: number;
  health: AgentPromptDashboardHealth;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function estimatePromptTokens(chars: number): number {
  return chars > 0 ? Math.ceil(chars / 4) : 0;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function unwrapToolDefinition(meta: unknown): unknown {
  const record = asRecord(meta);
  const functionDef = asRecord(record?.function);
  return functionDef ?? meta ?? {};
}

export function toolMetaLoadMode(meta: unknown): InjectedToolLoadMode {
  const record = asRecord(meta);
  if (record?.loadMode === "lazy") return "lazy";
  if (record?.loadMode === "skill") return "skill";
  return "direct";
}

function serializeToolMeta(meta: unknown): string {
  try {
    const definition = unwrapToolDefinition(meta);
    const wrapped = {
      type: "function",
      function: definition,
    };
    return JSON.stringify(wrapped) ?? "";
  } catch {
    return "";
  }
}

export function estimateToolPrompt(meta: unknown): ToolPromptEstimate {
  const serialized = serializeToolMeta(meta);
  return {
    chars: serialized.length,
    tokens: estimatePromptTokens(serialized.length) + TOOL_SCHEMA_OVERHEAD_TOKENS,
  };
}

function estimateToolPart(items: Array<Pick<InjectedPromptItem, "kind" | "meta">>): AgentPromptDashboardPart {
  const toolItems = items.filter((item) =>
    item.kind === "tools" && toolMetaLoadMode(item.meta) === "direct",
  );
  const summary = toolItems.reduce((sum, item) => {
    const estimate = estimateToolPrompt(item.meta);
    sum.chars += estimate.chars;
    sum.tokens += estimate.tokens;
    return sum;
  }, { chars: 0, tokens: 0 });

  return {
    key: "tools",
    chars: summary.chars,
    tokens: summary.tokens,
    share: 0,
  };
}

export function buildAgentPromptDashboard(
  stats: AgentSystemPromptStats | null,
  ruleItems: Array<Pick<RuleItem, "enabled">>,
  injectedItems: Array<Pick<InjectedPromptItem, "kind" | "meta">>,
): AgentPromptDashboardSummary {
  const parts: AgentPromptDashboardPart[] = [
    {
      key: "base",
      chars: Math.max(0, stats?.baseChars ?? 0),
      tokens: estimatePromptTokens(Math.max(0, stats?.baseChars ?? 0)),
      share: 0,
    },
    {
      key: "env",
      chars: Math.max(0, stats?.envChars ?? 0),
      tokens: estimatePromptTokens(Math.max(0, stats?.envChars ?? 0)),
      share: 0,
    },
    {
      key: "rules",
      chars: Math.max(0, stats?.rulesChars ?? 0),
      tokens: estimatePromptTokens(Math.max(0, stats?.rulesChars ?? 0)),
      share: 0,
    },
    {
      key: "knowledge",
      chars: Math.max(0, stats?.knowledgeChars ?? 0),
      tokens: estimatePromptTokens(Math.max(0, stats?.knowledgeChars ?? 0)),
      share: 0,
    },
    estimateToolPart(injectedItems),
  ];

  const totalChars = parts.reduce((sum, part) => sum + part.chars, 0);
  const totalTokens = parts.reduce((sum, part) => sum + part.tokens, 0);

  for (const part of parts) {
    part.share = totalTokens > 0 ? part.tokens / totalTokens : 0;
  }

  const totalRuleCount = ruleItems.length;
  const enabledRuleCount = ruleItems.filter((rule) => rule.enabled).length;
  const injectedContextCount = injectedItems.filter((item) => item.kind !== "tools").length;
  const toolItems = injectedItems.filter((item) => item.kind === "tools");
  const directToolCount = toolItems.filter((item) => toolMetaLoadMode(item.meta) === "direct").length;
  const lazyToolCount = toolItems.filter((item) => toolMetaLoadMode(item.meta) === "lazy").length;
  const skillToolCount = toolItems.filter((item) => toolMetaLoadMode(item.meta) === "skill").length;
  const toolCount = toolItems.length;
  const dominantPart = parts.reduce((best, part) => (
    part.share > best.share ? part : best
  ), parts[0]!);

  let score = 96;
  if (totalTokens > 32_000) score -= 18;
  else if (totalTokens > 20_000) score -= 12;
  else if (totalTokens > 12_000) score -= 8;
  else if (totalTokens > 6_000) score -= 6;

  if (totalRuleCount > 0 && enabledRuleCount === 0) {
    score -= 14;
  }

  const knowledgePart = parts.find((part) => part.key === "knowledge");
  const envPart = parts.find((part) => part.key === "env");
  const toolsPart = parts.find((part) => part.key === "tools");

  if (knowledgePart && knowledgePart.tokens > 1_400 && knowledgePart.share > 0.32) {
    score -= 10;
  }

  if (knowledgePart && knowledgePart.tokens > 2_200 && knowledgePart.share > 0.55) {
    score -= 14;
  }

  if (envPart && envPart.tokens > 900 && envPart.share > 0.28) {
    score -= 8;
  }

  if (toolsPart && toolsPart.tokens > 4_500 && toolsPart.share > 0.45) {
    score -= 8;
  }

  if (totalTokens > 8_000 && dominantPart.share > 0.62) {
    score -= 6;
  }

  if (totalTokens > 16_000 && dominantPart.share > 0.78) {
    score -= 10;
  }

  if (totalChars === 0) {
    score = 0;
  }

  score = clamp(score, 0, 99);

  let level: AgentPromptHealthLevel = "heavy";
  if (score >= 82) level = "healthy";
  else if (score >= 60) level = "watch";

  return {
    totalChars,
    totalTokens,
    parts,
    enabledRuleCount,
    totalRuleCount,
    injectedContextCount,
    toolCount,
    directToolCount,
    lazyToolCount,
    skillToolCount,
    health: {
      score,
      level,
      dominantPartKey: dominantPart.key,
      dominantShare: dominantPart.share,
    },
  };
}
