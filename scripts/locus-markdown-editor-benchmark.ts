import { JSDOM } from "jsdom";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import {
  markdownEditorBaseExtensions,
  markdownEditorLanguageExtension,
  markdownEditorModeExtension,
} from "../src/components/ui/markdown-editor/codeMirrorMarkdownExtensions";
import { createMinimalTextChange } from "../src/components/ui/markdown-editor/markdownEditorTransactions";

const RUNS = 30;

function installDom(): void {
  const dom = new JSDOM("<!doctype html><body><div id=app></div></body>", {
    pretendToBeVisual: true,
  });
  const win = dom.window;
  Object.assign(globalThis, {
    window: win,
    document: win.document,
    navigator: win.navigator,
    MutationObserver: win.MutationObserver,
    DOMParser: win.DOMParser,
    HTMLElement: win.HTMLElement,
    Node: win.Node,
    Range: win.Range,
    getComputedStyle: win.getComputedStyle.bind(win),
    requestAnimationFrame: (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now()), 0),
    cancelAnimationFrame: clearTimeout,
  });
  if (!win.Range.prototype.getClientRects) {
    Object.defineProperty(win.Range.prototype, "getClientRects", {
      configurable: true,
      value: () => [],
    });
  }
  if (!win.Range.prototype.getBoundingClientRect) {
    Object.defineProperty(win.Range.prototype, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }),
    });
  }
}

function repeatedMarkdown(targetCharacters: number): string {
  const paragraph = [
    "## 输入延迟验证",
    "",
    "普通文本、**强调**、`Assets/Scripts/Player.cs:42` 与 [链接](https://example.com)。",
    "",
    "- [ ] 保持选区、撤销历史与自动保存",
    "- [x] 只更新可见语法范围",
    "",
  ].join("\n");
  return paragraph.repeat(Math.ceil(targetCharacters / paragraph.length)).slice(0, targetCharacters);
}

const complexCorpus = [
  "# Complex Live Preview",
  "",
  "| Name | State | Notes |",
  "| :--- | :---: | ---: |",
  "| Player | Ready | 42 |",
  "| Enemy | Pending | 7 |",
  "",
  "Inline $E = mc^2$ and display:",
  "",
  "$$",
  "\\int_0^1 x^2 dx",
  "$$",
  "",
  "![Scene](Assets/Docs/scene.png)",
  "",
  "`design/combat/core-loop.md`",
  "`Assets/Scenes/Main.unity/Root/Player`",
  "",
  "```unity-property",
  "Assets/Prefabs/Player.prefab | PlayerController.speed",
  "```",
].join("\n");

function percentile(samples: number[], ratio: number): number {
  const sorted = [...samples].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * ratio) - 1));
  return Number((sorted[index] ?? 0).toFixed(3));
}

function stats(samples: number[]) {
  return {
    p50Ms: percentile(samples, 0.5),
    p95Ms: percentile(samples, 0.95),
    maxMs: Number(Math.max(...samples).toFixed(3)),
  };
}

function createState(doc: string, mode: Compartment): EditorState {
  return EditorState.create({
    doc,
    extensions: [
      ...markdownEditorBaseExtensions(() => undefined),
      markdownEditorLanguageExtension("markdown"),
      mode.of(markdownEditorModeExtension("rendered", "markdown")),
    ],
  });
}

function measureEditor(doc: string) {
  const parent = document.createElement("div");
  document.body.appendChild(parent);
  const mode = new Compartment();
  const mountStarted = performance.now();
  const view = new EditorView({ state: createState(doc, mode), parent });
  const mountMs = performance.now() - mountStarted;
  const dom = view.dom;

  const input: number[] = [];
  for (let index = 0; index < RUNS; index += 1) {
    const started = performance.now();
    view.dispatch({ changes: { from: view.state.doc.length, insert: "x" } });
    input.push(performance.now() - started);
  }

  const modeSwitch: number[] = [];
  for (let index = 0; index < RUNS; index += 1) {
    const nextMode = index % 2 === 0 ? "native" : "rendered";
    const started = performance.now();
    view.dispatch({
      effects: mode.reconfigure(markdownEditorModeExtension(nextMode, "markdown")),
      annotations: Transaction.addToHistory.of(false),
    });
    modeSwitch.push(performance.now() - started);
  }

  const next = `${view.state.doc.sliceString(0, 24)}[external]${view.state.doc.sliceString(24)}`;
  const externalDiffStarted = performance.now();
  const externalChange = createMinimalTextChange(view.state.doc.toString(), next);
  const externalDiffMs = performance.now() - externalDiffStarted;
  const externalStarted = performance.now();
  if (externalChange) {
    view.dispatch({
      changes: externalChange,
      annotations: Transaction.addToHistory.of(false),
    });
  }
  const externalPatchMs = performance.now() - externalStarted;
  const stableDom = view.dom === dom && parent.querySelectorAll(".cm-editor").length === 1;

  view.destroy();
  parent.remove();
  return {
    characters: doc.length,
    mountMs: Number(mountMs.toFixed(3)),
    input: stats(input),
    modeSwitch: stats(modeSwitch),
    externalDiffMs: Number(externalDiffMs.toFixed(3)),
    externalPatchMs: Number(externalPatchMs.toFixed(3)),
    stableDom,
  };
}

function measureRepetitiveMultiPointDiff() {
  const current = `${"a".repeat(524_288)}MIDDLE${"z".repeat(524_281)}`;
  const next = `AAA${current.slice(3, -3)}ZZZ`;
  const started = performance.now();
  const changes = createMinimalTextChange(current, next);
  const diffMs = performance.now() - started;
  const state = EditorState.create({ doc: current });
  const applyStarted = performance.now();
  const updated = changes ? state.update({ changes }).state : state;
  return {
    characters: current.length,
    diffMs: Number(diffMs.toFixed(3)),
    applyMs: Number((performance.now() - applyStarted).toFixed(3)),
    hunkCount: Array.isArray(changes) ? changes.length : changes ? 1 : 0,
    exact: updated.doc.toString() === next,
  };
}

function measureDocumentRoundTrip() {
  const parent = document.createElement("div");
  document.body.appendChild(parent);
  const states = Array.from({ length: 20 }, (_, index) =>
    EditorState.create({ doc: `# Document ${index}\n\n${repeatedMarkdown(8_000)}` })
  );
  const view = new EditorView({ state: states[0], parent });
  const dom = view.dom;
  const samples: number[] = [];
  for (let run = 0; run < RUNS; run += 1) {
    const started = performance.now();
    for (const state of states) view.setState(state);
    for (let index = states.length - 2; index >= 0; index -= 1) view.setState(states[index]);
    samples.push(performance.now() - started);
  }
  const stableDom = view.dom === dom && parent.querySelectorAll(".cm-editor").length === 1;
  view.destroy();
  parent.remove();
  return { roundTrip: stats(samples), stableDom };
}

installDom();
const result = {
  environment: "jsdom-structural",
  runs: RUNS,
  corpora: {
    small1Kb: measureEditor(repeatedMarkdown(1_024)),
    medium100Kb: measureEditor(repeatedMarkdown(100 * 1_024)),
    large1Mb: measureEditor(repeatedMarkdown(1_024 * 1_024)),
    complex: measureEditor(complexCorpus),
  },
  documents20: measureDocumentRoundTrip(),
  repetitiveMultiPoint1Mb: measureRepetitiveMultiPointDiff(),
};

console.log(`MARKDOWN_EDITOR_BENCHMARK ${JSON.stringify(result)}`);
