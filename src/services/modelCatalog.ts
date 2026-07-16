// Pure helpers that map models.dev catalog entries onto Locus custom
// provider/model configs, plus the picker's search matcher (opencode-style
// normalized + compact substring matching).

import type {
  ApiFormat,
  CustomProvider,
  CustomProviderModel,
  EffortLevel,
  ModelCatalogModel,
  ModelCatalogProvider,
  ReasoningParamFormat,
  ReasoningReplayField,
} from "../types";

export const DEFAULT_CATALOG_CONTEXT_LENGTH = 256_000;

export const DEFAULT_REASONING_EFFORTS: EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];

/** Format the reasoning param takes when a model row leaves it unset (null). */
export function defaultReasoningParamFormat(apiFormat: ApiFormat): ReasoningParamFormat {
  switch (apiFormat) {
    case "openai_responses": return "openai_responses_reasoning_effort";
    case "anthropic_messages": return "anthropic_thinking";
    default: return "openai_chat_reasoning_effort";
  }
}

export function newCustomProviderModel(): CustomProviderModel {
  return {
    id: "",
    apiModel: "",
    name: "",
    contextLength: DEFAULT_CATALOG_CONTEXT_LENGTH,
    betaFlags: [],
    supportedReasoningEfforts: [...DEFAULT_REASONING_EFFORTS],
    reasoningParamFormat: null,
    replayReasoningContent: null,
    reasoningReplayField: null,
    serverTools: { webSearch: false },
    supportsVision: true,
  };
}

export function newCustomProvider(): CustomProvider {
  return {
    id: crypto.randomUUID(),
    name: "",
    endpoint: "",
    apiFormat: "openai_chat",
    apiKey: "",
    catalogId: null,
    models: [newCustomProviderModel()],
  };
}

/** Providers whose models.dev entry has no `api` because the official SDK is
 *  implied; Locus talks plain HTTP, so map them to their public REST base
 *  (each vendor's documented OpenAI-compatible endpoint). */
const OFFICIAL_API_FALLBACKS: Record<string, string> = {
  anthropic: "https://api.anthropic.com",
  openai: "https://api.openai.com/v1",
  xai: "https://api.x.ai/v1",
  google: "https://generativelanguage.googleapis.com/v1beta/openai",
  mistral: "https://api.mistral.ai/v1",
  groq: "https://api.groq.com/openai/v1",
  togetherai: "https://api.together.xyz/v1",
  deepinfra: "https://api.deepinfra.com/v1/openai",
  cerebras: "https://api.cerebras.ai/v1",
  perplexity: "https://api.perplexity.ai",
  cohere: "https://api.cohere.ai/compatibility/v1",
  v0: "https://api.v0.dev/v1",
  vercel: "https://ai-gateway.vercel.sh/v1",
};

/** npm SDK ids Locus can talk to directly over HTTP — either natively
 *  OpenAI-compatible or covered by an OFFICIAL_API_FALLBACKS compat base. */
const OPENAI_COMPATIBLE_NPM = new Set([
  "@ai-sdk/openai-compatible",
  "@ai-sdk/openai",
  "@openrouter/ai-sdk-provider",
  "@ai-sdk/xai",
  "@ai-sdk/mistral",
  "@ai-sdk/groq",
  "@ai-sdk/togetherai",
  "@ai-sdk/deepinfra",
  "@ai-sdk/cerebras",
  "@ai-sdk/google",
  "@ai-sdk/perplexity",
  "@ai-sdk/cohere",
  "@ai-sdk/vercel",
  "@ai-sdk/gateway",
]);

export function catalogApiFormat(provider: ModelCatalogProvider): ApiFormat | null {
  const npm = provider.npm ?? "@ai-sdk/openai-compatible";
  if (npm === "@ai-sdk/anthropic") return "anthropic_messages";
  if (OPENAI_COMPATIBLE_NPM.has(npm)) return "openai_chat";
  return null;
}

export function catalogProviderEndpoint(
  providerId: string,
  provider: ModelCatalogProvider,
): string {
  return provider.api ?? OFFICIAL_API_FALLBACKS[providerId] ?? "";
}

/**
 * Official Anthropic-compatible bases for providers whose models.dev entry is
 * OpenAI-compatible. Custom providers work best on Anthropic Messages (native
 * lazy tool loading; deferred tool calls), so catalog adds prefer these.
 * Every URL is taken from the vendor's own Anthropic-API/Claude-Code docs and
 * follows the same "…/v1" shape as catalog endpoints (client appends
 * /messages). OpenAI itself and aggregators stay on their native protocol;
 * Alibaba is deliberately absent (no fixed intl endpoint, and switching would
 * drop the DashScope enable_thinking adaptation).
 */
