// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KnowledgeView from "../components/KnowledgeView.vue";
import {
  clearKnowledgeDocumentCacheForTests,
  getCachedKnowledgeDocument,
} from "../composables/knowledgeDocumentCache";
import { clearKnowledgeCatalogCacheForTests } from "../composables/knowledgeCatalogCache";
import type { KnowledgeDocument, KnowledgeDocumentSummary } from "../types";

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  edit: vi.fn(),
  notice: vi.fn(),
}));

vi.mock("../services/knowledge", async (importOriginal) => ({
  ...await importOriginal<typeof import("../services/knowledge")>(),
  knowledgeRead: mocks.read,
  knowledgeEdit: mocks.edit,
  listSkills: async () => [],
}));
vi.mock("../services/knowledgeWorkspaceEventHub", () => ({
  subscribeKnowledgeWorkspaceEvents: async () => () => undefined,
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({
    addNotice: mocks.notice,
    clearByOperation: vi.fn(),
  }),
}));
vi.mock("../components/ui/BaseMarkdownEditor.vue", () => ({
  default: defineComponent({
    props: ["modelValue"],
    setup: (props) => () => h("div", { class: "test-markdown" }, props.modelValue),
  }),
}));

function makeDocument(path = "AnimBehavior双域求值与权威Motion最终实施方案.md"): KnowledgeDocument {
  return {
    id: "plan-rename",
    type: "plan",
    path,
    title: path.replace(/\.md$/, ""),
    injectMode: "excerpt",
    effectiveInjectMode: "excerpt",
    readOnly: false,
    aiMaintained: false,
    effectiveAiMaintained: false,
    summary: "摘要",
    body: "# 正文标题\n\n保留正文内容。",
    maintenanceRules: null,
    effectiveMaintenanceRules: null,
    modifiedAt: 1,
  };
}

async function settle() {
  for (let i = 0; i < 20; i += 1) await nextTick();
}

let unmount: (() => void) | undefined;
let storedDocument: KnowledgeDocument;

async function mountDocument() {
  const target = ref<KnowledgeDocumentSummary>(storedDocument);
  const active = ref(true);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(defineComponent(() => () => h(KnowledgeView, {
    workingDir: "F:/rename-test",
    workspaceRef: { checkoutId: "rename-test", expectedGeneration: 1 },
    selectedModelId: "",
    modelDefaults: {
      mainModel: "",
      planModel: "",
      subagentModels: {},
      subagentEfforts: {},
      subagentFastModes: {},
    },
    embedded: true,
    active: active.value,
    selectedDocumentTarget: target.value,
    selectedDocumentId: target.value.id,
  })));
  app.use(createPinia());
  app.mount(host);
  unmount = () => app.unmount();
  await settle();
  const titleInput = () => host.querySelector<HTMLInputElement>(".document-title-input")!;
  const rename = async (name: string) => {
    const input = titleInput();
    input.value = name;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await settle();
  };
  return { target, active, host, titleInput, rename };
}

beforeEach(() => {
  clearKnowledgeDocumentCacheForTests();
  clearKnowledgeCatalogCacheForTests();
  vi.resetAllMocks();
  storedDocument = makeDocument();
  mocks.read.mockImplementation(async (request) => {
    if (request.path !== storedDocument.path) {
      throw new Error(`Knowledge document not found: plan/${request.path}`);
    }
    return { kind: "document", document: { ...storedDocument } };
  });
  mocks.edit.mockImplementation(async (request) => {
    if (request.path !== storedDocument.path) throw new Error("stale mutation path");
    storedDocument = {
      ...storedDocument,
      path: request.document.newPath ?? storedDocument.path,
      modifiedAt: storedDocument.modifiedAt + 1,
    };
    return { kind: "document", document: { ...storedDocument } };
  });
});

afterEach(() => {
  unmount?.();
  unmount = undefined;
  document.body.innerHTML = "";
});

describe("knowledge title rename", () => {
  it("keeps the renamed document open while the workbench target still has its old path", async () => {
    const mounted = await mountDocument();
    const oldPath = mounted.target.value.path;
    const name = "# Gameplay 权威运动与父子 AnimProgram 表现最终实施方案";
    await mounted.rename(name);

    expect(mounted.target.value.path).toBe(oldPath);
    expect(mounted.titleInput().value).toBe(name);
    expect(mocks.read).toHaveBeenCalledTimes(1);
    expect(mocks.notice).not.toHaveBeenCalled();
    expect(mounted.host.textContent).toContain("保留正文内容。");
    expect(getCachedKnowledgeDocument("F:/rename-test", {
      checkoutId: "rename-test", expectedGeneration: 1,
    }, mounted.target.value)).toBeNull();

    // A delayed workbench refresh may replace the target with the same old
    // metadata. It is not a request to navigate back to that removed path.
    mounted.target.value = { ...mounted.target.value };
    await settle();
    expect(mocks.read).toHaveBeenCalledTimes(1);

    await mounted.rename("Gameplay 权威运动与父子 AnimProgram 表现最终实施方案");
    expect(mocks.edit.mock.calls[1]?.[0].path).toBe(`${name}.md`);
    expect(mounted.titleInput().value).toBe(storedDocument.path.replace(/\.md$/, ""));
    expect(mocks.notice).not.toHaveBeenCalled();
  });

  it.each(["resolve", "reject"] as const)("ignores an old-path refresh that settles after rename: %s", async (outcome) => {
    const mounted = await mountDocument();
    const original = { ...storedDocument };
    let resolveRead!: (value: unknown) => void;
    let rejectRead!: (error: Error) => void;
    mocks.read.mockImplementationOnce(() => new Promise((resolve, reject) => {
      resolveRead = resolve;
      rejectRead = reject;
    }));
    mounted.active.value = false;
    await settle();
    mounted.active.value = true;
    await settle();
    expect(mocks.read).toHaveBeenCalledTimes(2);
    await mounted.rename("改名后的方案");

    if (outcome === "resolve") resolveRead({ kind: "document", document: original });
    else rejectRead(new Error(`Knowledge document not found: plan/${original.path}`));
    await settle();

    expect(mounted.titleInput().value).toBe("改名后的方案");
    expect(mocks.notice).not.toHaveBeenCalled();
  });

  it("opens a new path supplied by the workbench after an external rename", async () => {
    const mounted = await mountDocument();
    storedDocument = { ...storedDocument, path: "外部改名.md" };
    mounted.target.value = { ...storedDocument };
    await settle();
    expect(mounted.titleInput().value).toBe("外部改名");
    expect(mocks.read.mock.lastCall?.[0].path).toBe("外部改名.md");
    expect(mocks.notice).not.toHaveBeenCalled();
  });

  it("saves a title on blur and keeps failed renames available for retry", async () => {
    const mounted = await mountDocument();
    const oldPath = storedDocument.path;
    mocks.edit.mockRejectedValueOnce(new Error("Document already exists"));
    const input = mounted.titleInput();
    input.value = "同名方案";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("blur"));
    await settle();

    expect(storedDocument.path).toBe(oldPath);
    expect(input.value).toBe("同名方案");
    expect(mocks.notice).toHaveBeenCalledWith(
      "error", expect.stringContaining("Document already exists"), expect.anything(),
    );
    mocks.notice.mockClear();
    input.value = "可用名称";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("blur"));
    await settle();
    expect(storedDocument.path).toBe("可用名称.md");
    expect(mocks.edit.mock.lastCall?.[0].path).toBe(oldPath);
    expect(mocks.notice).not.toHaveBeenCalled();
  });
});
