/**
 * Which docs a build is. The production site (senzor.kankaj.cz) is built from the `release`
 * branch, which the release workflow moves to each release's tag: the docs of the version people
 * can download. Every other branch Cloudflare's Workers Builds builds is a preview of the next
 * version: `main` and anything not yet released. Workers Builds names the branch in
 * WORKERS_CI_BRANCH; outside it (`pnpm dev`, CI) a build counts as released. DOCS_CHANNEL set to
 * `next` or `released` overrides both, for trying the preview locally.
 */
export type Channel = 'released' | 'next';

export const releaseBranch = 'release';

export function channelOf(env: Record<string, string | undefined>): Channel {
  if (env.DOCS_CHANNEL === 'next' || env.DOCS_CHANNEL === 'released') {
    return env.DOCS_CHANNEL;
  }
  const branch = env.WORKERS_CI_BRANCH;
  return branch && branch !== releaseBranch ? 'next' : 'released';
}

export const channel: Channel = channelOf(process.env);

/**
 * What search engines may do with a build's pages. The preview repeats most of the released docs
 * on another address (main.next.senzor.kankaj.com, a custom domain, which unlike workers.dev
 * gets no `noindex` from Cloudflare), so it asks them to skip it; the released docs say nothing.
 */
export function robotsOf(channel: Channel): { index: false; follow: false } | undefined {
  return channel === 'next' ? { index: false, follow: false } : undefined;
}
