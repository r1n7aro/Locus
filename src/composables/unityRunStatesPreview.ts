export type UnityRunStatesPhaseKey = "variables" | "start" | "update" | "end";

export interface UnityRunStatesPhasePreview {
  key: UnityRunStatesPhaseKey;
  code: string;
  empty: boolean;
}

export interface UnityRunStatesStatePreview {
  name: string;
  isInitial: boolean;
  phases: UnityRunStatesPhasePreview[];
}

export interface UnityRunStatesArgsPreview {
  requestEditorStatus: string;
  initialState: string;
  states: UnityRunStatesStatePreview[];
}

export interface UnityRunStatesOutputField {
  key: string;
  label: string;
  value: string;
}

export interface UnityRunStatesOutputPreview {
  fields: UnityRunStatesOutputField[];
  prints: string;
}

export interface UnityRunStatesRuntimePrompt {
  token: string;
  message: string;
  stateName: string;
  phase: UnityRunStatesPhaseKey;
}

export interface UnityRunStatesRuntimePrint {
  value: string;
  stateName: string;
  phase: UnityRunStatesPhaseKey;
}

export interface UnityRunStatesRuntimePreview {
  currentState: string;
  promptText: string;
  printText: string;
  printCount: number;
  finalStatus: string;
  finalMessage: string;
  isFinal: boolean;
}

const PHASE_KEYS: UnityRunStatesPhaseKey[] = ["variables", "start", "update", "end"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(record: Record<string, unknown>, ...keys: string[]): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") return value;
  }
  return "";
}

function normalizeSnippet(value: string): string {
  return value.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim();
}

function hasVerbatimStringPrefix(source: string, quoteIndex: number): boolean {
  const prefix = source.slice(Math.max(0, quoteIndex - 2), quoteIndex);
  return prefix.includes("@");
}

function splitCSharpLines(source: string): string[] {
  const lines: string[] = [];
  let current = "";
  let quote: "\"" | "'" | null = null;
  let escaped = false;
  let verbatim = false;
  let parenDepth = 0;

  const pushLine = () => {
    const line = current.trim();
    if (line) lines.push(line);
    current = "";
  };

  for (let i = 0; i < source.length; i += 1) {
    const char = source[i] ?? "";
    const next = source[i + 1] ?? "";

    if (char === "\n" && quote === null) {
      pushLine();
      continue;
    }

    current += char;

    if (quote !== null) {
      if (quote === "\"" && verbatim) {
        if (char === "\"" && next === "\"") {
          current += next;
          i += 1;
          continue;
        }
        if (char === "\"") quote = null;
        continue;
      }

      if (escaped) {
        escaped = false;
        continue;
      }
      if (char === "\\") {
        escaped = true;
        continue;
      }
      if (char === quote) quote = null;
      continue;
    }

    if (char === "\"" || char === "'") {
      quote = char;
      verbatim = char === "\"" && hasVerbatimStringPrefix(source, i);
      escaped = false;
      continue;
    }

    if (char === "(") {
      parenDepth += 1;
      continue;
    }
    if (char === ")") {
      parenDepth = Math.max(0, parenDepth - 1);
      continue;
    }

    if (char === "{" || char === "}") {
      pushLine();
      continue;
    }

    if (char === ";" && parenDepth === 0) {
      pushLine();
    }
  }

  pushLine();
  return lines;
}

function indentCSharpLines(lines: string[]): string {
  let indent = 0;
  return lines
    .map((line) => {
      if (line.startsWith("}")) {
        indent = Math.max(0, indent - 1);
      }

      const formatted = `${"  ".repeat(indent)}${line}`;

      if (line.endsWith("{")) {
        indent += 1;
      }

      return formatted;
    })
    .join("\n");
}

export function formatUnityRunStatesSnippet(value: string): string {
  const normalized = normalizeSnippet(value);
  if (!normalized) return "";
  return indentCSharpLines(splitCSharpLines(normalized));
}

export function parseUnityRunStatesArguments(rawArguments: string): UnityRunStatesArgsPreview | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawArguments);
  } catch {
    return null;
  }

  if (!isRecord(parsed) || !Array.isArray(parsed.states)) return null;

  const initialState = readString(parsed, "initial_state", "initialState");
  const states = parsed.states
    .filter(isRecord)
    .map((state, index): UnityRunStatesStatePreview => {
      const name = readString(state, "name") || `state_${index + 1}`;
      return {
        name,
        isInitial: Boolean(initialState && name === initialState),
        phases: PHASE_KEYS.map((key) => {
          const code = formatUnityRunStatesSnippet(readString(state, key));
          return {
            key,
            code,
            empty: code.length === 0,
          };
        }),
      };
    });

  if (states.length === 0) return null;

  return {
    requestEditorStatus: readString(parsed, "request_editor_status", "requestEditorStatus"),
    initialState,
    states,
  };
}

