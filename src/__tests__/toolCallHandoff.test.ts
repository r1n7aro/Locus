import { describe, expect, it } from "vitest";
import {
  retainMaterializedToolCallDisplays,
  settleToolCallDisplaysForHandoff,
} from "../composables/toolCallHandoff";
import type { ToolCallDisplay } from "../types";

function tool(
  id: string,
  status: ToolCallDisplay["status"],
  argumentsText = "{}",
  nestedToolCalls?: ToolCallDisplay[],
): ToolCallDisplay {
  return {
    id,
    name: "edit",
    arguments: argumentsText,
    status,
    nestedToolCalls,
  };
}

describe("tool call handoff presentation", () => {
  it("settles stale foreground running states recursively without mutating the source", () => {
    const source = [tool("parent", "running", "{}", [
      tool("nested-running", "running"),
      tool("nested-error", "error"),
    ])];

    const settled = settleToolCallDisplaysForHandoff(source);

    expect(settled[0]?.status).toBe("done");
    expect(settled[0]?.nestedToolCalls?.map((item) => item.status)).toEqual(["done", "error"]);
    expect(source[0]?.status).toBe("running");
    expect(source[0]?.nestedToolCalls?.[0]?.status).toBe("running");
  });

  it.each(["async", "notify", "async_notify"])(
    "keeps an explicit %s background tool running",
    (mode) => {
      const settled = settleToolCallDisplaysForHandoff([
        tool("background", "running", JSON.stringify({ async: mode })),
      ]);

      expect(settled[0]?.status).toBe("running");
    },
  );

  it("settles malformed foreground arguments conservatively", () => {
    const settled = settleToolCallDisplaysForHandoff([
      tool("malformed", "running", "{not-json"),
    ]);

    expect(settled[0]?.status).toBe("done");
  });

  it("drops provisional tool calls that never receive a payload", () => {
    const retained = retainMaterializedToolCallDisplays(
      [tool("provisional", "running", ""), tool("confirmed", "done")],
    );

    expect(retained.map((item) => item.id)).toEqual(["confirmed"]);
  });

  it("drops an empty completed handoff when the response contains no tools", () => {
    const retained = retainMaterializedToolCallDisplays(
      settleToolCallDisplaysForHandoff([tool("provisional", "running", "")]),
    );

    expect(retained).toEqual([]);
  });
});
