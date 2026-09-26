/** Prefer the optional project-local Apple SDK for Swift only. Rust continues
 * using the existing host toolchain; no xcode-select global change is required. */
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
const local = `${root}artifacts/toolchain-downloads/executables/Payload/Library/Developer/CommandLineTools`;
const env = { ...process.env };
if (process.platform === 'darwin' && existsSync(local) && !env.TRANSLATEME_SWIFT_DEVELOPER_DIR) {
  env.TRANSLATEME_SWIFT_DEVELOPER_DIR = local;
}
const args = process.argv.slice(2);
const child = spawn(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'tauri', '--', ...(args.length ? args : ['build', '--debug', '--bundles', 'app'])], { cwd: root, env, stdio: 'inherit' });
child.on('exit', code => process.exit(code ?? 1));
