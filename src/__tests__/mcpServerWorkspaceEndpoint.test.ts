import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import {
  buildScopedMcpServerArtifacts,
  buildScopedMcpServerEndpoint,
  buildScopedMcpServerEntryName,
} from "../services/mcpServer";

const cwd = process.cwd();

function read(relativePath: string): string {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

describe("workspace-scoped MCP server endpoint", () => {
  it("encodes checkout identity and pins the runtime generation", () => {
    const endpoint = buildScopedMcpServerEndpoint(
      "http://127.0.0.1:27121/mcp",
      {
        checkoutId: "checkout-A/worktree?",
        expectedGeneration: 42,
      },
    );
    const parsed = new URL(endpoint);

    expect(`${parsed.origin}${parsed.pathname}`).toBe("http://127.0.0.1:27121/mcp");
    expect(parsed.searchParams.get("checkoutId")).toBe("checkout-A/worktree?");
    expect(parsed.searchParams.get("workspaceGeneration")).toBe("42");
  });

  it("replaces stale scope parameters and rejects incomplete WorkspaceRef values", () => {
    const endpoint = buildScopedMcpServerEndpoint(
      "http://127.0.0.1:27121/mcp?checkoutId=old&workspaceGeneration=1",
      { checkoutId: "checkout-B", expectedGeneration: 9 },
    );
    const parsed = new URL(endpoint);
    expect(parsed.searchParams.getAll("checkoutId")).toEqual(["checkout-B"]);
    expect(parsed.searchParams.getAll("workspaceGeneration")).toEqual(["9"]);

    expect(() => buildScopedMcpServerEndpoint(
      "http://127.0.0.1:27121/mcp",
      { checkoutId: "", expectedGeneration: 9 },
    )).toThrow("checkout ID");
    expect(() => buildScopedMcpServerEndpoint(
      "http://127.0.0.1:27121/mcp",
      { checkoutId: "checkout-B" },
    )).toThrow("checkout generation");
  });

  it("builds checkout-generation-specific command and JSON artifacts", () => {
    const workspaceRef = { checkoutId: "checkout-A/worktree?", expectedGeneration: 42 };
    const artifacts = buildScopedMcpServerArtifacts(
      "http://127.0.0.1:27121/mcp",
      "tok-abc",
      workspaceRef,
    );
    expect(artifacts.entryName).toBe("locus-checkout-a-worktree");
    expect(buildScopedMcpServerEntryName(workspaceRef)).toBe(artifacts.entryName);
    expect(artifacts.endpointUrl).toContain("checkoutId=checkout-A%2Fworktree%3F");
    expect(artifacts.endpointUrl).toContain("workspaceGeneration=42");
    expect(artifacts.claudeCodeCommand).toContain(artifacts.entryName);
    expect(artifacts.claudeCodeCommand).toContain(`"${artifacts.endpointUrl}"`);
    const json = JSON.parse(artifacts.jsonSnippet);
    expect(json.mcpServers[artifacts.entryName].url).toBe(artifacts.endpointUrl);
    expect(json.mcpServers[artifacts.entryName].headers.Authorization).toBe("Bearer tok-abc");
  });

  it("copies every setup artifact from the checkout menu while process settings stay app-scoped", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const service = read("src/services/mcpServer.ts");
    const settings = read("src/components/settings/McpServerSettings.vue");
    const backend = read("src-tauri/src/mcp/server/mod.rs");

    expect(workbench).toContain("copyCheckoutMcpArtifact");
    expect(workbench).toContain("buildScopedMcpServerArtifacts(");
    expect(workbench).toContain("checkoutWorkspaceRef(checkout)");
    expect(workbench).toContain("contextMenu.item.meta.kind === 'checkout'");
    expect(workbench).toContain("copyCheckoutMcpArtifact('endpoint')");
    expect(workbench).toContain("copyCheckoutMcpArtifact('claude')");
    expect(workbench).toContain("copyCheckoutMcpArtifact('json')");
    expect(workbench).toContain('t("app.dir.copyMcpEndpoint")');
    expect(workbench).toContain('t("app.dir.copyMcpClaudeCommand")');
    expect(workbench).toContain('t("app.dir.copyMcpJson")');
    expect(service).not.toContain("useWorkspaceContextStore");
    expect(settings).not.toContain("useWorkspaceContextStore");
    expect(settings).toContain("mcpServerGetState");
    expect(settings).not.toContain("mcpServerIntegrations");
    expect(settings).not.toContain("manualSetup");
    expect(settings).not.toContain("genericJsonSnippet");
    expect(backend).not.toContain("WindowContextRegistry");
    expect(backend).not.toContain('pane("main", "main")');
  });

  it("keeps the checkout action localized", () => {
    const zh = JSON.parse(read("src/language/zh.json")) as Record<string, string>;
    const en = JSON.parse(read("src/language/en.json")) as Record<string, string>;
    expect(zh["app.dir.copyMcpEndpoint"]).toBe("复制 MCP 端点");
    expect(zh["app.dir.copyMcpClaudeCommand"]).toBe("复制 Claude MCP 命令");
    expect(zh["app.dir.copyMcpJson"]).toBe("复制 MCP JSON");
    expect(en["app.dir.copyMcpEndpoint"]).toBe("Copy MCP endpoint");
    expect(en["app.dir.copyMcpClaudeCommand"]).toBe("Copy Claude MCP command");
    expect(en["app.dir.copyMcpJson"]).toBe("Copy MCP JSON");
  });
});
