import { t } from '../i18n';
import type { ClaudeConnectionDiagnostics, ClaudeTransportSnapshot } from '../services/claudeConnection';

interface DiagnosticRow { label: string; value: string; tier?: 'diagnostic'; mono?: boolean; multiline?: boolean }
const key = 'chat.status.claude.diagnostics.';
const stages = new Set(['ssh', 'remoteShell', 'pythonReady', 'environment', 'tui', 'historyLock', 'historyRead', 'historySerialize', 'historyTransfer', 'ready']);
const stage = (value: string) => stages.has(value) ? t(`${key}stage.${value}`) : value;
const seconds = (ms: number) => `${(ms / 1000).toFixed(1)} s`;
const age = (ms: number | null) => ms == null ? '—' : seconds(ms);
function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
}
function progress(trace: ClaudeTransportSnapshot): string {
  const rate = trace.stageBytesPerSecond == null ? '—' : `${bytes(trace.stageBytesPerSecond)}/s`;
  const received = trace.expectedFrameBytes == null ? trace.partialFrameBytes
    : Math.min(trace.stageReceivedBytes, trace.expectedFrameBytes);
  return `${bytes(received)} / ${trace.expectedFrameBytes == null ? '—' : bytes(trace.expectedFrameBytes)} · ${rate}`;
}
function timeline(trace: ClaudeTransportSnapshot): string {
  return trace.timeline.map(item => `${seconds(item.elapsedMs)}  ${stage(item.stage)}`).join('\n');
}

function helperDetails(trace: ClaudeTransportSnapshot): string | null {
  const helper = trace.helper;
  if (!helper) return null;
  return [
    `v${helper.helperVersion} · SHA-256 ${helper.sourceHash ?? '—'}`,
    `Python PID ${helper.hostPid} · Claude PID ${helper.claudePid ?? '—'} · ${helper.event}`,
    helper.exitReason == null ? null : `${helper.exitReason} · exit ${helper.exitCode ?? '—'} · Claude exit ${helper.claudeExitCode ?? '—'}`,
    ...['errorType', 'reportingError', 'cleanupError'].flatMap(field => {
      const value = helper[field as 'errorType' | 'reportingError' | 'cleanupError'];
      return value == null ? [] : [`${field}: ${value}`];
    }),
  ].filter(value => value != null).join('\n');
}

// Formatting only: no IPC, timers, network checks or side effects on opening the
// panel. The existing shared poll supplies both passive and failure-only data.
export function claudeDiagnosticRows(diagnostics: ClaudeConnectionDiagnostics | undefined): DiagnosticRow[] {
  if (!diagnostics) return [];
  const rows: DiagnosticRow[] = [];
  const row = (label: string, value: string, main = false) => rows.push({
    label: t(`${key}${label}`), value, tier: main ? undefined : 'diagnostic', multiline: true,
  });
  const active = diagnostics.active;
  if (active) {
    row('heartbeat', active.heartbeatAgeMs == null ? t(`${key}waitingHeartbeat`)
      : t(`${key}${active.heartbeatAgeMs <= 15_000 ? 'responsive' : 'stale'}`, age(active.heartbeatAgeMs)), true);
    row('operation', `#${active.id} · ${active.operation} · PID ${active.pid ?? '—'}`);
    const helper = helperDetails(active);
    if (helper) row('helper', helper);
    row('stage', `${stage(active.stage)} · ${seconds(active.stageElapsedMs)}`);
    row('traffic', `↓ ${bytes(active.receivedBytes)} · ↑ ${bytes(active.sentBytes)}`);
    row('lastReceive', age(active.lastReceiveAgeMs));
    row('pendingHeartbeats', String(active.pendingHeartbeats));
    if (active.partialFrameBytes || active.expectedFrameBytes != null) row('frame', progress(active));
    row('timeline', timeline(active));
  }
  const failure = diagnostics.lastFailure;
  if (failure) {
    const trace = failure.transport;
    row('lastFailure', `${new Date(failure.at).toLocaleString()} · #${trace.id} · ${stage(trace.stage)}`, true);
    row('failure', `${failure.kind} · ${trace.operation} · ${seconds(trace.elapsedMs)} · exit ${failure.exitCode ?? '—'}\n${failure.message}`);
    row('failureProgress', `${progress(trace)}\n${t(`${key}lastReceive`)}: ${age(trace.lastReceiveAgeMs)}`);
    row('failureTimeline', timeline(trace));
    const helper = helperDetails(trace);
    if (helper) row('failureHelper', helper);
    if (failure.stderr) row('stderr', failure.stderr);
  }
  const probe = diagnostics.probe;
  if (probe) {
    const states = new Set(['running', 'complete', 'busy', 'timedOut']);
    row('probe', `${new Date(probe.at).toLocaleString()} · #${probe.failureId} · ${states.has(probe.state) ? t(`${key}probe.${probe.state}`) : probe.state}`);
    if (probe.checks.length) row('probeResults', probe.checks.map(check =>
      `${check.name}: ${check.outcome} (${seconds(check.elapsedMs)})\n${check.detail}`).join('\n\n'));
  }
  if (active || failure || probe) row('policy', t(`${key}policyHint`));
  return rows;
}
