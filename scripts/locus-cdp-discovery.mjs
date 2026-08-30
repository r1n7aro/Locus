const DEFAULT_DEBUG_PORT_START = 19_222;
const DEFAULT_DEBUG_PORT_ATTEMPTS = 25;
const DEFAULT_REQUEST_TIMEOUT_MS = 350;

function normalizeBrowserUrl(value) {
  return typeof value === "string" && value.trim()
    ? value.trim().replace(/\/$/, "")
    : null;
}

export function browserUrlFromArgs(args) {
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--browserUrl") {
      return normalizeBrowserUrl(args[index + 1]);
    }
    if (arg?.startsWith("--browserUrl=")) {
      return normalizeBrowserUrl(arg.slice("--browserUrl=".length));
    }
  }
  return null;
}

export function withBrowserUrlArg(args, browserUrl) {
  const next = [];
  let replaced = false;
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--browserUrl") {
      if (!replaced) {
        next.push("--browserUrl", browserUrl);
        replaced = true;
      }
      index += 1;
      continue;
    }
    if (arg?.startsWith("--browserUrl=")) {
      if (!replaced) {
        next.push(`--browserUrl=${browserUrl}`);
        replaced = true;
      }
      continue;
    }
    next.push(arg);
  }
  if (!replaced) {
    next.push("--browserUrl", browserUrl);
  }
  return next;
}

export function isLocusPageTarget(target) {
  if (!target || target.type !== "page" || !target.webSocketDebuggerUrl) {
    return false;
  }
  return target.url === "http://tauri.localhost/"
    || /^http:\/\/localhost:\d+\/$/.test(target.url ?? "")
    || /^http:\/\/127\.0\.0\.1:\d+\/$/.test(target.url ?? "");
}

async function readLocusTargets(browserUrl, fetchImpl, timeoutMs) {
  try {
    const response = await fetchImpl(`${browserUrl}/json/list`, {
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) return [];
    const targets = await response.json();
    return Array.isArray(targets) ? targets.filter(isLocusPageTarget) : [];
  } catch {
    return [];
  }
}

export async function discoverLocusBrowserUrl({
  preferredUrl = null,
  startPort = DEFAULT_DEBUG_PORT_START,
  attempts = DEFAULT_DEBUG_PORT_ATTEMPTS,
  timeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
  fetchImpl = fetch,
} = {}) {
  const normalizedPreferred = normalizeBrowserUrl(preferredUrl);
  if (
    normalizedPreferred
    && (await readLocusTargets(normalizedPreferred, fetchImpl, timeoutMs)).length > 0
  ) {
    return normalizedPreferred;
  }

  const fallbackCandidates = [];
  const seen = new Set();
  const addCandidate = (value) => {
    const normalized = normalizeBrowserUrl(value);
    if (!normalized || normalized === normalizedPreferred || seen.has(normalized)) return;
    seen.add(normalized);
    fallbackCandidates.push(normalized);
  };

  for (let offset = 0; offset < attempts; offset += 1) {
    addCandidate(`http://127.0.0.1:${startPort + offset}`);
  }

  const discovered = (await Promise.all(fallbackCandidates.map(async (candidate) => (
    (await readLocusTargets(candidate, fetchImpl, timeoutMs)).length > 0 ? candidate : null
  )))).filter(Boolean);
  if (discovered.length === 1) {
    return discovered[0];
  }
  if (discovered.length > 1) {
    throw new Error(
      `Multiple Locus CDP targets are available (${discovered.join(", ")}); `
      + "keep --browserUrl pointed at the intended instance.",
    );
  }
  return normalizedPreferred ?? `http://127.0.0.1:${startPort}`;
}

export async function resolveLocusBrowserArgs(args, options = {}) {
  const preferredUrl = browserUrlFromArgs(args);
  const browserUrl = await discoverLocusBrowserUrl({ ...options, preferredUrl });
  return {
    args: withBrowserUrlArg(args, browserUrl),
    browserUrl,
    preferredUrl,
  };
}
