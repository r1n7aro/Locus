const SYSTEM_REMINDER_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<system-reminder>[\s\S]*?<\/system-reminder>[ \t]*(?:\r?\n)?/gi;

const UNITY_ASSET_REFS_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<unity-asset-refs>[\s\S]*?<\/unity-asset-refs>[ \t]*(?:\r?\n)?/gi;

const LOCUS_REFERENCES_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<locus-references>[\s\S]*?<\/locus-references>[ \t]*(?:\r?\n)?/gi;

const LOCUS_CONSOLE_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<locus-console>([\s\S]*?)<\/locus-console>[ \t]*(?:\r?\n)?/gi;

const UNITY_EDITOR_STATUS_CHANGED_PREFIX_RE =
  /^[ \t]*\[Unity Editor Status Changed\][^\r\n]*(?:\r?\n[ \t]*){0,2}/;

export interface UserConsoleEntryDisplay {
  title: string;
  level: "Error" | "Warning" | "Log";
  source: string;
  chars: number;
  text: string;
}

function trimInjectedPadding(text: string) {
  return text
    .replace(/^(?:[ \t]*\r?\n)+/, "")
    .replace(/(?:\r?\n[ \t]*)+$/, "");
}

function stripSystemReminderBlocks(text: string) {
  return text.replace(SYSTEM_REMINDER_BLOCK_RE, "\n");
}

function stripUnityAssetRefBlocks(text: string) {
  return text
    .replace(UNITY_ASSET_REFS_BLOCK_RE, "\n")
    .replace(LOCUS_REFERENCES_BLOCK_RE, "\n");
}

function stripLocusConsoleBlocks(text: string) {
  return text.replace(LOCUS_CONSOLE_BLOCK_RE, "\n");
}

function stripKnownLocusPrefixes(text: string) {
  return text.replace(UNITY_EDITOR_STATUS_CHANGED_PREFIX_RE, "");
}

function normalizeConsoleLevel(title: string, text: string): UserConsoleEntryDisplay["level"] {
  const value = `${title}\n${text}`.toLowerCase();
  if (value.includes("[warning]") || value.includes("warning")) return "Warning";
  if (
    value.includes("[error]")
    || value.includes("exception")
    || value.includes("assert")
    || value.includes("fatal")
  ) {
    return "Error";
  }
  return "Log";
}

function parseConsoleEntryChunk(chunk: string): UserConsoleEntryDisplay | null {
  const headerIndex = chunk.search(/^## Entry\s+\d+:/m);
  if (headerIndex < 0) return null;

  const entryText = chunk.slice(headerIndex).trim();
  const titleMatch = entryText.match(/^## Entry\s+\d+:\s*(.+)$/m);
  const title = titleMatch?.[1]?.trim() || "Unity Console";
  const source = entryText.match(/^Source:\s*(.+)$/m)?.[1]?.trim() || "unity-console";
  const charsValue = Number.parseInt(entryText.match(/^Chars:\s*(\d+)$/m)?.[1] ?? "", 10);
  const bodyStart = entryText.search(/\r?\n\r?\n/);
  const text = bodyStart >= 0 ? entryText.slice(bodyStart).trim() : "";
  return {
    title,
    level: normalizeConsoleLevel(title, text),
    source,
    chars: Number.isFinite(charsValue) ? charsValue : text.length,
    text,
  };
}

function parseConsoleBlock(block: string): UserConsoleEntryDisplay[] {
  return block
    .split(/(?:^|\r?\n)---(?:\r?\n|$)/)
    .map(parseConsoleEntryChunk)
    .filter((entry): entry is UserConsoleEntryDisplay => !!entry);
}

export function userMessageConsoleEntries(content: string): UserConsoleEntryDisplay[] {
  const entries: UserConsoleEntryDisplay[] = [];
  for (const match of content.matchAll(LOCUS_CONSOLE_BLOCK_RE)) {
    entries.push(...parseConsoleBlock(match[1] ?? ""));
  }
  return entries;
}

export function displayUserMessageContent(content: string) {
  let next = content;
  let previous = "";

  while (next !== previous) {
    previous = next;
    next = stripKnownLocusPrefixes(stripLocusConsoleBlocks(stripUnityAssetRefBlocks(stripSystemReminderBlocks(next))));
    next = trimInjectedPadding(next);
  }

  return next;
}
