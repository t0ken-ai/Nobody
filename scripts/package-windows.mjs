/** Build an x64 per-user NSIS installer and its version-bound updater signature.
 * No user profile data is packaged. Authenticode is separate from this update
 * signature and is deliberately not claimed by this release workflow. */
import { createReadStream, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Windows x64 builder required.');
/** Invoke Node entrypoints directly: Windows .cmd wrappers cannot safely be
 * spawned like Unix executables, and no shell interpolation is necessary. */
function run(args) {
  const result = spawnSync(process.execPath, args, { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error('Windows packaging verification failed.');
}
run(['node_modules/@tauri-apps/cli/tauri.js', 'build', '--bundles', 'nsis', '--', '--locked']);
run(['scripts/prepare-updater.mjs']);
run(['scripts/test-updater.mjs', '--release']);
const { version } = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const output = join(root, 'artifacts/releases');
const name = `Nobody-${version}-Windows-x64`;
const files = [`${name}-setup.exe`, `${name}-setup.exe.sig`, 'manifest-windows-x86_64.json'];
const sums = [];
for (const file of files) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(join(output, file))) hash.update(chunk);
  sums.push(`${hash.digest('hex')}  ${file}`);
}
writeFileSync(join(output, `${name}.sha256`), `${sums.join('\n')}\n`);
console.log(`Windows release files: ${output}\n${sums.join('\n')}`);
