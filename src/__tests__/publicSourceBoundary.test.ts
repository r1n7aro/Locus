import { afterEach, describe, expect, it } from 'vitest';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { assertPublicSourceBoundary } from '../../scripts/public-source-boundary.mjs';

const roots: string[] = [];
function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), 'locus-public-boundary-'));
  roots.push(root);
  expect(spawnSync('git', ['init', '--quiet'], { cwd: root }).status).toBe(0);
  mkdirSync(path.join(root, 'src-tauri'));
  writeFileSync(path.join(root, '.gitignore'), '/crates/locus-server/\n');
  writeFileSync(path.join(root, 'package.json'), '{"name":"locus"}');
  writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[package]\nname="locus"\n');
  return root;
}
afterEach(() => {
  for (const root of roots.splice(0)) {
    const absolute = path.resolve(root);
    if (path.dirname(absolute) !== path.resolve(tmpdir()) || !path.basename(absolute).startsWith('locus-public-boundary-')) {
      throw new Error('Refusing to clean a directory outside this test fixture');
    }
    rmSync(absolute, { recursive: true, force: true });
  }
});
describe('public source boundary', () => {
  it('allows a clean public checkout without the private crate', () => {
    expect(() => assertPublicSourceBoundary(fixture())).not.toThrow();
  });
  it('rejects forcibly tracked private sources even when the ignore rule exists', () => {
    const root = fixture();
    mkdirSync(path.join(root, 'crates', 'locus-server'), { recursive: true });
    writeFileSync(path.join(root, 'crates', 'locus-server', 'Cargo.toml'), '[package]\nname="private"');
    expect(spawnSync('git', ['add', '-f', '--', 'crates/locus-server/Cargo.toml'], { cwd: root }).status).toBe(0);
    expect(() => assertPublicSourceBoundary(root)).toThrow(/tracked/);
  });
  it('rejects public manifests with a private path dependency', () => {
    const root = fixture();
    writeFileSync(path.join(root, 'src-tauri', 'Cargo.toml'), '[dependencies]\nprivate={path="../crates/locus-server"}\n');
    expect(() => assertPublicSourceBoundary(root)).toThrow(/manifest references/);
  });
  it('rejects a private gitlink in the public index', () => {
    const root = fixture();
    const hash = 'a'.repeat(40);
    expect(spawnSync('git', ['update-index', '--add', '--cacheinfo', `160000,${hash},crates/locus-server`], { cwd: root }).status).toBe(0);
    expect(() => assertPublicSourceBoundary(root)).toThrow(/gitlink/);
  });
  it('rejects broad resource globs but permits the explicitly shared core', () => {
    const root = fixture();
    const config = path.join(root, 'src-tauri', 'tauri.conf.json');
    writeFileSync(config, JSON.stringify({ bundle: { resources: ['../crates/**/*'] } }));
    expect(() => assertPublicSourceBoundary(root)).toThrow(/resource includes/);
    writeFileSync(config, JSON.stringify({ bundle: { resources: ['../crates/locus-remote-core/resources/**/*'] } }));
    expect(() => assertPublicSourceBoundary(root)).not.toThrow();
  });
  it('fails closed if the private directory ignore rule is removed', () => {
    const root = fixture();
    writeFileSync(path.join(root, '.gitignore'), '');
    expect(() => assertPublicSourceBoundary(root)).toThrow(/must be excluded/);
  });
});
