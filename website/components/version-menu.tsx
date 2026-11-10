'use client';

import { usePathname } from 'next/navigation';
import { useEffect, useState } from 'react';
import { type ReleasedVersion, type VersionsFile, docsPathOf, versionChoices } from '@/lib/versions';

/**
 * The version menu above the sidebar: every released minor version, the latest first, and the
 * preview of the next one. Picking one opens the same page in that version (or, where it has no
 * such page, its start page: the 404 page sees to that). The list comes from the site root's
 * versions.json at run time, so older versions' docs offer newer versions too.
 */
export function VersionMenu({
  current,
  siteUrl,
  previewUrl,
}: Readonly<{
  /** The minor version these docs are, or `next` for the preview. */
  current: string;
  siteUrl: string;
  previewUrl: string;
}>) {
  // Next.js leaves the base path out of usePathname, so this is already /docs/….
  const docsPath = docsPathOf(usePathname(), '');
  const [released, setReleased] = useState<ReleasedVersion[]>([]);
  useEffect(() => {
    fetch('/versions.json')
      .then((response) => (response.ok ? (response.json() as Promise<VersionsFile>) : { versions: [] }))
      .then((file) => setReleased(file.versions))
      .catch(() => setReleased([]));
  }, []);

  const choices = versionChoices(released, docsPath, siteUrl, previewUrl);
  if (!choices.some((c) => c.minor === current)) {
    // These docs are not in the list yet (before their release, or a preview without its address).
    choices.unshift({
      label: current === 'next' ? 'next (unreleased)' : current,
      minor: current,
      href: '',
    });
  }
  return (
    <label className="flex items-center gap-2 text-sm text-fd-muted-foreground">
      <span>Version</span>
      <select
        className="flex-1 rounded-md border bg-fd-secondary px-2 py-1 text-fd-foreground"
        value={current}
        onChange={(event) => {
          const choice = choices.find((c) => c.minor === event.target.value);
          if (choice?.href) window.location.href = choice.href;
        }}
      >
        {choices.map((c) => (
          <option key={c.minor} value={c.minor}>
            {c.label}
          </option>
        ))}
      </select>
    </label>
  );
}
