// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  workspaceRef: { checkoutId: "fence-checkout", expectedGeneration: 1 },
}));
vi.mock("../services/unitySerializedProperty", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/unitySerializedProperty")>()),
  readUnitySerializedProperty: mocks.read,
}));
vi.mock("../stores/project", () => ({
  useProjectStore: () => ({ requireWorkspaceRef: () => mocks.workspaceRef }),
}));
vi.mock("../services/unityValueEditorWindow", () => ({
  listenUnityValueEditorCommitted: vi.fn(async () => () => {}),
}));
vi.mock("../components/unity/UnitySerializedPropertyTree.vue", () => ({
  default: { name: "UnitySerializedPropertyTree", render: () => null },
}));

import UnityPropertyFenceBlock from "../components/unity/UnityPropertyFenceBlock.vue";
import { provideSurfaceHidden } from "../composables/surfaceVisibility";

let app: App | undefined;
let host: HTMLDivElement;

async function flush() {
  for (let index = 0; index < 10; index += 1) await nextTick();
}

function fence(count: number): string {
  return Array.from({ length: count }, (_, index) => `Assets/Data/Config${index}.asset#m_Name`).join("\n");
}

function mount(initiallyHidden: boolean, source: string) {
  const hidden = ref(initiallyHidden);
  const props = reactive({ source });
  const Host = defineComponent({
    setup() {
      provideSurfaceHidden(hidden);
      return () => h(UnityPropertyFenceBlock, props);
    },
  });
  app = createApp(Host);
  app.use(createPinia());
  app.mount(host);
  return { hidden, props };
}

beforeEach(() => {
  mocks.read.mockReset().mockImplementation(async () => ({ ok: true, propertyPath: "m_Name", value: "x" }));
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("Unity property fence in a hidden surface", () => {
  it("reads immediately when shown", async () => {
    mount(false, fence(2));
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);
  });

  it("defers bridge reads until the surface is shown", async () => {
    const { hidden } = mount(true, fence(2));
    await flush();
    expect(mocks.read).not.toHaveBeenCalled();

    hidden.value = false;
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);
  });

  it("follows the surfaceHidden prop when mounted outside a providing component tree", async () => {
    const props = reactive({ source: fence(2), surfaceHidden: true });
    app = createApp({ render: () => h(UnityPropertyFenceBlock, props) });
    app.use(createPinia());
    app.mount(host);
    await flush();
    expect(mocks.read).not.toHaveBeenCalled();

    props.surfaceHidden = false;
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);
  });

  it("defers a source change while hidden and reads only the current rows when shown", async () => {
    const { hidden, props } = mount(false, fence(2));
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);

    hidden.value = true;
    await flush();
    props.source = fence(3);
    await flush();
    props.source = fence(4);
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);

    hidden.value = false;
    await flush();
    expect(mocks.read).toHaveBeenCalledTimes(6);
  });
});
