// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createPinia } from "pinia";
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import ToolCallBlock from "../components/ToolCallBlock.vue";
import hljs from "../hljs";

const root = process.cwd();

describe("highlight theme", () => {
  it("overrides Markdown emphasis that can span numbered Skill output", () => {
    const highlighted = hljs.highlight([
      "1\t---",
      "2\tid: kd_01ea75c3-43c6-4ef7-a0bd-d509c6378eb8",
      "3\tinjectMode: excerpt",
    ].join("\n"), { language: "markdown" }).value;
    const theme = readFileSync(resolve(root, "src/assets/hljs-theme.css"), "utf8");
    const readableTokenRule = theme.match(
      /:root \.hljs-code,[\s\S]*?:root \.hljs-tag\s*\{([^}]+)\}/,
    )?.[1] ?? "";

    expect(highlighted).toContain('class="hljs-emphasis"');
    expect(readableTokenRule).toContain("color: var(--md-code-fg)");
  });

  it("replaces the imported light-theme Markdown and diff fallbacks", () => {
    const theme = readFileSync(resolve(root, "src/assets/hljs-theme.css"), "utf8");

    for (const selector of [
      ".hljs-emphasis",
      ".hljs-strong",
      ".hljs-quote",
      ".hljs-bullet",
      ".hljs-addition",
      ".hljs-deletion",
    ]) {
      expect(theme).toContain(`:root ${selector}`);
    }
    expect(theme).toContain("background-color: var(--status-good-bg)");
    expect(theme).toContain("background-color: var(--status-danger-bg)");
  });

  it("renders numbered Markdown reads as stable plain text", async () => {
    const host = document.createElement("div");
    const app = createApp(ToolCallBlock, {
      toolCall: {
        id: "read-skill",
        name: "read",
        arguments: JSON.stringify({
          filePath: "Locus/knowledge/skill/ecs-action-authoring.md",
        }),
        status: "done",
        output: [
          "<content>",
          "1\t---",
          "2\tid: kd_01ea75c3-43c6-4ef7-a0bd-d509c6378eb8",
          "3\tinjectMode: excerpt",
          "</content>",
        ].join("\n"),
      },
    });
    app.use(createPinia());
    app.mount(host);
    host.querySelector<HTMLButtonElement>(".tool-call-header")?.click();
    await nextTick();

    const output = host.querySelector(".tool-call-pre");
    expect(output?.classList.contains("hljs")).toBe(false);
    expect(output?.textContent).toContain("kd_01ea75c3-43c6-4ef7-a0bd-d509c6378eb8");
    expect(output?.textContent).not.toContain("<content>");

    app.unmount();
  });
});
