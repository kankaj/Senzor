import { loader } from 'fumadocs-core/source';
import { basePath, docsRoute } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';

const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: pageSchema,
  },
  meta: {
    schema: metaSchema,
  },
});

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
});

/** The link-preview image of a docs page, rendered at build time by app/og/docs/[...slug]. */
export function pageImage(page: ReturnType<typeof source.getPages>[number]) {
  const segments = [...page.slugs, 'image.png'];
  return { segments, url: `${basePath}/og/docs/${segments.join('/')}` };
}
