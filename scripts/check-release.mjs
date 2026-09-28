/** Reject mismatched source/tag versions before signing. This shared gate runs
 * on both builders; platform-specific installers must describe one release. */
import { appendFileSync, readFileSync } from 'node:fs';
const read = path => JSON.parse(readFileSync(path, 'utf8'));
const { version } = read('src-tauri/tauri.conf.json');
const cargo = readFileSync('src-tauri/Cargo.toml', 'utf8').match(/\[package\][\s\S]*?\nversion = "([^"]+)"/);
const locked = readFileSync('src-tauri/Cargo.lock', 'utf8').match(/\[\[package\]\]\nname = "nobody"\nversion = "([^"]+)"/);
if (!/^\d+\.\d+\.\d+$/.test(version) || [read('package.json').version, read('package-lock.json').packages[''].version, cargo?.[1], locked?.[1]].some(v => v !== version)) {
  throw new Error('App, npm and Rust versions must agree before signing.');
}
if (process.env.GITHUB_REF_TYPE === 'tag' && process.env.GITHUB_REF_NAME !== `v${version}`) throw new Error(`Tag must be v${version}.`);
if (!((process.platform === 'darwin' && process.arch === 'arm64') || (process.platform === 'win32' && process.arch === 'x64'))) throw new Error('Unsupported release builder.');
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\n`);
console.log(`Release version: ${version} (${process.platform}/${process.arch})`);
