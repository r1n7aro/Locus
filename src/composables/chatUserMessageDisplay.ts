const SYSTEM_REMINDER_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<system-reminder>[\s\S]*?<\/system-reminder>[ \t]*(?:\r?\n)?/gi;

const UNITY_ASSET_REFS_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<unity-asset-refs>[\s\S]*?<\/unity-asset-refs>[ \t]*(?:\r?\n)?/gi;

const LOCUS_REFERENCES_BLOCK_RE =
  /(?:^|\r?\n)[ \t]*<locus-references>[\s\S]*?<\/locus-references>[ \t]*(?:\r?\n)?/gi;

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

function stripUnityAssetRefBlocks(text: string) {
  return text
    .replace(UNITY_ASSET_REFS_BLOCK_RE, "\n")
    .replace(LOCUS_REFERENCES_BLOCK_RE, "\n");
}

function stripKnownLocusPrefixes(text: string) {
  return text.replace(UNITY_EDITOR_STATUS_CHANGED_PREFIX_RE, "");
}

export function displayUserMessageContent(content: string) {
  let next = content;
  let previous = "";

  while (next !== previous) {
    previous = next;
    next = stripKnownLocusPrefixes(stripUnityAssetRefBlocks(stripSystemReminderBlocks(next)));
    next = trimInjectedPadding(next);
  }

  return next;
}
