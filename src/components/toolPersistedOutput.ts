export interface PersistedOutputDisplay {
  kind: "normal" | "persisted" | "deleted";
  text: string;
  path?: string;
}

export function persistedOutputDisplay(output: string): PersistedOutputDisplay {
  const deleted = output.match(/^<persisted-output-deleted>\n?([\s\S]*?)\n?<\/persisted-output-deleted>\s*$/);
  if (deleted) {
    const body = deleted[1].trim();
    const path = body
      .split(/\r?\n/)
      .find((line) => line.startsWith("Full output file deleted: "))
      ?.slice("Full output file deleted: ".length)
      .trim();
    return {
      kind: "deleted",
      text: body,
      path: path || undefined,
    };
  }

  const persisted = output.match(/^<persisted-output>\n?([\s\S]*?)\n?<\/persisted-output>\s*$/);
  if (persisted) {
    return {
      kind: "persisted",
      text: persisted[1].trim(),
    };
  }

  return {
    kind: "normal",
    text: output,
  };
}

