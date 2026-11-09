import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import { Provider } from '@/components/provider';
import { appDescription, appName, basePath, siteUrl } from '@/lib/shared';
import './global.css';

const inter = Inter({
  subsets: ['latin'],
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
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html lang="en" className={inter.className} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <style dangerouslySetInnerHTML={{ __html: searchHighlight }} />
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