export const CATALOG_ANTHROPIC_BASES: Record<string, string> = {
  deepseek: "https://api.deepseek.com/anthropic/v1",
  zhipuai: "https://open.bigmodel.cn/api/anthropic/v1",
  zai: "https://api.z.ai/api/anthropic/v1",
  moonshotai: "https://api.moonshot.ai/anthropic/v1",
  "moonshotai-cn": "https://api.moonshot.cn/anthropic/v1",
  xiaomi: "https://api.xiaomimimo.com/anthropic/v1",
  "tencent-tokenhub": "https://tokenhub.tencentmaas.com/v1",
};

/** Connection a catalog add should default to: the vendor's Anthropic base
 *  when one is known, otherwise whatever the catalog entry says. */
export function catalogPreferredConnection(
  providerId: string,
  provider: ModelCatalogProvider,
): { endpoint: string; apiFormat: ApiFormat | null } {
  const anthropicBase = CATALOG_ANTHROPIC_BASES[providerId];
  if (anthropicBase) return { endpoint: anthropicBase, apiFormat: "anthropic_messages" };
  return {
    endpoint: catalogProviderEndpoint(providerId, provider),
    apiFormat: catalogApiFormat(provider),
  };
}

/** A provider is directly connectable when we know both its protocol and URL.
 *  Endpoints with `${VAR}` templates (per-account URLs) don't count — they
 *  need substitution Locus can't do. */
export function isCatalogProviderConnectable(
  providerId: string,
  provider: ModelCatalogProvider,
): boolean {
  if (catalogApiFormat(provider) === null) return false;
  const endpoint = catalogProviderEndpoint(providerId, provider);
  return endpoint !== "" && !endpoint.includes("${");
}

/** What the pickers list: trusted (no relay/reseller gateways, see
 *  TRUSTED_CATALOG_PROVIDER_IDS) and reachable with just endpoint + key.
 *  Official platforms that need their own auth stack (Vertex/Bedrock/Azure…)
 *  stay hidden until models.dev gives them a directly usable endpoint. */
export function isListableCatalogProvider(
  providerId: string,
  provider: ModelCatalogProvider,
): boolean {
  return (
    isTrustedCatalogProvider(providerId) &&
    isCatalogProviderConnectable(providerId, provider)
  );
}

const EFFORT_VALUES: EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];

function effortsFromCatalog(model: ModelCatalogModel): EffortLevel[] {
  const effortOption = model.reasoning_options?.find((o) => o.type === "effort");
  if (!effortOption?.values) return [];
  return effortOption.values.filter(
    (v): v is EffortLevel => typeof v === "string" && EFFORT_VALUES.includes(v as EffortLevel),
  );
}

export function catalogReasoningParamFormat(
  providerId: string,
  model: ModelCatalogModel,
  apiFormat: ApiFormat,
): ReasoningParamFormat {
  if (!model.reasoning) return "none";
  if (apiFormat === "anthropic_messages") return "anthropic_thinking";
  if (apiFormat === "openai_responses") return "openai_responses_reasoning_effort";

  const options = model.reasoning_options ?? [];
  const hasEffort = options.some((o) => o.type === "effort");
  if (hasEffort || options.length === 0) return "openai_chat_reasoning_effort";
  // toggle / budget_tokens without an effort axis: pick the body switch the
  // provider family expects (DashScope wants enable_thinking, GLM thinking.type).
  if (providerId.startsWith("alibaba")) return "openai_chat_enable_thinking";
  if (providerId.startsWith("zhipu")) return "openai_chat_thinking_type";
  return "openai_chat_enable_thinking";
}

export function catalogReplayField(model: ModelCatalogModel): ReasoningReplayField | null {
  const interleaved = model.interleaved;
  if (interleaved && typeof interleaved === "object" && interleaved.field) {
    return interleaved.field;
  }
  return null;
}

export function modelRowIdFromApiModel(apiModel: string): string {
  const slug = apiModel.trim().replace(/[/\s]+/g, "-");
  return slug || "model";
}

export function catalogModelSupportsVision(model: ModelCatalogModel): boolean {
  return model.attachment === true || (model.modalities?.input ?? []).includes("image");
}

/** Build a provider-model config row from a catalog entry. All fields stay
 *  editable in the form afterwards. */
