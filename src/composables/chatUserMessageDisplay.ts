const SYSTEM_REMINDER_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<system-reminder>[\s\S]*?<\/system-reminder>[ \t]*(?:\r?\n)?/gi;

const UNITY_EDITOR_STATUS_CHANGED_PREFIX_RE =
  /^[ \t]*\[Unity Editor Status Changed\][^\r\n]*(?:\r?\n[ \t]*){0,2}/;

function trimInjectedPadding(text: string) {
  return text
    .replace(/^(?:[ \t]*\r?\n)+/, "")
    .replace(/(?:\r?\n[ \t]*)+$/, "");
}

function stripSystemReminderBlocks(text: string) {
  return text.replace(SYSTEM_REMINDER_BLOCK_RE, "\n");
}

function stripKnownLocusPrefixes(text: string) {
  return text.replace(UNITY_EDITOR_STATUS_CHANGED_PREFIX_RE, "");
}

export function displayUserMessageContent(content: string) {
  let next = content;
  let previous = "";

  while (next !== previous) {
    previous = next;
    next = stripKnownLocusPrefixes(stripSystemReminderBlocks(next));
    next = trimInjectedPadding(next);
  }

  return next;
}
