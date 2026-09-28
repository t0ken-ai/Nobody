/** Feed merging is an all-or-nothing release boundary. Synthetic files verify
 * both platforms survive, and missing/tampered/mismatched inputs cannot publish. */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { mergeUpdates } from './merge-updater.mjs';
import { nsisPayloadHash } from './nsis-payload.mjs';

// Installer bytes differ from the restored build output only at the bundle
// marker. Do not hide corruption or mutate the original binary while checking.
test('NSIS payload accounts for bundle marker and preserves all other bytes', () => {
  const original = Buffer.from('header\u0000__TAURI_BUNDLE_TYPE_VAR_UNK\u0000payload');
  const expected = Buffer.from('header\u0000__TAURI_BUNDLE_TYPE_VAR_NSS\u0000payload');
  assert.equal(nsisPayloadHash(original), createHash('sha256').update(expected).digest('hex'));
  assert.ok(original.includes('__TAURI_BUNDLE_TYPE_VAR_UNK'));
  assert.notEqual(nsisPayloadHash(Buffer.concat([original, Buffer.from('changed')])), nsisPayloadHash(original));
});
test('NSIS payload rejects missing or ambiguous bundle markers', () => {
  assert.throws(() => nsisPayloadHash(Buffer.from('unknown binary')), /Expected one/);
  assert.throws(() => nsisPayloadHash(Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK__TAURI_BUNDLE_TYPE_VAR_UNK')), /Expected one/);
});
function fixture() {
  const directory = mkdtempSync(join(tmpdir(), 'nobody-feed-'));
  for (const [target, label, suffixes] of [
    ['darwin-aarch64', 'macOS-arm64', ['.dmg', '.zip', '.app.tar.gz', '.app.tar.gz.sig']],
    ['windows-x86_64', 'Windows-x64', ['-setup.exe', '-setup.exe.sig']],
  ]) {
    const prefix = `Nobody-0.1.2-${label}`;
    const files = suffixes.map(s => prefix + s);
    files.forEach(file => writeFileSync(join(directory, file), file.endsWith('.sig') ? 'fixture-signature' : file));
    const asset = prefix + (target.startsWith('darwin') ? '.app.tar.gz' : '-setup.exe');
    const fragment = `manifest-${target}.json`;
    writeFileSync(join(directory, fragment), JSON.stringify({ version: '0.1.2', notes: 'notes', platforms: { [target]: { signature: 'fixture-signature', url: `https://github.com/t0ken-ai/Nobody/releases/download/v0.1.2/${asset}` } } }));
    files.push(fragment);
    writeFileSync(join(directory, `${prefix}.sha256`), files.map(file => `${createHash('sha256').update(readFileSync(join(directory, file))).digest('hex')}  ${file}`).join('\n'));
  }
  return directory;
}
for (const scenario of ['complete', 'missing-windows', 'tampered-package', 'different-notes']) {
  test(scenario, async () => {
    const directory = fixture();
    try {
      if (scenario === 'missing-windows') rmSync(join(directory, 'manifest-windows-x86_64.json'));
      if (scenario === 'tampered-package') writeFileSync(join(directory, 'Nobody-0.1.2-Windows-x64-setup.exe'), 'changed');
      const operation = mergeUpdates(directory, '0.1.2', scenario === 'different-notes' ? 'wrong' : 'notes');
      if (scenario !== 'complete') await assert.rejects(operation);
      else assert.deepEqual(Object.keys((await operation).platforms), ['darwin-aarch64', 'windows-x86_64']);
    } finally { rmSync(directory, { recursive: true, force: true }); }
  });
}
