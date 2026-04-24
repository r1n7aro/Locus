import type { GitConfigEntry } from "../../types";

export function isSafeGitConfigKey(value: string): boolean {
  const key = value.trim();
  if (!key || key.startsWith("-") || !key.includes(".")) return false;
  return [...key].every((char) =>
    /[A-Za-z0-9]/.test(char) || char === "." || char === "-" || char === "_" || char === "/" || char === ":",
  );
}

export function normalizeGitConfigEntries(entries: GitConfigEntry[]): GitConfigEntry[] {
  return entries.map((entry) => ({
    key: entry.key.trim(),
    value: entry.value,
  }));
}

export function gitConfigEntriesSignature(entries: GitConfigEntry[]): string {
  return JSON.stringify(normalizeGitConfigEntries(entries));
}
