import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (path: string) => readFileSync(resolve(process.cwd(), path), 'utf8');

describe('shared Unity profiler API', () => {
  it('makes profiling discoverable through both execution tools', () => {
    const skill = read('knowledge/skill/profiler.md');
    expect(skill).toContain('  - unity_execute');
    expect(skill).toContain('  - unity_run_states');
    expect(skill).toContain('ctx.Profiler');
    expect(JSON.parse(read('tools/unity_execute.json')).description).toContain('ctx.Profiler');
    expect(JSON.parse(read('tools/unity_run_states.json')).description).toContain('ctx.Profiler');
    for (const method of ['DiscoverMetrics', 'ProfilerMetrics', 'GetProfilerSamples', 'CompareProfilers', 'GetProfilerBudget', 'SaveMemorySnapshotAsync']) {
      expect(skill).toContain(method);
    }
  });

  it.skipIf(!process.env.LOCUS_PROFILER_UNITY_EDITOR)('runs native Unity capture and lifecycle checks in an isolated project', () => {
    const output = execFileSync('pwsh', [
      '-NoProfile', '-File', resolve('scripts/locus-profiler-check.ps1'),
      '-UnityEditor', process.env.LOCUS_PROFILER_UNITY_EDITOR!,
      '-OutputRoot', process.env.LOCUS_PROFILER_OUTPUT_ROOT || process.env.TEMP || '.',
    ], { encoding: 'utf8', timeout: 240_000, maxBuffer: 4 * 1024 * 1024 });
    const match = output.match(/LOCUS_PROFILER_TEST_JSON (\{[^\r\n]+\})/);
    expect(match, output).not.toBeNull();
    const result = JSON.parse(match![1]);
    expect(result.ok).toBe(true);
    expect(result.checks).toBeGreaterThanOrEqual(40);
  }, 250_000);
});
