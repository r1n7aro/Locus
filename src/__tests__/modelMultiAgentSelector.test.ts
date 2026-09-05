// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import ModelEffortSelector from "../components/ModelEffortSelector.vue";

describe("model multi agent selection", () => {
  it.each([true, false])("toggles independently and remains available with effortSupported=%s", async (effortSupported) => {
    const enabled = ref(false);
    const effort = ref<"high" | "max">("high");
    const fastMode = vi.fn();
    const host = document.createElement("div");
    document.body.append(host);
    const app = createApp(defineComponent({
      setup: () => () => h(ModelEffortSelector, {
        models: [{ id: "openai/gpt-6-astra", name: "GPT-6 Astra", provider: "openai_codex" }],
        selectedId: "openai/gpt-6-astra",
        effort: effort.value,
        efforts: ["high", "max"],
        effortSupported,
        multiAgentEnabled: enabled.value,
        onSelectMultiAgent: (value: boolean) => { enabled.value = value; },
        onSelectEffort: (value) => { effort.value = value as "high" | "max"; },
        onSelectFastMode: fastMode,
      }),
    }));
    app.mount(host);
    try {
      const open = async () => {
        host.querySelector<HTMLButtonElement>(".model-effort-trigger")!.click();
        await nextTick();
      };
      await open();
      const toggle = host.querySelector<HTMLButtonElement>(".model-effort-multi-agent")!;
      expect(toggle.textContent).toBe("Multi-Agent");
      expect(toggle.getAttribute("aria-pressed")).toBe("false");
      expect(toggle.parentElement!.lastElementChild).toBe(toggle);
      toggle.click();
      await nextTick();
      expect(toggle.getAttribute("aria-pressed")).toBe("true");
      expect(toggle.classList.contains("active")).toBe(true);
      expect(effort.value).toBe("high");
      if (effortSupported) {
        const max = [...host.querySelectorAll<HTMLButtonElement>(".model-effort-effort-panel button")]
          .find((button) => button.textContent?.trim() === "Max")!;
        max.click();
        await nextTick();
        expect(effort.value).toBe("max");
        expect(enabled.value).toBe(true);
        await open();
      }
      host.querySelector<HTMLButtonElement>(".model-effort-multi-agent")!.click();
      await nextTick();
      expect(enabled.value).toBe(false);
      expect(fastMode).not.toHaveBeenCalled();
    } finally {
      app.unmount();
      host.remove();
    }
  });
});
