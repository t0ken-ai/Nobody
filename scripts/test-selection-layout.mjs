/** Compile the production macOS adapter and append pure paragraph/UTF-16 tests
 * in an ignored scratch file. This performs no accessibility or UI operations,
 * and retains the same optional project-local Swift SDK as the app build. */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

if (process.platform !== 'darwin') throw new Error('The native layout checks require macOS.');
const root = fileURLToPath(new URL('../', import.meta.url));
const scratch = join(root, 'artifacts/selection-layout-tests');
mkdirSync(scratch, { recursive: true });
const source = join(scratch, 'main.swift');
const binary = join(scratch, 'selection-layout-tests');
writeFileSync(source, [
  'native/macos/Native.swift', 'native/macos/SelectionLayoutTests.swift',
].map(path => readFileSync(join(root, path), 'utf8')).join('\n'));
const env = { ...process.env };
const local = join(root, 'artifacts/toolchain-downloads/executables/Payload/Library/Developer/CommandLineTools');
if (env.TRANSLATEME_SWIFT_DEVELOPER_DIR) env.DEVELOPER_DIR = env.TRANSLATEME_SWIFT_DEVELOPER_DIR;
else if (existsSync(local)) env.DEVELOPER_DIR = local;
const target = process.arch === 'arm64' ? 'arm64-apple-macosx13.0' : 'x86_64-apple-macosx13.0';
for (const [command, args] of [
  ['xcrun', ['swiftc', '-target', target, source, '-o', binary]],
  [binary, []],
]) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
