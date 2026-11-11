export const appName = 'Senzor';
export const appDescription = 'Simulate fleets of MQTT sensors from a GUI or a headless CLI.';
export const docsRoute = '/docs';

/** Where the site is published; link previews need absolute image URLs. */
export const siteUrl = 'https://senzor.kankaj.cz';

/**
 * Path prefix when the site is hosted below a sub-path (set via `DOCS_BASE_PATH`,
 * forwarded by next.config.mjs). Empty at the domain root.
 * Next.js prefixes its own links, but not plain `<img>` sources or `fetch()` URLs.
 */
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';

/**
 * The preview of the next version (Cloudflare's preview of `main`, on its own domain), offered in
 * the version menu as "next".
 */
export const previewUrl = 'https://main.next.senzor.kankaj.com';

export const gitConfig = {
  user: 'kankaj',
  repo: 'Senzor',
  branch: 'main',
};
