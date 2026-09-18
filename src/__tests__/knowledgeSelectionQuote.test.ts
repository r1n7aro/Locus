// @vitest-environment jsdom
import { EditorView } from "@codemirror/view";
import { createPinia } from "pinia";
import { createApp, nextTick, type App } from "vue";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import KnowledgePreview from "../components/knowledge/KnowledgePreview.vue";
import { KNOWLEDGE_QUOTE_SELECTION_KEY } from "../services/knowledgeSelection";
import type { KnowledgeDocument } from "../types";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../services/ipc", () => ({ ipcInvoke: invoke }));
vi.mock("../i18n", () => ({ t: (key: string) => key }));
let app: App | null = null;
const body = "# 体验目标\n\n- 连续交锋体验\n- 魔法进攻机制\n\n## 空间位置";
const documentData: KnowledgeDocument = {
  id: "boss", type: "design", path: "combat/boss.md", title: "boss",
  injectMode: "none", effectiveInjectMode: "none", readOnly: false,
  aiMaintained: false, effectiveAiMaintained: false, maintenanceRules: null,
  effectiveMaintenanceRules: null, modifiedAt: 1, body,
};
beforeEach(() => {
  invoke.mockReset().mockImplementation(async (command: string) => {
    if (command === "knowledge_document_source") return {
      path: "F:/Project/Locus/knowledge/design/combat/boss.md",
      content: "---\nid: boss\n---\n\n" + body,
    };
    return [];
  });
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true, value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
});
afterEach(() => { app?.unmount(); app = null; document.body.replaceChildren(); });

it("quotes from the knowledge editor through the scoped source API into the conversation draft", async () => {
  const host = document.createElement("div"); document.body.appendChild(host);
  const quote = vi.fn(async () => undefined);
  const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 7, expectedMaterializationEpoch: 3 };
  app = createApp(KnowledgePreview, { document: documentData, workspaceRef, loading: false, saveLoading: false });
  app.use(createPinia());
  app.provide(KNOWLEDGE_QUOTE_SELECTION_KEY, quote);
  app.mount(host);
  await nextTick();
  const view = EditorView.findFromDOM(host.querySelector(".document-body .cm-editor")!)!;
  view.dispatch({ selection: { anchor: body.indexOf("- 连续"), head: body.indexOf("\n\n##") } });
  view.contentDOM.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await nextTick(); await nextTick();
  document.querySelector<HTMLButtonElement>(".markdown-editor-context-menu button")!.click();
  for (let i = 0; i < 8; i++) await nextTick();
  expect(invoke).toHaveBeenCalledWith("knowledge_document_source", {
    workspaceRef, request: { docType: "design", path: "combat/boss.md", kind: "document" },
  });
  expect(quote).toHaveBeenCalledWith(workspaceRef, {
    path: "design/combat/boss.md",
    name: "boss",
    content: "F:/Project/Locus/knowledge/design/combat/boss.md:7-8\n```markdown\n- 连续交锋体验\n- 魔法进攻机制\n```",
  });
  expect(view.state.doc.toString()).toBe(body);
});
