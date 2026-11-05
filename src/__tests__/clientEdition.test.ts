import { describe, expect, it } from 'vitest';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
// @ts-expect-error Native build helper is intentionally shared with Node/Bun.
import { clientEdition, assertPublicArguments, assertClientBundle } from '../../scripts/client-edition.mjs';

describe('client edition release boundary', () => {
  it('defaults to public and ignores unrelated frontend environment values', () => {
    expect(clientEdition({})).toBe('public');
    expect(clientEdition({ VITE_LOCUS_SERVER_CLIENT: 'true' })).toBe('public');
    expect(() => clientEdition({ LOCUS_BUILD_EDITION: 'typo' })).toThrow();
  });
  it.each([
    ['--features', 'server-client'], ['--features=server-client'], ['--all-features'],
    ['--config', 'src-tauri/tauri.internal.conf.json'],
  ])('rejects internal feature flags in release args %j', (...args) => {
    expect(() => assertPublicArguments(args, {})).toThrow();
  });
  it('rejects inherited internal build state', () => {
    expect(() => assertPublicArguments([], { LOCUS_BUILD_EDITION: 'internal' })).toThrow();
    expect(() => assertPublicArguments([], { CARGO_FEATURE_SERVER_CLIENT: '1' })).toThrow();
    expect(() => assertPublicArguments(['--no-default-features'], {})).not.toThrow();
  });
  it.each(['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS'])(
    'rejects feature injection through %s while retaining ordinary compiler tuning', (name) => {
      expect(() => assertPublicArguments([], { [name]: '--cfg\u001ffeature="server-client"' })).toThrow();
      expect(() => assertPublicArguments([], { [name.toLowerCase()]: '--cfg\u001ffeature="server-client"' })).toThrow();
      expect(() => assertPublicArguments([], { [name]: '-C target-cpu=native' })).not.toThrow();
    });
  it('inspects actual artifacts and rejects stale/mismatched outputs', () => {
    const root = mkdtempSync(path.join(tmpdir(), 'locus-edition-test-'));
    const dist = path.join(root, 'dist');
    mkdirSync(dist);
    const marker = (edition: string, files: string[]) => writeFileSync(path.join(dist, '.locus-edition.json'), JSON.stringify({ edition, files }));
    try {
      writeFileSync(path.join(root, 'overlay.json'), JSON.stringify({ build: { frontendDist: '../dist-internal' } }));
      expect(() => assertPublicArguments(['--config', 'overlay.json'], {}, root)).toThrow('Public configuration');
      expect(() => assertPublicArguments([], { TAURI_CONFIG: '{"build":{"frontendDist":"../other"}}' }, root)).toThrow('Public configuration');
      marker('public', ['app.js']);
      writeFileSync(path.join(dist, 'app.js'), 'export const ready = true;');
      expect(() => assertClientBundle(root)).not.toThrow();
      writeFileSync(path.join(dist, 'app.js'), 'invoke("server_client_login", credentials);');
      expect(() => assertClientBundle(root)).toThrow('Internal client');
      marker('internal', ['app.js']);
      expect(() => assertClientBundle(root)).toThrow('mismatch');
      marker('public', []);
      expect(() => assertClientBundle(root)).toThrow('Untracked');
    } finally {
      const relative = path.relative(path.resolve(tmpdir()), path.resolve(root));
      if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('Invalid temporary test path');
      rmSync(root, { recursive: true, force: true });
    }
  });
});
