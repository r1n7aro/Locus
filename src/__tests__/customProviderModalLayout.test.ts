import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CATALOG_ANTHROPIC_BASES,
  FEATURED_CATALOG_PROVIDERS,
  addCatalogModelRow,
  catalogProviderEndpoint,
  catalogProviderToCustomProvider,
  defaultReasoningParamFormat,
  isListableCatalogModel,
  isListableCatalogProvider,
  isRedundantModelId,
  isTrustedCatalogProvider,
  newCustomProvider,
  newCustomProviderModel,
  providerModelMatchesCatalog,
} from "../services/modelCatalog";
import type { CustomProvider, ModelCatalogModel, ModelCatalogProvider } from "../types";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

const catalogModel: ModelCatalogModel = {
  name: "DeepSeek Chat",
  limit: { context: 128_000, output: 8_192 },
  reasoning: false,
  tool_call: true,
};

function draftProvider(): CustomProvider {
  return {
    id: "p1",
    name: "DeepSeek",
    endpoint: "https://api.deepseek.com",
    apiFormat: "openai_chat",
    apiKey: "",
    catalogId: "deepseek",
    models: [],
  };
}

describe("custom provider modal helpers", () => {
  it("addCatalogModelRow appends a row without touching provider fields", () => {
    const draft = draftProvider();
    draft.endpoint = "https://my-gateway.example/v1";

    expect(addCatalogModelRow(draft, "deepseek", "deepseek-chat", catalogModel)).toBe(true);

    expect(draft.endpoint).toBe("https://my-gateway.example/v1");
    expect(draft.name).toBe("DeepSeek");
    expect(draft.catalogId).toBe("deepseek");
    expect(draft.models).toHaveLength(1);
    expect(draft.models[0].apiModel).toBe("deepseek-chat");
    expect(draft.models[0].contextLength).toBe(128_000);
  });

  it("addCatalogModelRow rejects duplicates by catalog id or api model", () => {
    const draft = draftProvider();
    addCatalogModelRow(draft, "deepseek", "deepseek-chat", catalogModel);

    expect(addCatalogModelRow(draft, "deepseek", "deepseek-chat", catalogModel)).toBe(false);
    expect(draft.models).toHaveLength(1);
  });

  it("providerModelMatchesCatalog matches by row catalog id and api model", () => {
    const row = { ...newCustomProviderModel(), apiModel: "deepseek-chat" };

    expect(providerModelMatchesCatalog(row, "deepseek-chat")).toBe(true);
    expect(providerModelMatchesCatalog(row, "deepseek-reasoner")).toBe(false);
    expect(
      providerModelMatchesCatalog(
        { ...newCustomProviderModel(), apiModel: "custom-alias", catalogModelId: "deepseek-chat" },
        "deepseek-chat",
      ),
    ).toBe(true);
  });

  it("newCustomProvider starts with one blank editable model row", () => {
    const draft = newCustomProvider();

    expect(draft.apiFormat).toBe("openai_chat");
    expect(draft.catalogId).toBeNull();
    expect(draft.models).toHaveLength(1);
    expect(draft.models[0].apiModel).toBe("");
    expect(draft.models[0].contextLength).toBe(256_000);
    expect(draft.models[0].supportedReasoningEfforts).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("derives the reasoning param format per wire format", () => {
    expect(defaultReasoningParamFormat("openai_chat")).toBe("openai_chat_reasoning_effort");
    expect(defaultReasoningParamFormat("openai_responses")).toBe("openai_responses_reasoning_effort");
    expect(defaultReasoningParamFormat("anthropic_messages")).toBe("anthropic_thinking");
  });

  it("defaults catalog adds to the vendor's Anthropic endpoint when it has one", () => {
    // Anthropic Messages is the preferred wire for custom providers (native
    // lazy tool loading), so vendors with an official Anthropic base get it.
    const deepseek = catalogProviderToCustomProvider("deepseek", {
      name: "DeepSeek",
      api: "https://api.deepseek.com",
      models: {},
    });
    expect(deepseek.apiFormat).toBe("anthropic_messages");
    expect(deepseek.endpoint).toBe("https://api.deepseek.com/anthropic/v1");

    // Vendors without a known Anthropic base keep their catalog connection.
    const openai = catalogProviderToCustomProvider("openai", {
      name: "OpenAI",
      api: "https://api.openai.com/v1",
      models: {},
    });
    expect(openai.apiFormat).toBe("openai_chat");
    expect(openai.endpoint).toBe("https://api.openai.com/v1");

    // Overrides ride on trusted ids and end in /v1 — stream_chat_native
    // appends /messages, same shape as catalog endpoints (…/anthropic/v1).
    for (const [id, base] of Object.entries(CATALOG_ANTHROPIC_BASES)) {
      expect(isTrustedCatalogProvider(id), `override ${id}`).toBe(true);
      expect(base.endsWith("/v1"), `override base ${base}`).toBe(true);
    }
  });
});

describe("custom provider modal layout", () => {
  it("runs the add flow as a two-step wizard with in-dialog validation", () => {
    const source = read("src/components/settings/CustomProviderModal.vue");

    expect(source).toContain('import ProviderCatalogStep from "./ProviderCatalogStep.vue"');
    expect(source).toContain('import ProviderModelsEditor from "./ProviderModelsEditor.vue"');
    expect(source).toContain('stage.value = props.isAdding ? "pick" : "config"');
    expect(source).toContain("catalogProviderToCustomProvider");
    expect(source).toContain("pickManualProvider");
    expect(source).toContain("backToPick");
    expect(source).toContain('emit("openCatalog")');
    // Validation errors surface inside the dialog, not on the page behind it,
    // and point at the offending field instead of toggling sections open.
    expect(source).toContain("localError");
    expect(source).toContain("invalidField");
    expect(source).toContain('t("settings.custom.nameRequired")');
    expect(source).toContain('t("settings.custom.endpointRequired")');
    expect(source).toContain('t("settings.custom.apiModelRequired")');
    // The shell no longer embeds the cross-provider picker directly.
    expect(source).not.toContain("ModelCatalogPicker");
    expect(source).not.toContain("applyCatalogSelection");
  });

  it("keeps connection settings in a fixed sidebar the models pane cannot crush", () => {
    const source = read("src/components/settings/CustomProviderModal.vue");

    // Two-pane config stage: connection fields live in their own always-visible
    // column; the models editor scrolls independently beside them.
    expect(source).toContain('class="config-side"');
    expect(source).toContain('class="config-main"');
    // The sidebar is a flat list of fields: no group headings, no hint text
    // crowding the labels. "Leave blank to keep the saved key" lives in the
    // key input's placeholder (edit flow) instead of next to the label.
    expect(source).not.toContain("side-group");
    expect(source).not.toContain("config-hint");
    expect(source).not.toContain("config-note");
    expect(source).not.toContain('t("settings.custom.connectionSettings")');
    expect(source).toContain('t("settings.custom.apiKeyKeepHint")');
    expect(source).toContain(':placeholder="keyPlaceholder"');
    // The old collapsible connection block (which a growing models editor
    // flex-squeezed to 2px, "disappearing" it) must stay gone.
    expect(source).not.toContain("connectionOpen");
    expect(source).not.toContain("connection-toggle");
    // Dropdowns are the themed BaseDropdown, never a native <select> whose
    // popup ignores the app theme.
    expect(source).toContain("BaseDropdown");
    expect(source).not.toContain("<select");
    // Sidebar keeps a fixed width and never flexes away.
    expect(source).toMatch(/\.config-side\s*\{[^}]*flex-shrink:\s*0/);
    // The endpoint edits in an auto-growing textarea so long URLs are fully
    // visible; whitespace (incl. pasted newlines) never reaches the draft.
    expect(source).toContain("endpoint-input");
    expect(source).toContain("field-sizing: content");
    expect(source).toContain('el.value.replace(/\\s+/g, "")');
    expect(source).toContain("@keydown.enter.prevent");
  });

  it("keeps provider choice and model membership separated in the editor", () => {
    const source = read("src/components/settings/ProviderModelsEditor.vue");

    expect(source).toContain("addCatalogModelRow");
    expect(source).not.toContain("applyCatalogSelection");
    expect(source).toContain("providerModelMatchesCatalog");
    expect(source).toContain("catalogProviderId");
    // The cross-provider browse panel is a manual-provider fallback only.
    expect(source).toContain('v-if="!hasCatalog"');
    expect(source).toContain("ModelCatalogPicker");
    // Rows get stable ids before a test run so results target the right row.
    expect(source).toContain("handleTestClick");
    expect(source).toContain("modelRowIdFromApiModel");
  });

  it("scrolls configured and available models as one list under a fixed header", () => {
    const source = read("src/components/settings/ProviderModelsEditor.vue");

    // Single scroll container: expanding a card grows the flow instead of
    // crushing sibling sections.
    expect(source).toContain('class="models-scroll"');
    // Filtering dims configured rows rather than hiding them (hiding reads as
    // data loss), and only hides rows in the available catalog below.
    expect(source).toContain("rowMatchesFilter");
    expect(source).toContain("dimmed");
    // A fresh blank row opens itself so its fields are visible immediately.
    expect(source).toContain("!first.apiModel.trim()");
    // Failed "no model id" validation marks the culprits in place.
    expect(source).toContain("highlightMissing");
  });

  it("offers manual setup beside the catalog on the pick step", () => {
    const source = read("src/components/settings/ProviderCatalogStep.vue");

    expect(source).toContain("pickManual");
    expect(source).toContain("pickCatalog");
    expect(source).toContain("FEATURED_CATALOG_PROVIDERS");
    // The provider search must also match model names (e.g. "kimi" → Moonshot).
    expect(source).toContain("parts.push(modelId, model.name)");
  });

  it("keeps relay/reseller gateways (中转站) out of the catalog pickers", () => {
    // Every featured provider must survive the allowlist.
    for (const id of FEATURED_CATALOG_PROVIDERS) {
      expect(isTrustedCatalogProvider(id), `featured ${id}`).toBe(true);
    }
    // First-party creators, official clouds, and openrouter-class brands stay.
    for (const id of [
      "openrouter",
      "vercel",
      "togetherai",
      "groq",
      "moonshotai-cn",
      "zai-coding-plan",
      "amazon-bedrock",
      "github-models",
    ]) {
      expect(isTrustedCatalogProvider(id), `trusted ${id}`).toBe(true);
    }
    // Relay/reseller gateways never surface, searched or not.
    for (const id of [
      "302ai",
      "aihubmix",
      "jiekou",
      "nano-gpt",
      "zenmux",
      "kilo",
      "llmgateway",
      "poe",
      "qihang-ai",
      "anyapi",
      "fastrouter",
    ]) {
      expect(isTrustedCatalogProvider(id), `relay ${id}`).toBe(false);
    }

    const step = read("src/components/settings/ProviderCatalogStep.vue");
    expect(step).toContain("isListableCatalogProvider");

    // Open browsing filters too, but an explicit provider restriction (editing
    // an existing provider) bypasses the allowlist so its models stay usable.
    const picker = read("src/components/settings/ModelCatalogPicker.vue");
    expect(picker).toContain("isListableCatalogProvider");
    expect(picker).toContain("props.restrictProviderId\n    ? [props.restrictProviderId].filter((id) => providers[id])");
  });

  it("hides providers Locus cannot reach with just endpoint + key", () => {
    const provider = (extra: Partial<ModelCatalogProvider>): ModelCatalogProvider => ({
      name: "P",
      models: {},
      ...extra,
    });

    // Trusted + plain endpoint → listed.
    expect(isListableCatalogProvider("deepseek", provider({ api: "https://api.deepseek.com" }))).toBe(true);
    // Official platforms needing their own auth stack stay trusted but hidden.
    expect(isListableCatalogProvider("google-vertex", provider({ npm: "@ai-sdk/google-vertex" }))).toBe(false);
    expect(isListableCatalogProvider("amazon-bedrock", provider({ npm: "@ai-sdk/amazon-bedrock" }))).toBe(false);
    // Per-account `${VAR}` template endpoints are not directly connectable.
    expect(
      isListableCatalogProvider(
        "databricks",
        provider({ api: "https://${DATABRICKS_HOST}/ai-gateway/mlflow/v1" }),
      ),
    ).toBe(false);
    // A usable endpoint never rescues an untrusted relay.
    expect(isListableCatalogProvider("nano-gpt", provider({ api: "https://nano-gpt.com/api/v1" }))).toBe(false);

    // Majors whose models.dev entry omits `api` keep their documented
    // OpenAI-compatible base via the official fallback table.
    expect(isListableCatalogProvider("google", provider({ npm: "@ai-sdk/google" }))).toBe(true);
    expect(catalogProviderEndpoint("togetherai", provider({ npm: "@ai-sdk/togetherai" }))).toBe(
      "https://api.together.xyz/v1",
    );
    expect(catalogProviderEndpoint("cerebras", provider({ npm: "@ai-sdk/cerebras" }))).toBe(
      "https://api.cerebras.ai/v1",
    );
    expect(isListableCatalogProvider("vercel", provider({ npm: "@ai-sdk/gateway" }))).toBe(true);
  });

  it("only offers current agent-capable models and dedupes echoed model ids", () => {
    const model = (extra: Partial<ModelCatalogModel>): ModelCatalogModel => ({
      name: "M",
      limit: { context: 128_000, output: 8_192 },
      tool_call: true,
      release_date: "2026-03-01",
      ...extra,
    });

    expect(isListableCatalogModel(model({}))).toBe(true);
    // No tool calling → no agent use → hidden.
    expect(isListableCatalogModel(model({ tool_call: false }))).toBe(false);
    expect(isListableCatalogModel(model({ tool_call: undefined }))).toBe(false);
    // Pre-2026 releases and undated entries are too old for the picker.
    expect(isListableCatalogModel(model({ release_date: "2025-12-31" }))).toBe(false);
    expect(isListableCatalogModel(model({ release_date: undefined }))).toBe(false);
    expect(isListableCatalogModel(model({ status: "deprecated" }))).toBe(false);

    // "GLM-5.2" next to "glm-5.2" is noise; a genuinely different id is not.
    expect(isRedundantModelId("GLM-5.2", "glm-5.2")).toBe(true);
    expect(isRedundantModelId("", "glm-5.2")).toBe(true);
    expect(isRedundantModelId("DeepSeek V4 Flash", "deepseek-v4-flash")).toBe(true);
    expect(isRedundantModelId("Claude Fable 5", "anthropic/claude-fable-5")).toBe(false);
    expect(isRedundantModelId("Qwen Max", "qwen-max-2026-01-25")).toBe(false);

    const editor = read("src/components/settings/ProviderModelsEditor.vue");
    expect(editor).toContain("isListableCatalogModel");
    expect(editor).toContain("isRedundantModelId");
    // The expanded card header is a plain title bar; details live in the form.
    expect(editor).toContain('v-if="expandedKey !== rowKey(model, index)"');

    const picker = read("src/components/settings/ModelCatalogPicker.vue");
    expect(picker).toContain("isListableCatalogModel");
    expect(picker).toContain("isRedundantModelId");

    const step = read("src/components/settings/ProviderCatalogStep.vue");
    expect(step).toContain("isListableCatalogModel");
  });

  it("groups advanced model settings into sections scoped by wire format", () => {
    const source = read("src/components/settings/ProviderModelForm.vue");

    expect(source).toContain('t("settings.custom.formSectionBasic")');
    expect(source).toContain('t("settings.custom.formSectionReasoning")');
    expect(source).toContain('t("settings.custom.formSectionAnthropic")');
    expect(source).toContain("FORMATS_BY_API");
    expect(source).toContain("effectiveReasoningFormat");
    // Settings-sheet layout: ONE grid for the whole form (section titles span
    // both columns) so the label column — and thus every control edge — lines
    // up across sections instead of each section computing its own width.
    expect(source).toContain("grid-template-columns: max-content minmax(0, 1fr)");
    expect(source).toContain("grid-column: 1 / -1");
    expect(source).not.toContain('class="form-grid"');
    // Toggles are plain label/control rows too; the replay-field select is a
    // sibling row, not an indented nested block, and helper notes are gone.
    expect(source).not.toContain("form-nested");
    expect(source).not.toContain("form-note");
    expect(source).toContain("select-md");
    expect(source).not.toContain("form-grid-2");
    // Dropdowns are the themed BaseDropdown, never a native <select>.
    expect(source).toContain("BaseDropdown");
    expect(source).not.toContain("<select");
  });
});
