// After `next build` of the production site: puts every released minor version's docs into
// out/<minor>/, from the archive its release attached to its GitHub Release
// (senzor-docs-<minor>.tar.gz, checked against its .sha256). public/versions.json lists them.
// Runs for the `release` branch on Cloudflare's Workers Builds, or when DOCS_VERSIONS=fetch;
// DOCS_ARCHIVES=<folder> takes the archives from that folder instead of GitHub, for trying it.
// Any failure fails the build: a deployment must not lose a version.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const wanted = process.env.WORKERS_CI_BRANCH === 'release' || process.env.DOCS_VERSIONS === 'fetch';
if (!wanted) process.exit(0);

const { versions } = JSON.parse(readFileSync('public/versions.json', 'utf8'));
const releases = 'https://github.com/kankaj/Senzor/releases/download';
const local = process.env.DOCS_ARCHIVES;
const work = mkdtempSync(join(tmpdir(), 'senzor-docs-'));
// tar from a system folder, not whichever one PATH finds first.
const tar = ['/usr/bin/tar', '/bin/tar'].find((path) => existsSync(path));
if (!tar) throw new Error('no tar in /usr/bin or /bin');

async function get(release, file) {
  if (local) return readFileSync(join(local, file));
  const url = `${releases}/v${release}/${file}`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

// Every version's archive and checksum at once; then each is checked and unpacked in turn.
const downloads = await Promise.all(
  versions.map(async ({ minor, release }) => {
    const name = `senzor-docs-${minor}.tar.gz`;
    const [archive, sum] = await Promise.all([get(release, name), get(release, `${name}.sha256`)]);
    return { minor, release, name, archive, expected: sum.toString('utf8').trim().split(/\s+/)[0] };
  }),
);
for (const { minor, release, name, archive, expected } of downloads) {
  const actual = createHash('sha256').update(archive).digest('hex');
  if (actual !== expected) throw new Error(`${name} of ${release}: SHA-256 ${actual}, expected ${expected}`);
  const path = join(work, name);
  writeFileSync(path, archive);
  // The archive holds one folder, <minor>/.
  execFileSync(tar, ['-xzf', path, '-C', 'out', `${minor}/`], { stdio: 'inherit' });
  console.log(`versions: ${minor} (${release}) in out/${minor}/`);
}
console.log(`versions: ${versions.length} version(s) added`);
