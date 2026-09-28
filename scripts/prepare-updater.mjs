/** Publish metadata only from the exact bundle version and its checked-in
 * release notes. Called after Tauri creates the version-bound signature; no
 * private key is read here and no network or GitHub publication occurs here. */
import { copyFileSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { nsisPayloadHash } from './nsis-payload.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const { productName, version } = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Only stable SemVer releases belong in the update feed.');
// Normalize checkout line endings so Windows and macOS produce identical notes.
const notes = readFileSync(join(root, 'docs/releases', `v${version}.md`), 'utf8').replace(/\r\n/g, '\n').trim();
if (!notes || notes.length > 24000) throw new Error('Release notes must contain 1–24,000 characters.');
const mac = process.platform === 'darwin' && process.arch === 'arm64';
const win = process.platform === 'win32' && process.arch === 'x64';
if (!mac && !win) throw new Error('Unsupported updater packaging platform.');
const platform = mac ? 'darwin-aarch64' : 'windows-x86_64';
const directory = join(root, 'src-tauri/target/release/bundle', mac ? 'macos' : 'nsis');
// Match the exact version/architecture, never the first old installer in target/.
const candidates = mac ? [`${productName}.app.tar.gz`] : readdirSync(directory).filter(name => name === `${productName}_${version}_x64-setup.exe`);
if (candidates.length !== 1) throw new Error('Expected exactly one installer for this version and architecture.');
const source = join(directory, candidates[0]);
const signature = readFileSync(`${source}.sig`, 'utf8').trim();
if (!signature || statSync(source).size > 256 * 1024 * 1024) throw new Error('Missing signature or oversized update archive.');
// This structural guard catches stale signatures early; package-release also
// runs real Tauri verification against the embedded public key before upload.
const trustedComment = Buffer.from(signature, 'base64').toString('utf8').split('\n').find(line => line.startsWith('trusted comment:'));
if (!trustedComment?.split('\t').includes(`version:${version}`)) throw new Error('Signature must bind the exact release version.');
const output = join(root, 'artifacts/releases');
mkdirSync(output, { recursive: true });
const name = mac ? `${productName}-${version}-macOS-arm64.app.tar.gz` : `${productName}-${version}-Windows-x64-setup.exe`;
copyFileSync(source, join(output, name));
copyFileSync(`${source}.sig`, join(output, `${name}.sig`));
// Each builder owns a distinct fragment. Only the final publish job may create
// latest.json, after verifying that both platforms and their assets are present.
writeFileSync(join(output, `manifest-${platform}.json`), `${JSON.stringify({
  version,
  notes,
  pub_date: new Date().toISOString(),
  // The independent Windows installer job has no Cargo target directory. Bind
  // its expected installed payload to the binary plus Tauri's NSIS marker.
  ...(win ? { binary_sha256: nsisPayloadHash(readFileSync(join(root, 'src-tauri/target/release/nobody.exe'))) } : {}),
  platforms: {
    [platform]: {
      signature,
      url: `https://github.com/t0ken-ai/Nobody/releases/download/v${version}/${name}`,
    },
  },
}, null, 2)}\n`);
console.log(`Prepared signed updater metadata for v${version}: ${platform}.`);
