import { describe, expect, it } from "vitest";
import { resolveViewToolOpenId } from "../components/viewToolCallActions";

describe("view tool call actions", () => {
  it("resolves view_run ids from arguments", () => {
    expect(resolveViewToolOpenId({
      name: "view_run",
      arguments: JSON.stringify({ viewId: "runtime-entity-monitor" }),
      status: "done",
    })).toBe("runtime-entity-monitor");
  });

  it("keeps view_run hidden while the tool is running", () => {
    expect(resolveViewToolOpenId({
      name: "view_run",
      arguments: JSON.stringify({ viewId: "runtime-entity-monitor" }),
      status: "running",
    })).toBe("");
  });

  it("uses the actual returned id for temporary view_create calls", () => {
    expect(resolveViewToolOpenId({
      name: "view_create",
      arguments: JSON.stringify({ id: "store-ui-prototype", template: "blank", temporary: true }),
      output: JSON.stringify({
        summary: {
          id: "store-ui-prototype-tmp-f9fc49f1",
        },
      }),
      status: "done",
    })).toBe("store-ui-prototype-tmp-f9fc49f1");
  });

  it("does not expose view_create open actions for failed calls", () => {
    expect(resolveViewToolOpenId({
      name: "view_create",
      arguments: JSON.stringify({ id: "store-ui-prototype", template: "blank" }),
      output: "Error parsing view_create arguments",
      status: "error",
    })).toBe("");
  });
});
