import { describe, expect, it } from "vitest";
import { editorBindingMatchesRuntime, workspaceRefForEditorBinding } from "../components/workbench/editorWorkspaceScope";
import { materializationEpochFromParams, workspaceMaterializationMatches, type WorkspaceRuntimeDescriptor } from "../services/project";
import { buildExtraWorkdirsWindowQuery, getExtraWorkdirsWindowPayload } from "../services/extraWorkdirsWindow";

const runtime = (epoch: number, generation = 1): WorkspaceRuntimeDescriptor => ({
  checkoutId: "slot", projectId: "project", root: "F:/pool/slot",
  workspaceGeneration: generation, materializationEpoch: epoch, leaseCount: 0, detectedServices: [],
});

describe("durable editor assignment", () => {
  it("does not upgrade an old buffer after pool reuse when process generations repeat", () => {
    const old = { checkoutId: "slot", expectedGeneration: 1, expectedMaterializationEpoch: 1 };
    expect(editorBindingMatchesRuntime(old, runtime(2))).toBe(false);
    expect(workspaceRefForEditorBinding(old, runtime(2))).toEqual(old);
    expect(workspaceRefForEditorBinding(old, runtime(2, 7))).toEqual(old);
    expect(old.expectedMaterializationEpoch).toBe(1);
  });

  it("refreshes the process generation only for the same durable assignment", () => {
    const persisted = { checkoutId: "slot", expectedMaterializationEpoch: 2 };
    expect(editorBindingMatchesRuntime(persisted, runtime(2, 9))).toBe(true);
    expect(workspaceRefForEditorBinding(persisted, runtime(2, 9))).toEqual({
      ...persisted, expectedGeneration: 9,
    });
    expect(workspaceRefForEditorBinding(persisted, { ...runtime(2, 9), checkoutId: "other" })).toEqual({
      ...persisted, expectedGeneration: undefined,
    });
  });

  it("allows historical unversioned handles only before slot reuse", () => {
    for (const epoch of [0, 1]) {
      expect(workspaceMaterializationMatches(undefined, epoch)).toBe(true);
    }
    expect(workspaceMaterializationMatches(undefined, 2)).toBe(false);
    expect(workspaceMaterializationMatches(null, 2)).toBe(false);
    expect(workspaceMaterializationMatches(0, 1)).toBe(false);
    expect(workspaceRefForEditorBinding({ checkoutId: "slot" }, runtime(2, 9))).toEqual({
      checkoutId: "slot", expectedGeneration: undefined, expectedMaterializationEpoch: undefined,
    });
  });

  it("roundtrips epoch zero and reused epochs through detached window URLs", () => {
    for (const epoch of [0, 2]) {
      const payload = { workspacePath: "F:/pool/slot", workspaceRef: {
        checkoutId: "slot", expectedGeneration: 1, expectedMaterializationEpoch: epoch,
      } };
      expect(getExtraWorkdirsWindowPayload(buildExtraWorkdirsWindowQuery(payload))).toEqual(payload);
    }
    expect(materializationEpochFromParams(new URLSearchParams())).toBeUndefined();
    for (const value of ["-1", "1.5", "NaN", "9007199254740993"]) {
      expect(() => materializationEpochFromParams(new URLSearchParams({ materializationEpoch: value }))).toThrow();
    }
  });
});
