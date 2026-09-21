// Source preservation check for the macOS parallel IPC implementation.
// This complements tests; it is not a substitute for Windows/Unity runtime QA.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const baseline = process.argv[2] || 'cf70db9636ee61e74c779bec1472111ff389e00d'
const normalize = (source) => source.replaceAll('\r\n', '\n')
const before = (path) => normalize(execFileSync('git', ['show', `${baseline}:${path}`], { cwd: root, encoding: 'utf8' }))
const after = (path) => normalize(readFileSync(resolve(root, path), 'utf8'))
function block(source, start) {
  const offset = source.indexOf(start)
  assert.ok(offset >= 0, `Missing block ${start}`)
  const opening = source.indexOf('{', offset)
  let depth = 1
  let end = opening + 1
  while (end < source.length && depth) {
    if (source[end] === '{') depth++
    if (source[end] === '}') depth--
    end++
  }
  assert.equal(depth, 0, `Unbalanced block ${start}`)
  return source.slice(offset, end)
}
function verify(path, starts) {
  const old = before(path)
  const current = after(path)
  for (const start of starts) {
    assert.equal(block(current, start), block(old, start), `Windows block changed: ${path} ${start}`)
    console.log(`PASS ${path}: ${start}`)
  }
}
verify('locus_native_plugin/src/lib.rs', ['mod imp {', 'mod hook {', 'mod overlay {', 'fn normalize_pipe_name('])
verify('src-tauri/src/unity_bridge/transport.rs', ['mod windows_impl {'])
verify('src-tauri/src/unity_bridge/mod.rs', ['mod native_state_plane_imp {', 'fn native_pipe_name_part(', 'pub(crate) fn get_native_pipe_name('])
assert.equal(after('src-tauri/src/unity_bridge/transport/requests.rs'), before('src-tauri/src/unity_bridge/transport/requests.rs'))
console.log('PASS Windows request/ACK tracking source unchanged')

// Every existing Windows FFI/dispatch block is retained verbatim.
for (const [path, marker] of [
  ['locus_native_plugin/src/lib.rs', '    #[cfg(windows)]\n    {'],
  ['src-tauri/src/unity_bridge/transport.rs', '#[cfg(target_os = "windows")]\npub '],
]) {
  const collect = (source) => {
    const result = []
    let offset = 0
    while ((offset = source.indexOf(marker, offset)) >= 0) {
      const value = block(source.slice(offset), marker)
      result.push(value)
      offset += value.length
    }
    return result
  }
  assert.deepEqual(collect(after(path)), collect(before(path)), `${path}: Windows dispatch/FFI changed`)
  console.log(`PASS ${path}: Windows dispatch/FFI unchanged`)
}
const managedPath = 'locus_unity/Editor/LocusBridge.Native.cs'
const windowsManaged = after(managedPath).replace(/#if UNITY_EDITOR_OSX\n[\s\S]*?#else\n([\s\S]*?)#endif\n/g, '$1')
assert.equal(windowsManaged, before(managedPath), 'Windows managed native bridge changed after preprocessing')
console.log('PASS Windows managed native bridge unchanged after preprocessing')
