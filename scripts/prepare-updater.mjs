/** Publish metadata only from the exact bundle version and its checked-in
 * release notes. Called after Tauri creates the version-bound signature; no
 * private key is read here and no network or GitHub publication occurs here. */
import { copyFileSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const { productName, version } = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Only stable SemVer releases belong in the update feed.');
const notes = readFileSync(join(root, 'docs/releases', `v${version}.md`), 'utf8').trim();
if (!notes || notes.length > 24000) throw new Error('Release notes must contain 1–24,000 characters.');
const source = join(root, 'src-tauri/target/release/bundle/macos', `${productName}.app.tar.gz`);
const signature = readFileSync(`${source}.sig`, 'utf8').trim();
if (!signature || statSync(source).size > 256 * 1024 * 1024) throw new Error('Missing signature or oversized update archive.');
// This structural guard catches stale signatures early; package-release also
// runs real Tauri verification against the embedded public key before upload.
const trustedComment = Buffer.from(signature, 'base64').toString('utf8').split('\n').find(line => line.startsWith('trusted comment:'));
if (!trustedComment?.split('\t').includes(`version:${version}`)) throw new Error('Signature must bind the exact release version.');
const output = join(root, 'artifacts/releases');
mkdirSync(output, { recursive: true });
const name = `${productName}-${version}-macOS-arm64.app.tar.gz`;
copyFileSync(source, join(output, name));
copyFileSync(`${source}.sig`, join(output, `${name}.sig`));
writeFileSync(join(output, 'latest.json'), `${JSON.stringify({
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    'darwin-aarch64': {
      signature,
      url: `https://github.com/t0ken-ai/Nobody/releases/download/v${version}/${name}`,
    },
  },
}, null, 2)}\n`);
writeFileSync(join(output, 'release-notes.md'), `${notes}\n`);
console.log(`Prepared signed updater metadata for v${version}.`);