function splitCallArguments(source: string): string[] {
  const args: string[] = [];
  let current = "";
  let quote: "\"" | "'" | null = null;
  let escaped = false;
  let verbatim = false;
  let depth = 0;

  const pushArg = () => {
    const value = current.trim();
    args.push(value);
    current = "";
  };

  for (let i = 0; i < source.length; i += 1) {
    const char = source[i] ?? "";
    const next = source[i + 1] ?? "";

    current += char;

    if (quote !== null) {
      if (quote === "\"" && verbatim) {
        if (char === "\"" && next === "\"") {
          current += next;
          i += 1;
          continue;
        }
        if (char === "\"") quote = null;
        continue;
      }

      if (escaped) {
        escaped = false;
        continue;
      }
      if (char === "\\") {
        escaped = true;
        continue;
      }
      if (char === quote) quote = null;
      continue;
    }

    if (char === "\"" || char === "'") {
      quote = char;
      verbatim = char === "\"" && hasVerbatimStringPrefix(source, i);
      escaped = false;
      continue;
    }

    if (char === "(" || char === "[" || char === "{") {
      depth += 1;
      continue;
    }

    if (char === ")" || char === "]" || char === "}") {
      depth = Math.max(0, depth - 1);
      continue;
    }

    if (char === "," && depth === 0) {
      current = current.slice(0, -1);
      pushArg();
    }
  }

  if (current.trim() || source.endsWith(",")) pushArg();
  return args;
}

function extractCtxCalls(source: string, methodName: "PromptUser" | "Print"): string[][] {
  const calls: string[][] = [];
  const needle = `ctx.${methodName}`;
  let index = 0;

  while (index < source.length) {
    const methodIndex = source.indexOf(needle, index);
    if (methodIndex < 0) break;

    let cursor = methodIndex + needle.length;
    while (/\s/.test(source[cursor] ?? "")) cursor += 1;
    if (source[cursor] !== "(") {
      index = cursor;
      continue;
    }

    cursor += 1;
    const start = cursor;
    let quote: "\"" | "'" | null = null;
    let escaped = false;
    let verbatim = false;
    let depth = 1;

    for (; cursor < source.length; cursor += 1) {
      const char = source[cursor] ?? "";
      const next = source[cursor + 1] ?? "";

      if (quote !== null) {
        if (quote === "\"" && verbatim) {
          if (char === "\"" && next === "\"") {
            cursor += 1;
            continue;
          }
          if (char === "\"") quote = null;
          continue;
        }

        if (escaped) {
          escaped = false;
          continue;
        }
        if (char === "\\") {
          escaped = true;
          continue;
        }
        if (char === quote) quote = null;
        continue;
      }

      if (char === "\"" || char === "'") {
        quote = char;
        verbatim = char === "\"" && hasVerbatimStringPrefix(source, cursor);
        escaped = false;
        continue;
      }

      if (char === "(") {
        depth += 1;
        continue;
      }

      if (char === ")") {
        depth -= 1;
        if (depth === 0) {
          calls.push(splitCallArguments(source.slice(start, cursor)));
          cursor += 1;
          break;
        }
      }
    }

    index = cursor;
  }

  return calls;
}

function unescapeRegularCSharpString(value: string): string {
  return value.replace(/\\(u[0-9a-fA-F]{4}|x[0-9a-fA-F]{1,4}|.)/g, (match, escaped: string) => {
    switch (escaped) {
      case "\"": return "\"";
      case "\\": return "\\";
      case "0": return "\0";
      case "a": return "\x07";
      case "b": return "\b";
      case "f": return "\f";
      case "n": return "\n";
      case "r": return "\r";
      case "t": return "\t";
      case "v": return "\v";
      default:
        if (escaped.startsWith("u") || escaped.startsWith("x")) {
          const parsed = Number.parseInt(escaped.slice(1), 16);
          return Number.isFinite(parsed) ? String.fromCharCode(parsed) : match;
        }
        return escaped;
    }
  });
}

function compactExpression(value: string): string {
  const text = value.replace(/\s+/g, " ").trim();
  return text.length <= 120 ? text : `${text.slice(0, 117)}...`;
}

function readCSharpStringArgument(value: string): string {
  const text = value.trim();
  const quoteIndex = text.indexOf("\"");
  if (quoteIndex < 0) return compactExpression(text);

  const prefix = text.slice(0, quoteIndex).trim();
  if (!/^[@$]*$/.test(prefix)) return compactExpression(text);

  const verbatim = prefix.includes("@");
  let content = "";
  let escaped = false;

  for (let i = quoteIndex + 1; i < text.length; i += 1) {
    const char = text[i] ?? "";
    const next = text[i + 1] ?? "";

    if (verbatim) {
      if (char === "\"" && next === "\"") {
        content += "\"";
        i += 1;
        continue;
      }
      if (char === "\"") return content;
      content += char;
      continue;
    }

    if (escaped) {
      content += `\\${char}`;
      escaped = false;
      continue;
    }

    if (char === "\\") {
      escaped = true;
      continue;
    }

    if (char === "\"") return unescapeRegularCSharpString(content);
    content += char;
  }

  return compactExpression(text);
}

