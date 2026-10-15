import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import { Provider } from '@/components/provider';
import { appDescription, appName } from '@/lib/shared';
import './global.css';

const inter = Inter({
  subsets: ['latin'],
});

/** Search-result highlighting (see app/global.css): kept out of the CSS pipeline, whose parser rejects it. */
const searchHighlight =
  '::highlight(fd-search){color:var(--color-fd-primary);text-decoration-line:underline}';

export const metadata: Metadata = {
  title: { default: appName, template: `%s | ${appName}` },
  description: appDescription,
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
