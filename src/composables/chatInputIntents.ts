import type { ChatMessage, SkillIntentItem, UserIntentMeta } from "../types";

export type IntentMode = "build" | "plan";
export type IntentCommandKind = "action" | "intent";
export type IntentCommandType = "plan" | "skill" | "compact" | "clear";

export interface ComposerIntentState {
  mode: IntentMode;
  skills: SkillIntentItem[];
}

export interface CommandDef {
  name: string;
  description: string;
  commandKind: IntentCommandKind;
  commandType: IntentCommandType;
  agentOnly?: string;
  skill?: SkillIntentItem;
  argumentHint?: string;
  execute?: (...args: any[]) => void;
  transformSend?: (input: string) => { displayText: string; actualText: string; mode?: string } | null;
}

export interface ActiveOperator {
  kind: "slash" | "mention";
  start: number;
  end: number;
  token: string;
  query: string;
}

export interface InlineIntentParseResult {
  cleanedText: string;
  intent: ComposerIntentState;
  blockedCommand: CommandDef | null;
}

const TOKEN_BOUNDARY_RE = /[\s,，。！？!?:：;；()[\]{}<>《》「」『』"“”'‘’]/;
const SLASH_COMMAND_CHAR_RE = /[A-Za-z0-9_-]/;
const EMAIL_LOCAL_RE = /[A-Za-z0-9._%+-]/;
const EMAIL_DOMAIN_RE = /[A-Za-z0-9.-]/;

function isTokenBoundary(char: string | undefined): boolean {
  return !char || TOKEN_BOUNDARY_RE.test(char);
}

function isSlashCommandChar(char: string | undefined): boolean {
  return !!char && SLASH_COMMAND_CHAR_RE.test(char);
}

function detectSlashOperator(text: string, safeCursor: number): ActiveOperator | null {
  let queryStart = safeCursor;
  while (queryStart > 0 && isSlashCommandChar(text[queryStart - 1])) {
    queryStart -= 1;
  }

  const slashIndex = queryStart - 1;
  if (slashIndex < 0 || text[slashIndex] !== "/" || !isTokenBoundary(text[slashIndex - 1])) {
    return null;
  }

  if (text[slashIndex + 1] === "/") return null;
  if (text[slashIndex - 1] === ":") return null;

  return {
    kind: "slash",
    start: slashIndex,
    end: safeCursor,
    token: text.slice(slashIndex, safeCursor),
    query: text.slice(queryStart, safeCursor),
  };
}

function looksLikeEmailMention(text: string, atIndex: number): boolean {
  let localStart = atIndex;
  while (localStart > 0 && EMAIL_LOCAL_RE.test(text[localStart - 1])) {
    localStart -= 1;
  }

  let domainEnd = atIndex + 1;
  while (domainEnd < text.length && EMAIL_DOMAIN_RE.test(text[domainEnd])) {
    domainEnd += 1;
  }

  const local = text.slice(localStart, atIndex);
  const domain = text.slice(atIndex + 1, domainEnd);
  return !!local && domain.includes(".");
}

function detectMentionOperator(text: string, safeCursor: number): ActiveOperator | null {
  let start = -1;
  for (let index = safeCursor - 1; index >= 0; index -= 1) {
    const char = text[index];
    if (char === "@") {
      start = index;
      break;
    }
    if (isTokenBoundary(char)) {
      break;
    }
  }

  if (start < 0) return null;
  if (looksLikeEmailMention(text, start)) return null;

  return {
    kind: "mention",
    start,
    end: safeCursor,
    token: text.slice(start, safeCursor),
    query: text.slice(start + 1, safeCursor),
  };
}

function isIntentMeta(value: unknown): value is UserIntentMeta {
  if (!value || typeof value !== "object") return false;
  const meta = value as Partial<UserIntentMeta>;
  if (meta.kind !== "user_intent_v1") return false;
  if (meta.mode !== "build" && meta.mode !== "plan") return false;
  if (!Array.isArray(meta.skills)) return false;
  return meta.skills.every((skill) =>
    !!skill
    && typeof skill === "object"
    && typeof (skill as SkillIntentItem).dirName === "string"
    && typeof (skill as SkillIntentItem).name === "string"
    && ((skill as SkillIntentItem).source === "app" || (skill as SkillIntentItem).source === "project"),
  );
}

export function emptyComposerIntent(): ComposerIntentState {
  return { mode: "build", skills: [] };
}

export function hasComposerIntent(intent: ComposerIntentState): boolean {
  return intent.mode === "plan" || intent.skills.length > 0;
}

export function dedupeSkillIntents(skills: SkillIntentItem[]): SkillIntentItem[] {
  const seen = new Set<string>();
  const deduped: SkillIntentItem[] = [];
  for (const skill of skills) {
    const key = `${skill.source}:${skill.dirName}`;
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(skill);
  }
  return deduped;
}

export function mergeComposerIntent(
  base: ComposerIntentState,
  next: Partial<ComposerIntentState>,
): ComposerIntentState {
  return {
    mode: next.mode === "plan" || base.mode === "plan" ? "plan" : "build",
    skills: dedupeSkillIntents([...(base.skills || []), ...(next.skills || [])]),
  };
}

export function buildUserIntentMeta(intent: ComposerIntentState): UserIntentMeta | null {
  const skills = dedupeSkillIntents(intent.skills);
  if (intent.mode !== "plan" && skills.length === 0) return null;
  return {
    kind: "user_intent_v1",
    mode: intent.mode,
    skills,
  };
}

export function parseUserIntentMeta(raw: string | undefined | null): UserIntentMeta | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return isIntentMeta(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function hydrateChatMessageIntent(message: ChatMessage): ChatMessage {
  const intentMeta = parseUserIntentMeta(message.thinkingSignature);
  if (!intentMeta) return message;
  return {
    ...message,
    intentMeta,
  };
}

export function hydrateChatMessagesIntent(messages: ChatMessage[]): ChatMessage[] {
  return messages.map(hydrateChatMessageIntent);
}

export function detectActiveOperator(text: string, cursor: number): ActiveOperator | null {
  const safeCursor = Math.max(0, Math.min(cursor, text.length));
  return detectMentionOperator(text, safeCursor) ?? detectSlashOperator(text, safeCursor);
}

export function removeTextRange(text: string, start: number, end: number): string {
  let before = text.slice(0, start);
  let after = text.slice(end);

  if (before.endsWith(" ") && after.startsWith(" ")) {
    after = after.slice(1);
  }

  return before + after;
}

export function normalizeComposerText(text: string): string {
  return text
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

export function replaceTextRange(text: string, start: number, end: number, replacement: string): string {
  return text.slice(0, start) + replacement + text.slice(end);
}

export function formatInlineMention(relPath: string): string {
  return `\`${relPath}\``;
}

export function insertInlineMention(
  text: string,
  start: number,
  end: number,
  relPath: string,
): { text: string; cursor: number } {
  const safeStart = Math.max(0, Math.min(start, text.length));
  const safeEnd = Math.max(safeStart, Math.min(end, text.length));
  const before = text.slice(0, safeStart);
  const after = text.slice(safeEnd);
  const mention = formatInlineMention(relPath);
  const leadingSpace = before && !isTokenBoundary(before[before.length - 1]) ? " " : "";
  const trailingSpace = after && !isTokenBoundary(after[0]) ? " " : "";
  const nextText = before + leadingSpace + mention + trailingSpace + after;

  return {
    text: nextText,
    cursor: before.length + leadingSpace.length + mention.length + trailingSpace.length,
  };
}

function exactCommandForToken(commands: CommandDef[], token: string): CommandDef | null {
  const normalized = token.toLowerCase();
  return commands.find((cmd) => cmd.name.toLowerCase() === normalized) ?? null;
}

export function parseInlineIntentCommands(
  text: string,
  commands: CommandDef[],
  selectedAgentId: string,
): InlineIntentParseResult {
  const removalRanges: Array<{ start: number; end: number }> = [];
  const intent = emptyComposerIntent();

  for (let index = 0; index < text.length; index += 1) {
    if (text[index] !== "/" || !isTokenBoundary(text[index - 1])) continue;

    let end = index + 1;
    while (end < text.length && !isTokenBoundary(text[end])) {
      end += 1;
    }

    const token = text.slice(index, end);
    const command = exactCommandForToken(commands, token);
    if (!command || command.commandKind !== "intent") {
      index = end - 1;
      continue;
    }

    if (command.agentOnly && command.agentOnly !== selectedAgentId) {
      return {
        cleanedText: text,
        intent,
        blockedCommand: command,
      };
    }

    removalRanges.push({ start: index, end });
    if (command.commandType === "plan") {
      intent.mode = "plan";
    } else if (command.commandType === "skill" && command.skill) {
      intent.skills = dedupeSkillIntents([...intent.skills, command.skill]);
    }

    index = end - 1;
  }

  if (removalRanges.length === 0) {
    return {
      cleanedText: text.trim(),
      intent,
      blockedCommand: null,
    };
  }

  let cleaned = text;
  for (let i = removalRanges.length - 1; i >= 0; i -= 1) {
    const range = removalRanges[i];
    cleaned = removeTextRange(cleaned, range.start, range.end);
  }

  return {
    cleanedText: normalizeComposerText(cleaned),
    intent,
    blockedCommand: null,
  };
}

