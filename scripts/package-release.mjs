/** Build a self-contained macOS Release and package only its signed app plus
 * installation notes. User settings/keys under ~/.translateme are never read.
 * This intentionally retains the configured signing identity: an optimized
 * ad-hoc build must not be presented as Developer ID signed or notarized. */
import { copyFileSync, createReadStream, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { basename, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
if (process.platform !== 'darwin' || process.arch !== 'arm64') {
  throw new Error('This packaging workflow currently targets Apple Silicon macOS only.');
}
const { productName, version } = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const output = join(root, 'artifacts/releases');
const staging = join(root, 'artifacts/release-staging', `${productName}-${version}-macOS-arm64`);
const app = join(root, 'src-tauri/target/release/bundle/macos', `${productName}.app`);
const name = `${productName}-${version}-macOS-arm64`;
mkdirSync(output, { recursive: true });

/** Fail immediately on build/signature/archive errors, before publishing hashes.
 * Arguments are passed directly, never interpolated through a shell. */
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed with status ${result.status}`);
}

run(process.execPath, ['scripts/local-build.mjs', 'build', '--bundles', 'app', '--', '--locked']);
run('codesign', ['--verify', '--deep', '--strict', app]);
// Only this reproducible staging directory is replaced, never user files or
// an installed application. Older versioned release artifacts remain intact.
rmSync(staging, { recursive: true, force: true });
mkdirSync(staging, { recursive: true });
run('ditto', [app, join(staging, `${productName}.app`)]);
copyFileSync(join(root, 'docs/release-macos.md'), join(staging, '安装说明.txt'));
symlinkSync('/Applications', join(staging, 'Applications'));
const zip = join(output, `${name}.zip`);
const dmg = join(output, `${name}.dmg`);
run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', app, zip]);
run('hdiutil', ['create', '-volname', productName, '-srcfolder', staging, '-ov', '-format', 'UDZO', dmg]);
run('hdiutil', ['verify', dmg]);
run('unzip', ['-tq', zip]);

// Stream hashes so packaging memory does not grow with the bundle size.
const sums = [];
for (const path of [dmg, zip]) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  sums.push(`${hash.digest('hex')}  ${basename(path)}`);
}
writeFileSync(join(output, `${name}.sha256`), `${sums.join('\n')}\n`);
console.log(`Release files: ${output}\n${sums.join('\n')}`);
