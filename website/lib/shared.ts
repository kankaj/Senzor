export const appName = 'Senzor';
export const appDescription = 'Simulate fleets of MQTT sensors from a GUI or a headless CLI.';
export const docsRoute = '/docs';

/**
 * Path prefix when the site is hosted below a sub-path (set via `DOCS_BASE_PATH`,
 * forwarded by next.config.mjs). Empty at the domain root.
 * Next.js prefixes its own links, but not plain `<img>` sources or `fetch()` URLs.
 */
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';

export const gitConfig = {
  user: 'kankaj',
  repo: 'Senzor',
  branch: 'main',
};
