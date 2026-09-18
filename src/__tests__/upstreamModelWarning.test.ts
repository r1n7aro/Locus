// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick } from "vue";
import TopBannerHost from "../components/TopBannerHost.vue";
import { normalizeAppError } from "../services/errors";
import { useNotificationStore } from "../stores/notification";
import zh from "../language/zh.json";

vi.mock("../i18n", () => ({
  t: (key: string, ...args: unknown[]) => (zh[key as keyof typeof zh] ?? key)
    .replace(/\{(\d+)\}/g, (_, index: string) => String(args[Number(index)])),
}));

const warning = {
  code: "llm.upstream_model_mismatch",
  message: "Upstream response model differs from the request.",
  severity: "warning" as const,
  retryable: false,
  operation: "upstream-model:session",
  detail: JSON.stringify({ requestedModel: "gpt-requested", reportedModel: "gpt-reported", sessionId: "session" }),
};

beforeEach(() => { vi.useFakeTimers(); setActivePinia(createPinia()); });
afterEach(() => { useNotificationStore().clearAll(); vi.useRealTimers(); });

describe("upstream model warning banner", () => {
  it("uses the standard warning banner with both model IDs and localizes the message", () => {
    const payload = normalizeAppError(warning);
    const store = useNotificationStore();
    store.addNotice(payload.severity, payload.message, { code: payload.code, operation: payload.operation });
    expect(store.visibleNotices).toHaveLength(1);
    expect(store.visibleNotices[0]).toMatchObject({
      level: "warning", message: "上游响应模型不一致：请求 gpt-requested，返回 gpt-reported。",
      code: warning.code, sticky: false,
    });
  });

  it("renders a dismissible warning through the existing banner host", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const app = createApp(TopBannerHost);
    app.mount(root);
    try {
      const payload = normalizeAppError(warning);
      const store = useNotificationStore();
      store.addNotice(payload.severity, payload.message, { code: payload.code, operation: payload.operation });
      await nextTick();
      expect(root.querySelector(".banner-warning .banner-msg")?.textContent).toBe(payload.message);
      root.querySelector<HTMLButtonElement>(".banner-close")?.click();
      await nextTick();
      expect(store.visibleNotices).toHaveLength(0);
    } finally {
      app.unmount();
      root.remove();
    }
  });

  it.each(["legacy detail", "null", "{}", '{"requestedModel":4}'])("keeps readable fallback for malformed detail %s", (detail) => {
    expect(normalizeAppError({ ...warning, detail }).message).toBe(warning.message);
  });
});
