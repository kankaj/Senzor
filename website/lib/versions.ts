/**
 * The documentation's versions. Every released minor version keeps its docs at
 * `/<minor>/docs/…` (built by the release workflow, collected by the production build); `/docs/…`
 * is the latest release. `public/versions.json` lists the released minor versions, newest first,
 * each with its latest release; `dev/release.sh` adds a release to it. The version menu and the
 * 404 page read it from the site's root at run time, so the docs of an older version also offer
 * the versions released after it.
 */

export interface ReleasedVersion {
  /** `1.0`: the folder below the site's root. */
  minor: string;
  /** `1.0.3`: the release whose docs the folder holds (the tag without its `v`). */
  release: string;
}

export interface VersionsFile {
  versions: ReleasedVersion[];
}

export interface VersionChoice {
  label: string;
  /** The minor version, or `next` for the preview. */
  minor: string;
  href: string;
}

/** `1.0.3` → `1.0`; a pre-release belongs to the minor version it leads to. */
export function minorOf(version: string): string {
  const [major, minor] = version.split('-')[0].split('.');
  return `${major}.${minor}`;
}

/** The page of `pathname` below `basePath`, as `/docs/…`: what the same page is in any version. */
export function docsPathOf(pathname: string, basePath: string): string {
  const below = basePath && pathname.startsWith(basePath) ? pathname.slice(basePath.length) : pathname;
  return below.startsWith('/docs') ? below : '/docs/';
}

/** The version menu's entries for the page `docsPath`: each released version, then the preview. */
export function versionChoices(
  released: ReleasedVersion[],
  docsPath: string,
  siteUrl: string,
  previewUrl: string,
): VersionChoice[] {
  const choices = released.map((v, i) => ({
    label: i === 0 ? `${v.minor} (latest)` : v.minor,
    minor: v.minor,
    href: `${siteUrl}/${v.minor}${docsPath}`,
  }));
  if (previewUrl) {
    choices.push({ label: 'next (unreleased)', minor: 'next', href: `${previewUrl}${docsPath}` });
  }
  return choices;
}

/**
 * Where the 404 page sends `pathname`, or `null` to show the 404. A page that a released version
 * lacks goes to that version's start page; a version folder that does not exist (a development
 * build of an unreleased version links to one) goes to the same page of the latest docs.
 */
export function notFoundTarget(pathname: string, released: ReleasedVersion[]): string | null {
  const match = /^\/(\d+\.\d+)(\/docs(?:\/.*)?)$/.exec(pathname);
  if (!match) return null;
  const [, minor, docsPath] = match;
  if (released.some((v) => v.minor === minor)) {
    const start = `/${minor}/docs/`;
    return pathname === start || pathname === `/${minor}/docs` ? null : start;
  }
  return docsPath;
}

/** `released` with `release` in it: its minor version updated, or added first. */
export function withRelease(released: ReleasedVersion[], release: string): ReleasedVersion[] {
  if (release.includes('-')) {
    throw new Error(`${release} is a pre-release: only releases get versioned docs`);
  }
  const minor = minorOf(release);
  if (released.some((v) => v.minor === minor)) {
    return released.map((v) => (v.minor === minor ? { minor, release } : v));
  }
  return [{ minor, release }, ...released];
}
