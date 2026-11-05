import { describe, expect, it } from "vitest";
import {
  makeDefaultSubagentModel,
  normalizeSubagentModelChoices,
  toggleSubagentModel,
  withSubagentModels,
} from "../utils/subagentModels";

describe("sub-agent model choices", () => {
  it("reads legacy single models and cleans lists", () => {
    expect(normalizeSubagentModelChoices({
      explorer: "openai/gpt-5.6-luna",
      researcher: [" a ", "", "b", "a", 3],
      doc: [],
      wiki: "  ",
      simple: null,
    })).toEqual({
      explorer: ["openai/gpt-5.6-luna"],
      researcher: ["a", "b"],
    });
    expect(normalizeSubagentModelChoices(null)).toEqual({});
    expect(normalizeSubagentModelChoices(["a"])).toEqual({});
  });

  it("toggles membership, promotes defaults, and drops emptied entries", () => {
    expect(toggleSubagentModel(["a"], "b")).toEqual(["a", "b"]);
    expect(toggleSubagentModel(["a", "b"], "a")).toEqual(["b"]);
    expect(makeDefaultSubagentModel(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
    expect(makeDefaultSubagentModel(["a"], "missing")).toEqual(["a"]);

    const choices = { explorer: ["a"], doc: ["b"] };
    expect(withSubagentModels(choices, "explorer", ["a", "c"])).toEqual({ explorer: ["a", "c"], doc: ["b"] });
    expect(withSubagentModels(choices, "doc", [])).toEqual({ explorer: ["a"] });
    expect(choices).toEqual({ explorer: ["a"], doc: ["b"] });
  });
});
