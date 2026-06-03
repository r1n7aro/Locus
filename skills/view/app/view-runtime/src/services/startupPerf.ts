const startupOriginMs =
  typeof performance !== "undefined" && Number.isFinite(performance.timeOrigin)
    ? performance.timeOrigin
    : Date.now();

let lastMarkMs =
  typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : 0;

function nowMs(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now() - startupOriginMs;
}

function roundMs(value: number): number {
  return Math.round(value);
}

function formatDetail(detail?: Record<string, unknown>): string {
  if (!detail) return "";
  const parts = Object.entries(detail)
    .filter(([, value]) => value !== undefined && value !== null && value !== "")
    .map(([key, value]) => `${key}=${String(value)}`);
  return parts.length ? ` ${parts.join(" ")}` : "";
}

export function markStartupPhase(
  phase: string,
  detail?: Record<string, unknown>,
): void {
  const currentMs = nowMs();
  const deltaMs = currentMs - lastMarkMs;
  lastMarkMs = currentMs;
  console.info(
    `[startup] phase=${phase} total=${roundMs(currentMs)}ms delta=${roundMs(deltaMs)}ms timeOrigin=${new Date(startupOriginMs).toISOString()}${formatDetail(detail)}`,
  );
}

export async function measureStartupAsync<T>(
  phase: string,
  task: () => Promise<T>,
): Promise<T> {
  markStartupPhase(`${phase}_start`);
  try {
    return await task();
  } finally {
    markStartupPhase(`${phase}_done`);
  }
}

function readNavigationTiming(): Record<string, unknown> {
  const navigation = performance.getEntriesByType("navigation")[0] as
    | PerformanceNavigationTiming
    | undefined;
  if (!navigation) return {};
  return {
    domInteractive: `${roundMs(navigation.domInteractive)}ms`,
    domContentLoaded: `${roundMs(navigation.domContentLoadedEventEnd)}ms`,
    loadEventEnd: `${roundMs(navigation.loadEventEnd)}ms`,
  };
}

function readPaintTiming(): Record<string, unknown> {
  const paints = performance.getEntriesByType("paint");
  const firstPaint = paints.find((entry) => entry.name === "first-paint");
  const firstContentfulPaint = paints.find(
    (entry) => entry.name === "first-contentful-paint",
  );
  return {
    firstPaint: firstPaint ? `${roundMs(firstPaint.startTime)}ms` : "missing",
    fcp: firstContentfulPaint
      ? `${roundMs(firstContentfulPaint.startTime)}ms`
      : "missing",
  };
}

export function scheduleStartupPaintReport(): void {
  const report = (phase: string) => {
    markStartupPhase(phase, {
      ...readPaintTiming(),
      ...readNavigationTiming(),
    });
  };

  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => {
      setTimeout(() => report("frontend_paint_after_mount"), 0);
    });
  } else {
    setTimeout(() => report("frontend_paint_after_mount"), 0);
  }

  setTimeout(() => report("frontend_paint_late"), 1500);
}