export function catalogModelToProviderModel(
  providerId: string,
  catalogModelId: string,
  model: ModelCatalogModel,
  apiFormat: ApiFormat,
): CustomProviderModel {
  const reasoningParamFormat = catalogReasoningParamFormat(providerId, model, apiFormat);
  const replayField = catalogReplayField(model);
  const efforts = effortsFromCatalog(model);
  return {
    id: modelRowIdFromApiModel(catalogModelId),
    apiModel: catalogModelId,
    name: model.name || catalogModelId,
    contextLength: model.limit.context > 0 ? model.limit.context : DEFAULT_CATALOG_CONTEXT_LENGTH,
    betaFlags: [],
    supportedReasoningEfforts: efforts,
    reasoningParamFormat,
    replayReasoningContent:
      apiFormat === "openai_chat" && replayField !== null ? true : undefined,
    reasoningReplayField: replayField,
    serverTools: { webSearch: false },
    supportsVision: catalogModelSupportsVision(model),
    catalogModelId,
  };
}

export function catalogProviderToCustomProvider(
  providerId: string,
  provider: ModelCatalogProvider,
): CustomProvider {
  const connection = catalogPreferredConnection(providerId, provider);
  return {
    id: `${providerId}-${crypto.randomUUID().slice(0, 8)}`,
    name: provider.name,
    endpoint: connection.endpoint,
    apiFormat: connection.apiFormat ?? "openai_chat",
    apiKey: "",
    catalogId: providerId,
    models: [],
  };
}

export function catalogKeyPlaceholder(provider: ModelCatalogProvider): string | null {
  return provider.env?.[0] ?? null;
}

/** Apply a catalog pick onto a draft provider: blank provider-level fields are
 *  filled from the catalog (never clobbering user edits), the picked model
 *  replaces the first blank row or appends. Returns false on duplicates. */
export function applyCatalogSelection(
  draft: CustomProvider,
  providerId: string,
  catalogProvider: ModelCatalogProvider,
  modelId: string,
  model: ModelCatalogModel,
): boolean {
  const apiFormat = catalogApiFormat(catalogProvider) ?? draft.apiFormat;
  if (!draft.name.trim()) draft.name = catalogProvider.name;
  if (!draft.endpoint.trim()) {
    draft.endpoint = catalogProviderEndpoint(providerId, catalogProvider);
  }
  if (draft.models.length === 0 || draft.models.every((m) => !m.apiModel.trim())) {
    draft.apiFormat = apiFormat;
  }
  if (!draft.catalogId) draft.catalogId = providerId;

  const row = catalogModelToProviderModel(providerId, modelId, model, draft.apiFormat);
  const blankIdx = draft.models.findIndex((m) => !m.apiModel.trim());
  if (blankIdx >= 0) {
    draft.models.splice(blankIdx, 1, row);
    return true;
  }
  if (draft.models.some((m) => m.apiModel === row.apiModel)) return false;
  draft.models.push(row);
  return true;
}

/** True when a configured row corresponds to the given catalog model id. */
export function providerModelMatchesCatalog(
  row: CustomProviderModel,
  catalogModelId: string,
): boolean {
  return row.catalogModelId === catalogModelId || row.apiModel === catalogModelId;
}

/** Append a catalog model as a configured row. Unlike applyCatalogSelection
 *  this never touches provider-level fields (endpoint/name/format/catalogId):
 *  the provider identity is decided before models are picked. */
export function addCatalogModelRow(
  draft: CustomProvider,
  providerId: string,
  modelId: string,
  model: ModelCatalogModel,
): boolean {
  if (draft.models.some((row) => providerModelMatchesCatalog(row, modelId))) return false;
  draft.models.push(catalogModelToProviderModel(providerId, modelId, model, draft.apiFormat));
  return true;
}

