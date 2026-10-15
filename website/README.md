# Senzor documentation site

The documentation for Senzor, built with [Fumadocs](https://fumadocs.dev) on Next.js and exported as a static site.

## Requirements

- Node.js 24 or newer
- [pnpm](https://pnpm.io)

## Run it

```bash
pnpm install
pnpm dev      # development server on http://localhost:3000
pnpm build    # static export into out/
pnpm types:check
pnpm check:links   # internal links, #anchors, images and screenshots in content/docs
```

`pnpm build` writes the finished site to `out/`, which any static host can serve. Every page is exported as a folder with an `index.html` (`trailingSlash: true` in `next.config.mjs`), so hosts that do not rewrite `.html` URLs, such as nginx, Apache and S3, find it. The search index is the plain file `out/api/search`.

### Hosting below a sub-path

Set `DOCS_BASE_PATH` when the site is served from a sub-path instead of a domain root, for example GitHub Pages at `https://<user>.github.io/Senzor/`:

```bash
DOCS_BASE_PATH=/Senzor pnpm build
```

All pages, assets, and the search index are then built under `/Senzor/`. Leave it unset for a root deployment.

`pnpm start` serves `out/` at the root and ignores the base path. To preview a sub-path build, copy `out/` into a folder with the sub-path's name and serve its parent:

```bash
DOCS_BASE_PATH=/Senzor pnpm build
rm -rf /tmp/preview && mkdir /tmp/preview && cp -r out /tmp/preview/Senzor
pnpm exec serve /tmp/preview   # then open http://localhost:3000/Senzor/
```

## Deploying to Cloudflare Pages

The site is published with Cloudflare Pages connected to the GitHub repository. Connecting the repository starts a production deployment of `main` right away, and every later push to `main` deploys again; each pull request gets its own preview URL.

In the Cloudflare dashboard, **Workers & Pages** → **Create application** → **Pages** → **Connect to Git**, pick the repository, and on **Set up builds and deployments** set:

| Setting                | Value                                                                              |
| ---------------------- | ---------------------------------------------------------------------------------- |
| Production branch      | `main`                                                                             |
| Framework preset       | None                                                                               |
| Root directory         | leave empty (the repository root)                                                  |
| Build command          | `cd website && pnpm install --frozen-lockfile && pnpm check:links && pnpm build`   |
| Build output directory | `website/out`                                                                      |

The root directory stays at the repository root because the pages include files from outside `website/` (`examples/`, the built-in templates in `crates/senzor-core/`) and the version badge reads `Cargo.toml`.

On the same page, under **Environment variables (optional)**, add these (later they are in the project's **Settings**). Cloudflare's build image defaults to Node.js 22 and pnpm 10, older than what the site is built with:

| Variable        | Value     |
| --------------- | --------- |
| `NODE_VERSION`  | `24`      |
| `PNPM_VERSION`  | `11.28.4` |

Leave `DOCS_BASE_PATH` unset: Cloudflare serves the site from the root of its domain.

Optional: under **Settings** → **Build** → **Build watch paths**, include only the paths the site is built from, so commits that touch only the Rust code do not rebuild it:

```text
website/*
examples/*
crates/senzor-core/src/project/builtin.toml
Cargo.toml
```

### Custom domain

In the Pages project, **Custom domains** → **Set up a domain**, and enter the domain, for example `senzor.kankaj.cz`. Add the domain here first; a DNS record that points to the project before that returns error 522.

- If the domain's DNS is on Cloudflare, Cloudflare creates the record.
- Otherwise, create a `CNAME` record at your DNS provider from the subdomain (`senzor`) to `<project>.pages.dev`.
- An apex domain such as `kankaj.cz` only works when its DNS zone is on Cloudflare.

## Where things live

| Path                  | Contents                                                                  |
| --------------------- | ------------------------------------------------------------------------- |
| `content/docs/`       | The documentation pages (MDX). `meta.json` files define the sidebar order. |
| `app/(home)/page.tsx` | The landing page.                                                         |
| `lib/layout.shared.tsx` | The navbar: title, version badge, GitHub link. The home page adds its **Docs** link in `app/(home)/layout.tsx`. |
| `lib/version.ts`      | Reads the Senzor version from the workspace `Cargo.toml`.                 |
| `public/screenshots/` | Screenshots used by the pages (generated, see below).                     |
| `scripts/`            | `check-links.mjs`, behind `pnpm check:links`: validates the internal links, `#anchors`, relative `.mdx` links and screenshots in `content/docs/`, and flags raw `<img src="/...">` tags. |

The version shown in the navbar comes from `[workspace.package]` in the repository's `Cargo.toml`, so bumping the release version there is enough.

## Screenshots

The screenshots are rendered by the GUI itself. From the repository root:

```bash
cargo run -p senzor --example screenshots --features screenshots
```

When an MQTT broker already answers on `localhost:1883`, the program uses it, publishes the showcase devices there and leaves their retained `offline` availability messages behind. Stop that broker first (or use a throwaway one) if that matters.

Put the generated PNGs in `public/screenshots/` and reference them from a page with Markdown image syntax, `![alt](/screenshots/name.png)`. The build imports each file, so a missing file fails the build, and the base path is applied automatically. A raw `<img src="/...">` tag in MDX is not rewritten and breaks under a sub-path; `pnpm check:links` flags it.

## Examples

The project files shown in the documentation live in [`examples/`](../examples) at the repository root. `cargo test` validates every one of them with `senzor-cli validate`, so a documented example cannot silently go stale.

Pages pull them in with `<include>` tags whose paths lead outside `website/` (for example `../../../examples/quick-start.toml`, relative to the MDX file). That is why `next.config.mjs` sets the bundler root (`turbopack.root` and `outputFileTracingRoot`) to the repository root; keep it when you change the config.
