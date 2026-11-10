import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  type ReleasedVersion,
  docsPathOf,
  minorOf,
  notFoundTarget,
  versionChoices,
  withRelease,
} from './versions.ts';

const site = 'https://senzor.kankaj.cz';
const released: ReleasedVersion[] = [
  { minor: '1.1', release: '1.1.2' },
  { minor: '1.0', release: '1.0.3' },
];

test('a version belongs to its minor version, pre-releases too', () => {
  assert.equal(minorOf('1.0.0'), '1.0');
  assert.equal(minorOf('1.0.3'), '1.0');
  assert.equal(minorOf('2.11.0-rc.1'), '2.11');
});

test('the docs path is the page below the base path', () => {
  assert.equal(docsPathOf('/docs/gui/settings/', ''), '/docs/gui/settings/');
  assert.equal(docsPathOf('/1.0/docs/gui/settings/', '/1.0'), '/docs/gui/settings/');
  assert.equal(docsPathOf('/1.0/', '/1.0'), '/docs/');
  assert.equal(docsPathOf('/', ''), '/docs/');
});

test('the choices name every released version, the newest as latest, then next', () => {
  const choices = versionChoices(released, '/docs/mcp/', site, 'https://next.example');
  assert.deepEqual(choices, [
    { label: '1.1 (latest)', minor: '1.1', href: `${site}/1.1/docs/mcp/` },
    { label: '1.0', minor: '1.0', href: `${site}/1.0/docs/mcp/` },
    { label: 'next (unreleased)', minor: 'next', href: 'https://next.example/docs/mcp/' },
  ]);
});

test('without a preview address there is no next choice', () => {
  const choices = versionChoices(released, '/docs/', site, '');
  assert.deepEqual(
    choices.map((c) => c.minor),
    ['1.1', '1.0'],
  );
});

test('a missing page of a released version goes to that version start page', () => {
  assert.equal(notFoundTarget('/1.0/docs/gone/', released), '/1.0/docs/');
});

test('a version that is not released goes to the same page of the latest docs', () => {
  assert.equal(notFoundTarget('/1.2/docs/mcp/', released), '/docs/mcp/');
  assert.equal(notFoundTarget('/1.2/docs/', released), '/docs/');
});

test('other missing pages stay a plain 404', () => {
  assert.equal(notFoundTarget('/docs/gone/', released), null);
  assert.equal(notFoundTarget('/1.0/docs/', released), null, 'no loop on a version start page');
  assert.equal(notFoundTarget('/favicon.png', released), null);
});

test('a release takes its minor version first or updates it', () => {
  assert.deepEqual(withRelease([], '1.0.0'), [{ minor: '1.0', release: '1.0.0' }]);
  assert.deepEqual(withRelease(released, '1.0.4'), [
    { minor: '1.1', release: '1.1.2' },
    { minor: '1.0', release: '1.0.4' },
  ]);
  assert.deepEqual(withRelease(released, '1.2.0'), [
    { minor: '1.2', release: '1.2.0' },
    ...released,
  ]);
  assert.throws(() => withRelease(released, '1.2.0-rc.1'), /pre-release/);
});
