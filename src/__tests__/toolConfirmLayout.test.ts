// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createPinia } from "pinia";
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import ToolConfirmCard from "../components/chat/ToolConfirmCard.vue";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool confirm layout", () => {
  it("renders single-card confirmation for one tool and batch card for multiple tools", () => {
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");

    expect(chatView).toContain('v-if="showBatchToolConfirmCard"');
    expect(chatView).toContain('v-else-if="showSingleToolConfirmCard"');
    expect(chatView).toContain("<ToolConfirmCard");

    expect(embeddedPane).toContain('v-if="showBatchToolConfirmCard"');
    expect(embeddedPane).toContain('v-else-if="showSingleToolConfirmCard"');
    expect(embeddedPane).toContain("<ToolConfirmCard");
  });

  it("uses the neutral Unity status confirmation treatment", () => {
    const card = read("src/components/chat/ToolConfirmCard.vue");
    const labels = read("src/components/chat/toolConfirmLabels.ts");
    const zh = read("src/language/zh.json");

    expect(card).toContain("is-unity-status-change");
    expect(card).toContain("unity-status-change-details");
    expect(card).toContain("titleForUnityEditorStatusChange");
    expect(labels).toContain("titleForUnityEditorStatusChange");
    expect(zh).toContain('"chat.toolConfirm.unityStatus.title.playing": "请求进入运行状态"');
  });

  it("renders plan approval content as constrained Markdown", async () => {
    const card = read("src/components/chat/ToolConfirmCard.vue");

    expect(card).toContain('import MarkdownRenderer from "../MarkdownRenderer.vue";');
    expect(card).toContain('<MarkdownRenderer :content="planApprovalDisplay.plan" />');
    expect(card).not.toContain('<pre class="plan-approval-content">');
    expect(card).toContain(".plan-approval-content :deep(.markdown-body)");
    expect(card).toMatch(/\.plan-approval-content\s*\{[^}]*max-width:\s*100%;[^}]*overflow:\s*auto;/s);

    const host = document.createElement("div");
    const app = createApp(ToolConfirmCard, {
      toolConfirm: {
        questionId: "question-plan",
        toolCallId: "tool-plan",
        display: {
          kind: "planApproval",
          planFilePath: "C:/Locus/data/plan/session.md",
          plan: "## 修改范围\n\n- 保留 `ReadFromFields()`\n- 更新解析流程",
        },
      },
    });
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    const preview = host.querySelector(".plan-approval-content .markdown-body");
    expect(preview?.querySelector("h2")?.textContent).toBe("修改范围");
    expect(preview?.querySelectorAll("li")).toHaveLength(2);
    expect(preview?.querySelector("code")?.textContent).toBe("ReadFromFields()");
    expect(preview?.textContent).not.toContain("##");
    expect(preview?.textContent).not.toContain("`");

    app.unmount();
  });

  it("collapses plan approval to approve / send-back with the note attached to send-back", async () => {
    const card = read("src/components/chat/ToolConfirmCard.vue");

    // No separately-submitted third action: the generic feedback form stays
    // exclusive to basic tool confirms, and the plan note rides on send-back.
    expect(card).toContain('v-if="basicDisplay"');
    expect(card).not.toContain('v-if="basicDisplay || planApprovalDisplay"');
    expect(card).toContain("handlePlanFeedbackEnter");
    expect(card).toContain("event.isComposing");

    const answers: string[] = [];
    const host = document.createElement("div");
    const app = createApp(ToolConfirmCard, {
      toolConfirm: {
        questionId: "question-plan",
        toolCallId: "tool-plan",
        display: {
          kind: "planApproval",
          planFilePath: "C:/Locus/data/plan/session.md",
          plan: "## 计划",
        },
      },
      onAnswer: (answer: string) => answers.push(answer),
    });
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    const buttons = [...host.querySelectorAll(".tool-confirm-actions button")];
    expect(buttons).toHaveLength(2);

    // Empty note → plain deny.
    (buttons[1] as HTMLButtonElement).click();
    expect(answers).toEqual(["deny"]);

    // With a note → feedback-encoded deny.
    const input = host.querySelector<HTMLInputElement>(".plan-approval-feedback-input");
    expect(input).toBeTruthy();
    input!.value = "先别动数据库";
    input!.dispatchEvent(new Event("input"));
    await nextTick();
    (buttons[1] as HTMLButtonElement).click();
    expect(answers).toEqual(["deny", "feedback:先别动数据库"]);

    app.unmount();
  });
});
