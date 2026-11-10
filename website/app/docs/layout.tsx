import { source } from '@/lib/source';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { VersionMenu } from '@/components/version-menu';
import { channel } from '@/lib/channel';
import { baseOptions } from '@/lib/layout.shared';
import { basePath, previewUrl, siteUrl } from '@/lib/shared';
import { version } from '@/lib/version';
import { minorOf } from '@/lib/versions';

/** The version these docs are: the preview's `next`, a version folder's, or the release's. */
const current = channel === 'next' ? 'next' : (/^\/(\d+\.\d+)$/.exec(basePath)?.[1] ?? minorOf(version));

export default function Layout({ children }: LayoutProps<'/docs'>) {
  return (
    <DocsLayout
      tree={source.getPageTree()}
      {...baseOptions()}
      sidebar={{ banner: <VersionMenu current={current} siteUrl={siteUrl} previewUrl={previewUrl} /> }}
    >
      {children}
    </DocsLayout>
  );
}
