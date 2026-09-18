import { ref } from "vue";
import * as Vue from "vue";
import * as Pinia from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getLocusRuntime } from "./locusRuntime";
import { findFrontendWorkbench } from "./frontendWorkbench";
import { normalizeAppError } from "./errors";
import { viewAutomationRespond, type ViewPackageDetail } from "./view";
import type { WorkspaceRef } from "./project";
import { compileViewPackage } from "../components/view/viewCompilation";
import { createViewExecutionScope } from "../components/view/viewExecutionScope";
import { createFrontendSdk, type FrontendImage } from "./frontendSdk";

interface FrontendRequest { requestId: string; code: string; workspaceRef: WorkspaceRef; targetLabel: string; timeoutMs?: number; }
let installed = false;
const requests = new Map<string, number>();

export async function executeFrontendTypeScript(code: string, workspaceRef: WorkspaceRef, timeoutMs = 30_000, target: { windowLabel?: string; ownerWindow?: Window } = {}) {
  const logs: Array<{ level: string; message: string }> = [];
  const images: FrontendImage[] = [];
  const execution = createViewExecutionScope({ viewId: "locus-frontend", editorId: "locus-frontend", ...target, workspaceRef, active: ref(true), state: new Map(), log: (level, args) => {
    if (logs.length < 128) logs.push({ level, message: args.map(String).join(" ").slice(0, 16_384) });
  } });
  const locus = createFrontendSdk(workspaceRef, { signal: execution.context.signal, images, ...target });
  const detail: ViewPackageDetail = {
    summary: { id: "locus-frontend", name: "Locus Frontend", apiVersion: "1", version: "1", displayPath: "", packageRelPath: "", packageRoot: "locus-frontend", manifestPath: "", updatedAt: 0, capabilities: { unity: false }, requirements: { unityConnection: false } },
    manifest: { schema: "locus.view.v1", apiVersion: "1", id: "locus-frontend", name: "Locus Frontend", version: "1", entry: "execute.ts", style: "", scripts: [], capabilities: { unity: false }, requirements: { unityConnection: false } },
    files: [{ relPath: "execute.ts", kind: "source", content: code, size: code.length, truncated: false }],
  };
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const task = (async () => {
      const compiled = await compileViewPackage(detail);
      if (execution.disposed) throw new Error("Frontend execution expired before compilation completed.");
      const globals = execution.globals(locus);
      const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
      const run = new AsyncFunction("__import", "exports", "module", "locus", ...Object.keys(globals).filter((key) => key !== "locus"), compiled.modules["execute.ts"]!.code);
      const module = { exports: {} };
      const importModule = (name: string) => {
        if (name === "@locus/frontend") return { locus };
        if (name === "vue") return execution.vueRuntime(Vue);
        if (name === "pinia") return Pinia;
        throw new Error(`Frontend tool module is not available: ${name}`);
      };
      return await run(importModule, module.exports, module, locus, ...Object.entries(globals).filter(([key]) => key !== "locus").map(([, value]) => value));
    })();
    const result = await Promise.race([task, new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => { execution.dispose(); reject(new Error("Frontend TypeScript execution timed out.")); }, Math.min(60_000, Math.max(250, timeoutMs)));
    })]);
    const seen = new WeakSet<object>();
    const safe = JSON.parse(JSON.stringify(result ?? null, (_key, value) => {
      if (typeof value === "bigint") return String(value);
      if (value instanceof Element) return { tag: value.tagName, text: value.textContent?.slice(0, 500) };
      if (value && typeof value === "object") { if (seen.has(value)) return "[Circular]"; seen.add(value); }
      return value;
    }));
    return { result: safe, logs, images };
  } finally { if (timer) clearTimeout(timer); execution.dispose(); }
}

export function bootstrapFrontendExecution() {
  if (installed) return;
  installed = true;
  const target = getCurrentWindow();
  void getLocusRuntime().subscribe<FrontendRequest>("locus-frontend-request", async (request) => {
    // Shared Workbench windows render in their opener's Vue realm. Only the
    // realm that owns that window's controller may execute its request.
    const workbench = findFrontendWorkbench(request.targetLabel);
    if (request.targetLabel !== target.label && !workbench) return;
    if (requests.has(request.requestId)) return;
    requests.set(request.requestId, Date.now());
    for (const [id, time] of requests) if (Date.now() - time > 120_000) requests.delete(id);
    let outcome: { ok: boolean; result?: Awaited<ReturnType<typeof executeFrontendTypeScript>>; error?: string };
    try { outcome = { ok: true, result: await executeFrontendTypeScript(request.code, request.workspaceRef, request.timeoutMs, { windowLabel: request.targetLabel, ownerWindow: workbench?.ownerWindow }) }; }
    catch (error) { outcome = { ok: false, error: normalizeAppError(error).message }; }
    try { await viewAutomationRespond(request.workspaceRef, request.requestId, outcome.ok, outcome.result, outcome.error); }
    catch (error) { console.debug("[frontend-sdk] response is no longer accepted", normalizeAppError(error).message); }
  }).catch(() => { installed = false; });
}
