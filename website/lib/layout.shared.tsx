import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { appName, gitConfig } from './shared';
import { version } from './version';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: (
        <>
          <span className="font-semibold">{appName}</span>
          <span className="rounded-md border bg-fd-secondary px-1.5 py-0.5 text-xs font-medium text-fd-muted-foreground">
            v{version}
          </span>
        </>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
