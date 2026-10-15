import path from 'node:path';
import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

// Pages include example projects and the built-in templates from outside website/
// (../examples, ../crates). The bundler only reads files below its root, so the
// root is the repository root.
const repoRoot = path.join(import.meta.dirname, '..');

// Set DOCS_BASE_PATH (e.g. "/Senzor") when the site is hosted below a sub-path.
const basePath = process.env.DOCS_BASE_PATH || undefined;

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  // Every page is exported as <route>/index.html, so hosts that do not rewrite .html extensions serve it.
  trailingSlash: true,
  reactStrictMode: true,
  basePath,
  images: { unoptimized: true },
  env: { NEXT_PUBLIC_BASE_PATH: basePath ?? '' },
  turbopack: { root: repoRoot },
  outputFileTracingRoot: repoRoot,
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md into website/.
  agentRules: false,
};

export default withMDX(config);
