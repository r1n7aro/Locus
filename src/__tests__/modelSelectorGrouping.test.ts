import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { groupModelsForSelector, modelListEntryName } from "../utils/modelGrouping";
import type { ModelOption } from "../types";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

const providerOrder = ["anthropic", "openai_codex", "custom"] as const;
const providerLabels: Record<string, string> = {
  anthropic: "Claude Subscription",
  openai_codex: "ChatGPT Subscription",
  custom: "Custom",
};

function model(partial: Partial<ModelOption> & Pick<ModelOption, "id" | "name" | "provider">): ModelOption {
  return { ...partial, id: partial.id, name: partial.name, provider: partial.provider };
}

describe("model selector grouping", () => {
  it("splits custom models into one section per custom provider, like subscription groups", () => {
    const models: ModelOption[] = [
      model({ id: "claude-opus-4.8", name: "Claude Opus 4.8", provider: "anthropic" }),
      model({ id: "openai/gpt-5.5", name: "GPT-5.5", provider: "openai_codex" }),
      model({
        id: "custom/qingyun/main",
        name: "qingyun-5.5",
        provider: "custom",
        customProviderId: "qingyun",
        customProviderName: "qingyun-5.5",
        customModelName: "qingyun-5.5",
      }),
      model({
        id: "custom/deepseek/v4-flash",
        name: "DeepSeek / DeepSeek V4 Flash",
        provider: "custom",
        customProviderId: "deepseek",
        customProviderName: "DeepSeek",
        customModelName: "DeepSeek V4 Flash",
      }),
      model({
        id: "custom/deepseek/v4-pro",
        name: "DeepSeek / DeepSeek V4 Pro",
        provider: "custom",
        customProviderId: "deepseek",
        customProviderName: "DeepSeek",
        customModelName: "DeepSeek V4 Pro",
      }),
    ];

    const groups = groupModelsForSelector(models, providerOrder, providerLabels);

    expect(groups.map((g) => g.key)).toEqual([
      "anthropic",
      "openai_codex",
      "custom:qingyun",
      "custom:deepseek",
    ]);
    expect(groups.map((g) => g.label)).toEqual([
      "Claude Subscription",
      "ChatGPT Subscription",
      "qingyun-5.5",
      "DeepSeek",
    ]);
    // Every custom group keeps provider "custom" so provider-specific UI
    // (e.g. the codex fast toggle check) stays keyed on real providers.
    expect(groups.filter((g) => g.key.startsWith("custom:")).every((g) => g.provider === "custom")).toBe(true);
    expect(groups[3].models.map((m) => m.id)).toEqual([
      "custom/deepseek/v4-flash",
      "custom/deepseek/v4-pro",
    ]);
  });

  it("keeps custom sections in configuration order and skips empty providers", () => {
    const models: ModelOption[] = [
      model({
        id: "custom/b/one",
        name: "B",
        provider: "custom",
        customProviderId: "b",
        customProviderName: "B Provider",
        customModelName: "one",
      }),
      model({
        id: "custom/a/one",
        name: "A",
        provider: "custom",
        customProviderId: "a",
        customProviderName: "A Provider",
        customModelName: "one",
      }),
    ];

    const groups = groupModelsForSelector(models, providerOrder, providerLabels);
    expect(groups.map((g) => g.key)).toEqual(["custom:b", "custom:a"]);
  });

  it("falls back to the generic custom label when grouping metadata is missing", () => {
    const models: ModelOption[] = [
      model({ id: "custom/legacy", name: "Legacy", provider: "custom" }),
    ];

    const groups = groupModelsForSelector(models, providerOrder, providerLabels);
    expect(groups).toHaveLength(1);
    expect(groups[0].key).toBe("custom:");
    expect(groups[0].label).toBe("Custom");
  });

  it("drops the provider prefix inside a custom section but keeps full names elsewhere", () => {
    const custom = model({
      id: "custom/deepseek/v4-flash",
      name: "DeepSeek / DeepSeek V4 Flash",
      provider: "custom",
      customProviderId: "deepseek",
      customProviderName: "DeepSeek",
      customModelName: "DeepSeek V4 Flash",
    });
    const builtin = model({ id: "claude-opus-4.8", name: "Claude Opus 4.8", provider: "anthropic" });
    const legacy = model({ id: "custom/legacy", name: "Legacy", provider: "custom" });

    expect(modelListEntryName(custom)).toBe("DeepSeek V4 Flash");
    expect(modelListEntryName(builtin)).toBe("Claude Opus 4.8");
    expect(modelListEntryName(legacy)).toBe("Legacy");
  });

  it("joins provider and model names with a slash, not a middle dot", () => {
    const source = read("src/stores/model.ts");
    expect(source).toContain("`${provider.name} / ${model.name}`");
    expect(source).not.toContain("·");
  });

  it("both selector dropdowns render per-provider custom sections", () => {
    for (const path of ["src/components/ModelSelector.vue", "src/components/ModelEffortSelector.vue"]) {
      const source = read(path);
      expect(source).toContain("groupModelsForSelector");
      expect(source).toContain('key="group.key"');
      expect(source).toContain("optionDisplayName(model)");
    }
  });

  it("collapsed trigger shows the bare name and only prefixes the provider on duplicates", () => {
    for (const path of ["src/components/ModelSelector.vue", "src/components/ModelEffortSelector.vue"]) {
      const source = read(path);
      // The duplicate check compares the same bare names the dropdown shows...
      expect(source).toMatch(/duplicated = props\.models\.some\(\s*\(m(odel)?\) => m(odel)?\.id !== sel(ected)?\.id && optionDisplayName\(m(odel)?\) === displayName,?\s*\)/);
      // ...and a duplicated custom model is prefixed with its provider name.
      expect(source).toContain("customProviderName} / ${displayName}");
    }
  });
});
