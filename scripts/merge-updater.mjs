/** Merge only a complete, matching pair of platform builds. Runs once after
 * matrix jobs succeed; missing/corrupt assets must leave the public feed intact. */
import { createHash } from 'node:crypto';
import { createReadStream, readFileSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Verify checksums, origin, signature association and release text before
 * combining fragments. Signature cryptography is checked by each native build. */
export async function mergeUpdates(directory, version, notes) {
  const platforms = {};
  for (const [target, label, required] of [
    ['darwin-aarch64', 'macOS-arm64', ['.dmg', '.zip', '.app.tar.gz', '.app.tar.gz.sig']],
    ['windows-x86_64', 'Windows-x64', ['-setup.exe', '-setup.exe.sig']],
  ]) {
    const prefix = `Nobody-${version}-${label}`;
    const files = new Set();
    for (const line of readFileSync(join(directory, `${prefix}.sha256`), 'utf8').trim().split('\n')) {
      const match = /^([a-f0-9]{64})  ([A-Za-z0-9._-]+)$/.exec(line.trim());
      if (!match || files.has(match[2])) throw new Error('Invalid or duplicate checksum entry.');
      const [, expected, file] = match;
      const hash = createHash('sha256');
      for await (const chunk of createReadStream(join(directory, file))) hash.update(chunk);
      if (hash.digest('hex') !== expected) throw new Error(`Checksum mismatch: ${file}`);
      files.add(file);
    }
    const fragmentName = `manifest-${target}.json`;
    if ([...required.map(suffix => prefix + suffix), fragmentName].some(file => !files.has(file))) throw new Error(`Incomplete ${target} artifacts.`);
    const fragment = JSON.parse(readFileSync(join(directory, fragmentName), 'utf8'));
    if (fragment.version !== version || fragment.notes !== notes || Object.keys(fragment.platforms).join() !== target) throw new Error(`Mismatched ${target} metadata.`);
    const entry = fragment.platforms[target];
    const asset = prefix + (target === 'darwin-aarch64' ? '.app.tar.gz' : '-setup.exe');
    if (entry.url !== `https://github.com/t0ken-ai/Nobody/releases/download/v${version}/${asset}` || entry.signature !== readFileSync(join(directory, `${asset}.sig`), 'utf8').trim()) throw new Error(`Mismatched ${target} signature or URL.`);
    platforms[target] = entry;
  }
  return { version, notes, pub_date: new Date().toISOString(), platforms };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const directory = resolve(process.argv[2] ?? join(root, 'artifacts/releases'));
  const { version } = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  // Normalize checkout line endings so Windows and macOS produce identical notes.
  const notes = readFileSync(join(root, 'docs/releases', `v${version}.md`), 'utf8').replace(/\r\n/g, '\n').trim();
  const merged = await mergeUpdates(directory, version, notes);
  const latest = `${JSON.stringify(merged, null, 2)}\n`;
  writeFileSync(join(directory, 'latest.json'), latest);
  writeFileSync(join(directory, 'latest.json.sha256'), `${createHash('sha256').update(latest).digest('hex')}  latest.json\n`);
  writeFileSync(join(directory, 'release-notes.md'), `${notes}\n`);
  console.log(`Merged ${Object.keys(merged.platforms).join(', ')} into ${basename(directory)}/latest.json`);
}
