import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("GitTerminal ask_user_question handling", () => {
  it("renders a reply card and sends answers through the session question channel", () => {
    const terminal = read("src/components/GitTerminal.vue");

    expect(terminal).toContain('import AskUserCard from "./chat/AskUserCard.vue";');
    expect(terminal).toContain("answerQuestion as answerSessionQuestion");
    expect(terminal).toContain("const pendingQuestion = ref<PendingQuestion | null>(null);");
    expect(terminal).toContain("async function answerPendingQuestion(answer: string)");
    expect(terminal).toContain("await answerSessionQuestion(question.questionId, answer);");
    expect(terminal).toContain('case "askUser":');
    expect(terminal).toContain('case "inputAnswered":');
    expect(terminal).toContain('<AskUserCard');
    expect(terminal).toContain('@answer="answerPendingQuestion"');
    expect(terminal).toContain('@click.stop');
  });
});
