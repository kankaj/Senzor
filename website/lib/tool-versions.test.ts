// mise.toml pins Node and pnpm for development and CI. Cloudflare's builds, which run from the
// repository root, read their own files: .node-version there for Node, and `packageManager` in
// website/package.json, where pnpm runs, for pnpm (ahead of any PNPM_VERSION build variable).
// These tests keep both equal to mise.toml, so the docs build the same everywhere.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const website = new URL('../', import.meta.url);

/** The version mise.toml pins for `tool`, as in `pnpm = "12.15.0"`. */
function misePin(tool: string): string {
  const mise = readFileSync(new URL('../mise.toml', website), 'utf8');
  const pin = new RegExp(`^${tool} = "([^"]+)"$`, 'm').exec(mise);
  assert.ok(pin, `mise.toml pins ${tool}`);
  return pin[1];
}

test("Cloudflare's pnpm is the one mise pins", () => {
  const pkg = JSON.parse(readFileSync(new URL('package.json', website), 'utf8'));
  assert.equal(pkg.packageManager, `pnpm@${misePin('pnpm')}`);
});

test("Cloudflare's Node is the one mise pins", () => {
  const nodeVersion = readFileSync(new URL('../.node-version', website), 'utf8').trim();
  assert.equal(nodeVersion, misePin('node'));
});
