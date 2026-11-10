'use client';

import { useEffect } from 'react';
import { type VersionsFile, notFoundTarget } from '@/lib/versions';

/**
 * On the 404 page: a page that a released version lacks goes to that version's start page, and
 * a version folder that does not exist yet (a development build of an unreleased version links
 * to one) goes to the same page of the latest docs. Anything else stays a 404.
 */
export function VersionRedirect() {
  useEffect(() => {
    fetch('/versions.json')
      .then((response) => (response.ok ? (response.json() as Promise<VersionsFile>) : { versions: [] }))
      .then((file) => {
        const target = notFoundTarget(window.location.pathname, file.versions);
        if (target) window.location.replace(target + window.location.hash);
      })
      .catch(() => {});
  }, []);
  return null;
}
