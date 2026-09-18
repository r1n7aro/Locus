// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
const transport = vi.hoisted(() => ({ handler: null as ((event: any) => Promise<void>) | null, respond: vi.fn(async (..._args: unknown[]) => {}), listen: vi.fn() }));
vi.mock("../services/locusRuntime", () => ({ getLocusRuntime: () => ({ subscribe: transport.listen }) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: "main", listen: transport.listen }) }));
vi.mock("../services/view", async (original) => ({ ...await original<typeof import("../services/view")>(), viewAutomationRespond: transport.respond }));
import { bootstrapFrontendExecution } from "../services/frontendExecution";
import { registerFrontendWorkbench } from "../services/frontendWorkbench";

it("executes only in the addressed window and deduplicates transport retries", async () => {
  transport.listen.mockImplementation(async (_event, handler) => { transport.handler = handler; return () => {}; });
  bootstrapFrontendExecution();
  const request = { requestId: "request-one", code: "return 7 as number", workspaceRef: { checkoutId: "a", expectedGeneration: 1 }, targetLabel: "sub-pool-1" };
  await transport.handler!(request);
  expect(transport.respond).not.toHaveBeenCalled();
  await transport.handler!({ ...request, targetLabel: "main" });
  expect(transport.respond).toHaveBeenCalledTimes(1);
  expect(transport.respond.mock.calls[0]?.[3]).toEqual({ result: 7, logs: [], images: [] });
  await transport.handler!({ ...request, targetLabel: "main" });
  expect(transport.respond).toHaveBeenCalledTimes(1);
  const iframe = document.createElement("iframe");
  document.body.append(iframe);
  const ownerWindow = iframe.contentWindow!;
  ownerWindow.document.body.innerHTML = '<input id="shared-value">';
  const release = registerFrontendWorkbench({ ownerWindow, tabs: () => [], activate: async () => {}, close: async () => {} }, "workbench-shared");
  try {
    await transport.handler!({ ...request, requestId: "shared-request", targetLabel: "workbench-shared", code: 'await locus.ui.locator("#shared-value").fill("shared"); return document.querySelector("#shared-value").value;' });
    expect(transport.respond).toHaveBeenCalledTimes(2);
    expect(transport.respond.mock.calls[1]?.[3]).toEqual({ result: "shared", logs: [], images: [] });
    expect(document.querySelector("#shared-value")).toBeNull();
  } finally { release(); iframe.remove(); }
});
