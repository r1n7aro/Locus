import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("GitTerminal pending input handling", () => {
  it("renders terminal-native ask and confirm prompts through the session question channel", () => {
    const terminal = read("src/components/GitTerminal.vue");

    expect(terminal).not.toContain('import AskUserCard from "./chat/AskUserCard.vue";');
    expect(terminal).not.toContain('import ToolConfirmCard from "./chat/ToolConfirmCard.vue";');
    expect(terminal).toContain("answerQuestion as answerSessionQuestion");
    expect(terminal).toContain("const pendingQuestion = ref<PendingQuestion | null>(null);");
    expect(terminal).toContain("const pendingToolConfirm = ref<PendingToolConfirm | null>(null);");
    expect(terminal).toContain("async function answerPendingQuestion(answer: string)");
    expect(terminal).toContain("await answerSessionQuestion(question.questionId, answer);");
    expect(terminal).toContain("async function answerPendingToolConfirm(answer: string)");
    expect(terminal).toContain("await answerSessionQuestion(confirm.questionId, answer);");
    expect(terminal).toContain('case "askUser":');
    expect(terminal).toContain('case "toolConfirm":');
    expect(terminal).toContain('case "inputAnswered":');
    expect(terminal).toContain('class="term-inline-panel term-question-panel"');
    expect(terminal).toContain('class="term-inline-panel term-confirm-panel"');
    expect(terminal).toContain("pendingQuestionQuickOptions");
    expect(terminal).toContain("toolConfirmRows");
    expect(terminal).toContain("@click=\"answerPendingQuestion(option.label)\"");
    expect(terminal).toContain("@click=\"answerPendingToolConfirm('allow')\"");
    expect(terminal).toContain("answerPendingToolConfirmFeedback");
    expect(terminal).toContain('@click.stop');
  });
});
