/** Exercise actual Tauri download, version-bound signature verification and
 * installation into a disposable .app. An ephemeral signing key is never used
 * by the shipped app; the running Nobody and all user data remain untouched. */
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

if (process.platform !== 'darwin') throw new Error('The isolated installer check requires macOS.');
const root = fileURLToPath(new URL('../', import.meta.url));
const artifacts = join(root, 'artifacts');
mkdirSync(artifacts, { recursive: true });
const fixture = mkdtempSync(join(artifacts, 'updater-fixture-'));
const env = { ...process.env, NOBODY_UPDATE_FIXTURE_DIR: fixture };
const localSdk = join(artifacts, 'toolchain-downloads/executables/Payload/Library/Developer/CommandLineTools');
if (!env.TRANSLATEME_SWIFT_DEVELOPER_DIR && existsSync(localSdk)) env.TRANSLATEME_SWIFT_DEVELOPER_DIR = localSdk;
/** Secret generation output is suppressed; commands receive argument arrays so
 * fixture paths and signing flags are never interpreted by a shell. */
function run(command, args, quiet = false) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: quiet ? 'ignore' : 'inherit' });
  if (result.error || result.status !== 0) throw new Error(`${command} failed during the isolated updater check.`);
}
try {
  const release = process.argv.includes('--release');
  const config = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  const version = release ? config.version : '0.1.1';
  writeFileSync(join(fixture, 'version.txt'), version);
  if (release) {
    // Validate the exact published bytes against the public key embedded in
    // the client; a mistakenly configured CI secret must fail before upload.
    const bundle = join(root, 'src-tauri/target/release/bundle/macos/Nobody.app');
    copyFileSync(`${bundle}.tar.gz`, join(fixture, 'update.tar.gz'));
    copyFileSync(`${bundle}.tar.gz.sig`, join(fixture, 'update.tar.gz.sig'));
    copyFileSync(join(bundle, 'Contents/MacOS/nobody'), join(fixture, 'expected-binary'));
    writeFileSync(join(fixture, 'test.key.pub'), config.plugins.updater.pubkey);
  } else {
    const binary = join(fixture, 'payload/Nobody.app/Contents/MacOS');
    mkdirSync(binary, { recursive: true });
    writeFileSync(join(binary, 'nobody'), 'updated fixture; never executed\n', { mode: 0o755 });
    run('tar', ['-czf', join(fixture, 'update.tar.gz'), '-C', join(fixture, 'payload'), 'Nobody.app']);
    const cli = join(root, 'node_modules/.bin/tauri');
    run(cli, ['signer', 'generate', '--ci', '-p', '', '-w', join(fixture, 'test.key')], true);
    run(cli, ['signer', 'sign', '-f', join(fixture, 'test.key'), '-p', '', '--app-version', '0.1.1', join(fixture, 'update.tar.gz')], true);
    copyFileSync(join(binary, 'nobody'), join(fixture, 'expected-binary'));
  }
  run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--locked', 'isolated_signed_updater_install', '--', '--ignored', '--nocapture']);
} finally {
  // Only the mkdtemp directory owned by this invocation is removed.
  rmSync(fixture, { recursive: true, force: true });
}
