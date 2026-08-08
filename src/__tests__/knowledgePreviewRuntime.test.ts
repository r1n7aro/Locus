// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp } from "vue";
import { describe, expect, it } from "vitest";
import KnowledgePreview from "../components/knowledge/KnowledgePreview.vue";

describe("KnowledgePreview runtime initialization", () => {
  it("initializes request state before the immediate skill status watcher runs", () => {
    const host = document.createElement("div");
    const runtimeErrors: unknown[] = [];
    const app = createApp(KnowledgePreview, {
      document: null,
      loading: false,
      saveLoading: false,
    });
    app.use(createPinia());
    app.config.errorHandler = (error) => runtimeErrors.push(error);

    expect(() => app.mount(host)).not.toThrow();
    expect(runtimeErrors).toEqual([]);
    expect(host.querySelector(".preview-empty")).not.toBeNull();

    app.unmount();
  });
});
