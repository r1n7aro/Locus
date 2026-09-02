// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive } from "vue";
import { describe, expect, it, vi } from "vitest";
import ChatComposer from "../components/chat/ChatComposer.vue";

describe("interrupted chat resume", () => {
  it("exposes an enabled, labelled Resume action and emits only resume", async () => {
    const state = reactive({
      canResume: true,
      canSend: false,
    });
    const resume = vi.fn();
    const send = vi.fn();
    const host = document.createElement("div");
    const app = createApp(defineComponent({
      setup() {
        return () => h(ChatComposer, {
          modelValue: "",
          canResume: state.canResume,
          canSend: state.canSend,
          resumeLabel: "Resume interrupted session",
          onResume: resume,
          onSend: send,
        });
      },
    }));

    app.mount(host);
    await nextTick();

    const action = host.querySelector<HTMLButtonElement>(".chat-composer-action");
    expect(action).not.toBeNull();
    expect(action?.disabled).toBe(false);
    expect(action?.getAttribute("aria-label")).toBe("Resume interrupted session");
    expect(action?.classList.contains("is-resume")).toBe(true);

    action?.click();
    await nextTick();

    expect(resume).toHaveBeenCalledOnce();
    expect(send).not.toHaveBeenCalled();
    app.unmount();
  });
});
