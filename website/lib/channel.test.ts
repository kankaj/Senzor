import assert from 'node:assert/strict';
import { test } from 'node:test';
import { channelOf } from './channel.ts';

test('the release branch builds the released docs', () => {
  assert.equal(channelOf({ WORKERS_CI_BRANCH: 'release' }), 'released');
});

test('every other branch Cloudflare builds is a preview of the next version', () => {
  assert.equal(channelOf({ WORKERS_CI_BRANCH: 'main' }), 'next');
  assert.equal(channelOf({ WORKERS_CI_BRANCH: 'docs-tweak' }), 'next');
});

test('a build outside Workers Builds counts as released', () => {
  assert.equal(channelOf({}), 'released');
  assert.equal(channelOf({ WORKERS_CI_BRANCH: '' }), 'released');
});

test('DOCS_CHANNEL overrides the branch, and only its two values count', () => {
  assert.equal(channelOf({ DOCS_CHANNEL: 'next' }), 'next');
  assert.equal(channelOf({ DOCS_CHANNEL: 'released', WORKERS_CI_BRANCH: 'main' }), 'released');
  assert.equal(channelOf({ DOCS_CHANNEL: 'beta', WORKERS_CI_BRANCH: 'main' }), 'next');
});
