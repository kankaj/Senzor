import type { Metadata } from 'next';
import { IBM_Plex_Mono, IBM_Plex_Sans } from 'next/font/google';
import { Banner } from 'fumadocs-ui/components/banner';
import { Provider } from '@/components/provider';
import { channel, robotsOf } from '@/lib/channel';
import { appDescription, appName, basePath, siteUrl } from '@/lib/shared';
import { version } from '@/lib/version';
import './global.css';

// IBM Plex for text and code alike; app/global.css hands the variables to Tailwind's font-sans
// and font-mono.
const plexSans = IBM_Plex_Sans({
  subsets: ['latin', 'latin-ext'],
  weight: ['400', '500', '600', '700'],
  variable: '--font-plex-sans',
});
const plexMono = IBM_Plex_Mono({
  subsets: ['latin', 'latin-ext'],
  weight: ['400', '500', '600'],
  variable: '--font-plex-mono',
});

/** Search-result highlighting (see app/global.css): kept out of the CSS pipeline, whose parser rejects it. */
const searchHighlight =
  '::highlight(fd-search){color:var(--color-fd-primary);text-decoration-line:underline}';

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: { default: appName, template: `%s | ${appName}` },
  description: appDescription,
  // Pages without their own preview, such as the home page, show the docs' first page's.
  openGraph: { siteName: appName, images: `${basePath}/og/docs/image.png` },
  twitter: { card: 'summary_large_image', images: `${basePath}/og/docs/image.png` },
  robots: robotsOf(channel),
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html
      lang="en"
      className={`${plexSans.variable} ${plexMono.variable}`}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        <style dangerouslySetInnerHTML={{ __html: searchHighlight }} />
        {channel === 'next' ? (
          <Banner>
            <span>
              These docs describe Senzor {version}, not released yet. For the released version, see{' '}
              <a href={siteUrl} className="underline">
                {siteUrl.replace('https://', '')}
              </a>
              .
            </span>
          </Banner>
        ) : null}
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
