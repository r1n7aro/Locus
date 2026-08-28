import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  ipcInvoke: vi.fn(),
}));

vi.mock("../services/ipc", () => ({
  ipcInvoke: mocks.ipcInvoke,
}));

import {
  mcpServerIntegrationApply,
  mcpServerIntegrationRemove,
  mcpServerIntegrations,
} from "../services/mcpServer";

describe("checkout-scoped MCP integration IPC", () => {
  beforeEach(() => {
    mocks.ipcInvoke.mockReset();
    mocks.ipcInvoke.mockResolvedValue([]);
  });

  it("rejects missing generation before invoking the backend", () => {
    expect(() => mcpServerIntegrations({ checkoutId: "checkout-a" })).toThrow(
      "checkout generation",
    );
    expect(() => mcpServerIntegrationApply("claude_code", {
      checkoutId: "checkout-a",
    })).toThrow("checkout generation");
    expect(() => mcpServerIntegrationRemove("claude_code", {
      checkoutId: "checkout-a",
    })).toThrow("checkout generation");
    expect(mocks.ipcInvoke).not.toHaveBeenCalled();
  });

  it("keeps concurrent checkout requests independently scoped", async () => {
    const workspaceA = { checkoutId: "checkout-a", expectedGeneration: 11 };
    const workspaceB = { checkoutId: "checkout-b", expectedGeneration: 23 };
    await Promise.all([
      mcpServerIntegrationApply("claude_code", workspaceA),
      mcpServerIntegrationApply("claude_code", workspaceB),
    ]);

    expect(mocks.ipcInvoke).toHaveBeenNthCalledWith(
      1,
      "mcp_server_integration_apply",
      { integrationId: "claude_code", workspaceRef: workspaceA },
    );
    expect(mocks.ipcInvoke).toHaveBeenNthCalledWith(
      2,
      "mcp_server_integration_apply",
      { integrationId: "claude_code", workspaceRef: workspaceB },
    );
  });
});
