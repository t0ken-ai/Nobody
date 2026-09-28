/** Return the hash of the unsigned payload Tauri puts inside NSIS. Tauri 2.12
 * patches the bundle marker before packaging, then restores target/release's
 * executable. Comparing the restored file directly causes a false install
 * failure. Keep every other byte unchanged and reject unknown marker layouts.
 * This applies only without Authenticode; signed binaries need a captured or
 * extracted post-signing payload instead.
 * Reference: tauri-bundler-v2.10.0/src/bundle.rs (patch_binary and restoration). */
import { createHash } from 'node:crypto';

export function nsisPayloadHash(original) {
  const marker = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
  const offset = original.indexOf(marker);
  if (offset < 0 || original.indexOf(marker, offset + marker.length) !== -1) {
    throw new Error('Expected one unpatched Tauri bundle marker; review the NSIS payload verification.');
  }
  const payload = Buffer.from(original);
  Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS').copy(payload, offset);
  return createHash('sha256').update(payload).digest('hex');
}
