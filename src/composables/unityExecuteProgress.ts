export interface UnityExecuteProgressEvent {
  active: boolean;
  title: string;
  info: string;
  progress: number;
  revision: number;
}

export interface UnityExecuteProgressPreview {
  progress: UnityExecuteProgressEvent | null;
  displayOutput: string;
}

export const UNITY_EXECUTE_PROGRESS_TAG = "locus-unity-progress";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function parseProgressPayload(value: string): UnityExecuteProgressEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    return null;
  }

  if (!isRecord(parsed)) return null;

  return {
    active: parsed.active !== false,
    title: typeof parsed.title === "string" ? parsed.title : "",
    info: typeof parsed.info === "string" ? parsed.info : "",
    progress: clampProgress(typeof parsed.progress === "number" ? parsed.progress : 0),
    revision: typeof parsed.revision === "number" ? parsed.revision : 0,
  };
}

export function parseUnityExecuteProgressOutput(rawOutput: string | undefined): UnityExecuteProgressPreview {
  if (!rawOutput) {
    return {
      progress: null,
      displayOutput: "",
    };
  }

  const openTag = `<${UNITY_EXECUTE_PROGRESS_TAG}>`;
  const closeTag = `</${UNITY_EXECUTE_PROGRESS_TAG}>`;
  const outputLines: string[] = [];
  let progress: UnityExecuteProgressEvent | null = null;

  for (const line of rawOutput.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n")) {
    const trimmed = line.trim();
    if (trimmed.startsWith(openTag) && trimmed.endsWith(closeTag)) {
      const payload = trimmed.slice(openTag.length, trimmed.length - closeTag.length);
      const nextProgress = parseProgressPayload(payload);
      if (nextProgress) progress = nextProgress.active ? nextProgress : null;
      continue;
    }

    outputLines.push(line);
  }

  return {
    progress,
    displayOutput: outputLines.join("\n").trim(),
  };
}

export function formatUnityExecuteProgressPercent(progress: UnityExecuteProgressEvent): string {
  return `${Math.round(clampProgress(progress.progress) * 100)}%`;
}
