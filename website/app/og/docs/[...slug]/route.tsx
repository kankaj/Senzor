import { generateOGImage } from 'fumadocs-ui/og';
import { notFound } from 'next/navigation';
import { appName } from '@/lib/shared';
import { pageImage, source } from '@/lib/source';

// Rendered once at build time for every page, so the static export holds a PNG per page.
export const revalidate = false;

export async function GET(_req: Request, { params }: RouteContext<'/og/docs/[...slug]'>) {
  const { slug } = await params;
  const page = source.getPage(slug.slice(0, -1));
  if (!page) notFound();

  return generateOGImage({
    title: page.data.title,
    description: page.data.description,
    site: appName,
    primaryColor: 'rgba(14, 107, 95, 0.4)',
    primaryTextColor: 'rgb(79, 194, 173)',
  });
}

export function generateStaticParams() {
  return source.getPages().map((page) => ({ slug: pageImage(page).segments }));
}
