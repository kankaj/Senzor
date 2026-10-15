// Checks every internal link in content/docs/**/*.{md,mdx}: page paths, #anchors (also within the
// same page), relative links to other content files, and files in public/ (screenshots). It also
// flags raw `<img src="/...">` tags, which break under a sub-path. Run it with `pnpm check:links`;
// it exits non-zero on a problem.
//
// Not covered: JSX expression attributes (`href={...}`), external links, and the content of
// `<include>`d .md/.mdx files (the current includes are TOML code).
//
// This is the "Validate links" setup from the Fumadocs docs (next-validate-link). The usual
// guide reads pages through the `source` object, which needs a runtime MDX loader outside the
// bundler; building the page list with `loader()` from the file names and taking each page's
// headings from `getTableOfContents` gives the same result without one.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { frontmatter } from 'fumadocs-core/content/md/frontmatter';
import { getTableOfContents } from 'fumadocs-core/content/toc';
import { loader } from 'fumadocs-core/source';
import { printErrors, scanURLs, validateFiles } from 'next-validate-link';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const docsDir = path.join(root, 'content/docs');
const publicDir = path.join(root, 'public');
const docsRoute = '/docs';

// Slugs and URLs come from the same loader the site uses, so (group) folders and index pages
// map the way they do in the app.
const source = loader({
  baseUrl: docsRoute,
  source: {
    files: readdirSync(docsDir, { recursive: true })
      .filter((file) => /\.mdx?$/.test(file))
      .sort()
      .map((file) => ({
        type: 'page',
        path: file.split(path.sep).join('/'),
        data: { title: file },
      })),
  },
});

const pages = source.getPages().map((page) => {
  const absolutePath = path.join(docsDir, page.path);
  return {
    absolutePath,
    slugs: page.slugs,
    url: page.url,
    content: readFileSync(absolutePath, 'utf8'),
  };
});

const scanned = await scanURLs({
  preset: 'next',
  cwd: root,
  populate: {
    'docs/[[...slug]]': pages.map((page) => ({
      value: { slug: page.slugs },
      // The frontmatter would otherwise be read as a heading.
      hashes: getTableOfContents(frontmatter(page.content).content).map((item) =>
        item.url.slice(1),
      ),
    })),
  },
});

// Files in public/ are served as-is and are not routes, so check them on disk.
const publicFileExists = (href) => {
  if (!href.startsWith('/') || href.startsWith('//')) return false;
  try {
    const file = path.join(publicDir, decodeURI(href.split(/[?#]/)[0]));
    return file.startsWith(publicDir + path.sep) && statSync(file).isFile();
  } catch {
    return false;
  }
};

// A relative link to a content file (`./page.mdx`) is checked as that page's URL. A path that is
// not a page must not pass silently, so it maps to a URL that does not exist.
const urlByFile = new Map(pages.map((page) => [path.normalize(page.absolutePath), page.url]));
const pathToUrl = (file) => urlByFile.get(path.normalize(file)) ?? '/__missing__';

// next-validate-link only looks at Markdown links by default. Also check image sources,
// link reference definitions, and the `href`/`src` of any JSX element (<Card href>, <img src>).
const LINK_ATTRIBUTES = new Set(['href', 'src']);
const linksOf = (node) => {
  if (node.type === 'link' || node.type === 'image' || node.type === 'definition') {
    return { hrefs: [node.url] };
  }
  if (node.type === 'mdxJsxFlowElement' || node.type === 'mdxJsxTextElement') {
    const hrefs = node.attributes
      .filter((attr) => attr.type === 'mdxJsxAttribute' && LINK_ATTRIBUTES.has(attr.name))
      .map((attr) => attr.value)
      .filter((value) => typeof value === 'string');
    return { hrefs };
  }
};

// Next.js prefixes the base path to Markdown images and links, but not to a raw <img src>.
const rawImageSource = (node) => {
  if (
    (node.type !== 'mdxJsxFlowElement' && node.type !== 'mdxJsxTextElement') ||
    node.name !== 'img'
  ) {
    return undefined;
  }
  const src = node.attributes.find(
    (attr) => attr.type === 'mdxJsxAttribute' && attr.name === 'src',
  )?.value;
  return typeof src === 'string' && src.startsWith('/') ? src : undefined;
};

// Each page is validated on its own so that a same-page link (`#anchor`), which the library
// skips because it has no path, can be rewritten to the page's own URL.
const validatePage = async (page) => {
  const rawImages = [];
  const [result] = await validateFiles(
    [{ path: page.absolutePath, content: page.content, url: page.url }],
    {
      scanned,
      whitelist: publicFileExists,
      pathToUrl,
      checkRelativePaths: 'as-url',
      markdown: {
        onNode: (node) => {
          const src = rawImageSource(node);
          if (src !== undefined) {
            rawImages.push({
              url: src,
              line: node.position.start.line,
              column: node.position.start.column,
              reason: new Error(
                'raw <img src="/..."> is not prefixed with the base path, use ![alt](/path) instead',
              ),
            });
          }
          const links = linksOf(node);
          return (
            links && {
              hrefs: links.hrefs.map((href) => (href.startsWith('#') ? page.url + href : href)),
            }
          );
        },
      },
    },
  );
  const errors = [...(result?.errors ?? []), ...rawImages].sort(
    (a, b) => a.line - b.line || a.column - b.column,
  );
  return errors.length > 0 ? { file: page.absolutePath, errors, detected: [] } : undefined;
};

const results = (await Promise.all(pages.map(validatePage))).filter(Boolean);

printErrors(results, true);
console.log(`Checked ${pages.length} pages: no broken links.`);