export function formatContextLength(context: number): string {
  if (context >= 1_000_000) {
    const m = context / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`;
  }
  if (context >= 1_000) return `${Math.round(context / 1_000)}k`;
  return `${context}`;
}

// ── Search (opencode-style) ──────────────────────────────────────────────

export function normalizeModelSearch(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^\p{Letter}\p{Number}]+/gu, " ")
    .trim()
    .replace(/\s+/g, " ");
}

export function compactModelSearch(value: string): string {
  return normalizeModelSearch(value).split(" ").join("");
}

export function matchesModelSearch(query: string, values: string[]): boolean {
  const search = normalizeModelSearch(query);
  if (!search) return true;
  const compact = compactModelSearch(query);
  return values.some(
    (value) =>
      normalizeModelSearch(value).includes(search) ||
      compactModelSearch(value).includes(compact),
  );
}

/** True when the wire id is just a case/punctuation variant of the display
 *  name (GLM-5.2 vs glm-5.2): rendering both next to each other is noise. */
export function isRedundantModelId(name: string, apiModel: string): boolean {
  if (!name.trim() || !apiModel.trim()) return true;
  return compactModelSearch(name) === compactModelSearch(apiModel);
}

/** Model pickers only offer current agent-capable models: tool calling plus a
 *  release cutoff (missing dates read as too old). Anything older can still
 *  be configured via manual add. */
export const CATALOG_MODEL_MIN_RELEASE_DATE = "2026-01-01";

export function isListableCatalogModel(model: ModelCatalogModel): boolean {
  if (model.status === "deprecated") return false;
  if (!model.tool_call) return false;
  return (model.release_date ?? "") >= CATALOG_MODEL_MIN_RELEASE_DATE;
}

/** Providers pinned to the top of pickers (中文用户常用厂商优先). */
export const FEATURED_CATALOG_PROVIDERS = [
  "deepseek",
  "zhipuai",
  "moonshotai",
  "minimax",
  "alibaba",
  "xiaomi",
  // Hunyuan's only first-party catalog entries are Tencent's plan/platform
  // variants; TokenHub is the general pay-per-token one (hy3, hy3-preview).
  "tencent-tokenhub",
  "siliconflow",
  "openrouter",
  "openai",
  "anthropic",
];

/**
 * Catalog allowlist: first-party model creators (incl. regional/plan
 * variants), official clouds, and major inference/aggregator brands.
 * models.dev also carries dozens of small relay/reseller gateways (中转站,
 * recognizable by hawking other vendors' proprietary models) — those stay out
 * of the pickers; anyone who really wants one can still configure it
 * manually. Ids not on the list default to hidden, so catalog refreshes fail
 * closed against new relays.
 */
export const TRUSTED_CATALOG_PROVIDER_IDS: ReadonlySet<string> = new Set([
  // Model creators — first-party endpoints and their regional/plan variants.
  "anthropic",
  "openai",
  "google",
  "xai",
  "mistral",
  "cohere",
  "meta",
  "llama",
  "deepseek",
  "zhipuai",
  "zhipuai-coding-plan",
  "zai",
  "zai-coding-plan",
  "moonshotai",
  "moonshotai-cn",
  "kimi-for-coding",
  "minimax",
  "minimax-cn",
  "minimax-coding-plan",
  "minimax-cn-coding-plan",
  "alibaba",
  "alibaba-cn",
  "alibaba-coding-plan",
  "alibaba-coding-plan-cn",
  "alibaba-token-plan",
  "alibaba-token-plan-cn",
  "stepfun",
  "stepfun-ai",
  "stepfun-ai-step-plan",
  "stepfun-step-plan",
  "tencent-coding-plan",
  "tencent-token-plan",
  "tencent-tokenhub",
  "xiaomi",
  "xiaomi-token-plan-ams",
  "xiaomi-token-plan-cn",
  "xiaomi-token-plan-sgp",
  "longcat",
  "bailing",
  "iflowcn",
  "modelscope",
  "perplexity",
  "perplexity-agent",
  "upstage",
  "sarvam",
  "inception",
  "morph",
  "sakana",
  "poolside",
  "nova",
  "v0",
  "ollama-cloud",
  "lmstudio",
  // Official clouds and platforms (incl. licensed closed-model resale).
  "azure",
  "azure-cognitive-services",
  "amazon-bedrock",
  "google-vertex",
  "google-vertex-anthropic",
  "cloudflare-workers-ai",
  "databricks",
  "snowflake-cortex",
  "sap-ai-core",
  "gitlab",
  "github-models",
  "nvidia",
  "huggingface",
  "digitalocean",
  "ovhcloud",
  "scaleway",
  "vultr",
  "wandb",
  "nebius",
  // Major aggregators and open-model inference brands.
  "openrouter",
  "vercel",
  "togetherai",
  "fireworks-ai",
  "groq",
  "deepinfra",
  "cerebras",
  "siliconflow",
  "siliconflow-cn",
  "novita-ai",
  "baseten",
  "friendli",
]);

export function isTrustedCatalogProvider(providerId: string): boolean {
  return TRUSTED_CATALOG_PROVIDER_IDS.has(providerId);
}
