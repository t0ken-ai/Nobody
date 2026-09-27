/** Package the approved Nobody B artwork with Tauri's deterministic resizer.
 * Run manually after an approved artwork change; normal builds use checked-in
 * assets and never call an image service. Only desktop resources are copied.
 * The separate alpha silhouette remains legible at menu-bar size and lets
 * macOS/WebKit color it for the current appearance without polling. */
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const cli = join(root, 'node_modules/@tauri-apps/cli/tauri.js');
const output = join(root, 'artifacts/nobody-icons');
const templateOutput = join(root, 'artifacts/nobody-template');
const icons = join(root, 'src-tauri/icons');
const assets = join(root, 'src/assets');
mkdirSync(assets, { recursive: true });

// Tauri also emits mobile/store artwork. Keep those intermediate exports out
// of the source tree because this app currently ships only desktop bundles.
for (const args of [
  ['icon', 'design/nobody/exports/b-app-icon.png', '-o', output],
  ['icon', 'design/nobody/exports/b-menu-template.png', '-o', templateOutput, '-p', '64'],
]) {
  const result = spawnSync(process.execPath, [cli, ...args], { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
for (const name of ['icon.png', 'icon.icns', 'icon.ico', '32x32.png', '128x128.png', '128x128@2x.png']) {
  copyFileSync(join(output, name), join(icons, name));
}
copyFileSync(join(output, '128x128.png'), join(assets, 'nobody-icon.png'));
copyFileSync(join(templateOutput, '64x64.png'), join(icons, 'tray-template.png'));
copyFileSync(join(templateOutput, '64x64.png'), join(assets, 'nobody-symbol.png'));
