/** Preserve two different icons: the mounted volume icon lives inside the DMG;
 * a Finder file icon is extended metadata and survives download only in the
 * companion ZIP. No deprecated UDIF resource/flatten tricks are used. */
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join, basename } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../', import.meta.url));

/** Exact argument arrays keep mount paths with spaces out of shell parsing. */
function run(command, args, capture = false) {
  const result = spawnSync(command, args, { encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed while preparing installer icons: ${result.stderr ?? ''}`);
  return result.stdout;
}

/** Finder flags are big-endian; 0x0400 declares a custom icon on files/volumes. */
function verifyCustomIcon(path) {
  const metadata = Buffer.from(run('/usr/bin/xattr', ['-px', 'com.apple.FinderInfo', path], true).replace(/\s/g, ''), 'hex');
  if (metadata.length < 10 || !(metadata.readUInt16BE(8) & 0x0400)) throw new Error(`Missing Finder icon flag: ${path}`);
}

/** Build and verify a branded volume plus a metadata-preserving download ZIP.
 * Mounts and cleanup are restricted to a fresh packaging-owned directory.
 * A detach failure deliberately leaves it intact rather than deleting a mount. */
export function packageDmg({ staging, dmg, icon, productName }) {
  if (process.platform !== 'darwin') throw new Error('macOS packaging host required.');
  const temporary = mkdtempSync(join(dirname(dmg), '.nobody-dmg-'));
  const mount = join(temporary, 'mount');
  const writable = join(temporary, 'writable.dmg');
  let mounted = false;
  let complete = false;
  const attach = (file, readonly = false) => {
    run('hdiutil', ['attach', file, '-nobrowse', '-noautoopen', '-mountpoint', mount, ...(readonly ? ['-readonly'] : [])]);
    mounted = true;
  };
  const detach = () => { run('hdiutil', ['detach', mount]); mounted = false; };
  try {
    mkdirSync(mount);
    run('hdiutil', ['create', '-volname', productName, '-srcfolder', staging, '-fs', 'HFS+', '-format', 'UDRW', writable]);
    attach(writable);
    copyFileSync(icon, join(mount, '.VolumeIcon.icns'));
    run('xcrun', ['SetFile', '-a', 'C', mount]);
    detach();
    run('hdiutil', ['convert', writable, '-format', 'UDZO', '-o', dmg, '-ov']);
    run('hdiutil', ['verify', dmg]);
    attach(dmg, true);
    verifyCustomIcon(mount);
    if (!readFileSync(join(mount, '.VolumeIcon.icns')).equals(readFileSync(icon))) throw new Error('Mounted volume icon differs from the approved ICNS.');
    detach();
    const tool = join(temporary, 'set-file-icon');
    run('xcrun', ['swiftc', join(root, 'scripts/set-file-icon.swift'), '-o', tool]);
    run(tool, [icon, dmg]);
    verifyCustomIcon(dmg);
    const zip = `${dmg}.zip`;
    // keepParent on a file adds its release directory; keep the DMG at ZIP root.
    run('ditto', ['-c', '-k', '--sequesterRsrc', dmg, zip]);
    // A byte-only copy models GitHub/HTTP transport, which strips outer xattrs.
    const downloaded = join(temporary, 'downloaded.zip');
    copyFileSync(zip, downloaded);
    const restored = join(temporary, 'restored');
    run('ditto', ['-x', '-k', downloaded, restored]);
    const restoredDmg = join(restored, basename(dmg));
    verifyCustomIcon(restoredDmg);
    if (!readFileSync(restoredDmg).equals(readFileSync(dmg))) throw new Error('ZIP round trip changed the disk image.');
    if (!run('/usr/bin/xattr', [restoredDmg], true).includes('com.apple.ResourceFork')) throw new Error('ZIP did not preserve the Finder icon resource.');
    complete = true;
    console.log('PASS: mounted volume icon and downloaded ZIP Finder icon verified.');
    return zip;
  } finally {
    if (mounted) detach();
    // Keep failed fixtures for diagnosis. Never recursively remove a mount.
    if (complete && !mounted && existsSync(temporary)) rmSync(temporary, { recursive: true, force: true });
  }
}