function collectRuntimeHints(rawArguments: string): {
  initialState: string;
  prompts: UnityRunStatesRuntimePrompt[];
  prints: UnityRunStatesRuntimePrint[];
} | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawArguments);
  } catch {
    return null;
  }

  if (!isRecord(parsed) || !Array.isArray(parsed.states)) return null;

  const initialState = readString(parsed, "initial_state", "initialState");
  const prompts: UnityRunStatesRuntimePrompt[] = [];
  const prints: UnityRunStatesRuntimePrint[] = [];

  parsed.states.filter(isRecord).forEach((state, stateIndex) => {
    const stateName = readString(state, "name") || `state_${stateIndex + 1}`;
    for (const phase of PHASE_KEYS) {
      const code = readString(state, phase);
      if (!code) continue;

      for (const args of extractCtxCalls(code, "PromptUser")) {
        const token = args[0] ? readCSharpStringArgument(args[0]) : "";
        const message = args[1] ? readCSharpStringArgument(args[1]) : "";
        if (message) prompts.push({ token, message, stateName, phase });
      }

      for (const args of extractCtxCalls(code, "Print")) {
        const value = args[0] ? readCSharpStringArgument(args[0]) : "";
        if (value) prints.push({ value, stateName, phase });
      }
    }
  });

  return { initialState, prompts, prints };
}

function prettifyOutputKey(key: string): string {
  return key.replace(/_/g, " ").trim().toLowerCase();
}

export function parseUnityRunStatesOutput(rawOutput: string): UnityRunStatesOutputPreview | null {
  const text = rawOutput.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim();
  if (!text) return null;

  const fields: UnityRunStatesOutputField[] = [];
  const printLines: string[] = [];
  let readingPrints = false;

  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (readingPrints) {
      printLines.push(line);
      continue;
    }

    if (/^prints:\s*$/i.test(trimmed)) {
      readingPrints = true;
      continue;
    }

    const match = trimmed.match(/^([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$/);
    if (match) {
      fields.push({
        key: match[1],
        label: prettifyOutputKey(match[1]),
        value: match[2] ?? "",
      });
      continue;
    }

    if (trimmed) {
      printLines.push(line);
    }
  }

  const prints = printLines.join("\n").trim();
  if (fields.length === 0 && !prints) return null;

  return {
    fields,
    prints,
  };
}

function outputField(preview: UnityRunStatesOutputPreview | null, key: string): string {
  return preview?.fields.find((field) => field.key === key)?.value ?? "";
}

function joinRuntimeMessages(messages: string[]): string {
  return Array.from(new Set(messages.map((message) => message.trim()).filter(Boolean))).join("\n");
}

export function buildUnityRunStatesRuntimePreview(
  rawArguments: string,
  rawOutput: string | undefined,
  toolStatus: "running" | "done" | "error" | "interrupted",
): UnityRunStatesRuntimePreview | null {
  const hints = collectRuntimeHints(rawArguments);
  if (!hints) return null;

  const outputText = rawOutput ?? "";
  const hasOutput = !!outputText.trim();
  if (toolStatus === "running" && !hasOutput) return null;

  const outputPreview = hasOutput ? parseUnityRunStatesOutput(outputText) : null;
  const finalState = outputField(outputPreview, "final_state");
  const currentState = finalState || hints.initialState;
  const statePrompts = hints.prompts.filter((prompt) => prompt.stateName === currentState);
  const fallbackPrompts = currentState ? statePrompts : hints.prompts;
  const finalPrints = outputPreview?.prints ?? "";
  const printOutput = outputField(outputPreview, "print_output").trim().toLowerCase();
  const printLines = Number.parseInt(outputField(outputPreview, "print_lines"), 10);
  const resultFile = outputField(outputPreview, "result_file");
  const statePrints = hints.prints.filter((item) => item.stateName === currentState);
  const fallbackPrints = currentState ? statePrints : hints.prints;
  const largePrintText =
    printOutput === "too large"
      ? ["too large", Number.isFinite(printLines) ? `${printLines} lines` : "", resultFile]
          .filter(Boolean)
          .join("\n")
      : "";
  const printText =
    largePrintText || finalPrints || joinRuntimeMessages(fallbackPrints.map((item) => item.value));
  const finalStatus = outputField(outputPreview, "status");
  const finalMessage = outputField(outputPreview, "message");
  const printCount =
    printOutput === "too large" && Number.isFinite(printLines)
      ? printLines
      : (printText ? printText.split("\n").filter((line) => line.trim()).length : 0);

  return {
    currentState,
    promptText: joinRuntimeMessages(fallbackPrompts.map((prompt) => prompt.message)),
    printText,
    printCount,
    finalStatus,
    finalMessage,
    isFinal: toolStatus !== "running",
  };
}
