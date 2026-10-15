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

## Deploying to Cloudflare

The site is published as a Cloudflare Worker that only serves files: [`wrangler.jsonc`](../wrangler.jsonc) at the repository root names the Worker (`senzor-docs`) and points it at `website/out`; unknown paths get `404.html`. Cloudflare's Git integration builds and deploys it: once the repository is connected, every push to `main` deploys, and with preview builds on, other branches get preview URLs.

In the Cloudflare dashboard, **Workers & Pages** → **Create application**, connect the GitHub repository, and on **Set up your application** set:

| Setting                        | Value                                                                            |
| ------------------------------ | -------------------------------------------------------------------------------- |
| Project name                   | `senzor-docs` (the same as `name` in `wrangler.jsonc`)                           |
| Build command                  | `cd website && pnpm install --frozen-lockfile && pnpm check:links && pnpm build` |
| Deploy command                 | `npx wrangler deploy` (the default)                                              |
| Preview command                | `npx wrangler preview` (the default)                                             |
| Enable Preview builds          | on                                                                               |
| Protect with Cloudflare Access | off (the docs are public)                                                        |

The build runs from the repository root, where `wrangler.jsonc` is: the pages include files from outside `website/` (`examples/`, the built-in templates in `crates/senzor-core/`) and the version badge reads `Cargo.toml`.

Cloudflare's build image comes with an older pnpm than the pnpm 11 the site is built with (the first build ran pnpm 9, although Cloudflare's docs list 10). `pnpm-workspace.yaml` lists `packages` so that older pnpm versions can still install, but pin the version so Cloudflare builds exactly like a local build. Add a build variable under **Settings** → **Build** → **Build Variables and Secrets** (Node.js is already 24 there):

| Variable       | Value     |
| -------------- | --------- |
| `PNPM_VERSION` | `11.28.4` |

If the first build ran before you added it and failed, run it again from the deployments list.

Do not set `NODE_VERSION` unless you need to: the default Node.js 24 works, and Cloudflare cannot install a Node.js release that came out only days ago (a build with `NODE_VERSION` = `24.22.0`, released the day before, failed at "Installing nodejs"). A version must also exist: check `npm view pnpm@<version> version` before you set `PNPM_VERSION`.

Leave `DOCS_BASE_PATH` unset: the Worker serves the site from the root of its domain.

Optional: under **Settings** → **Build** → **Build watch paths**, include only the paths the site is built from, so commits that touch only the Rust code do not rebuild it:

```text
website/*
examples/*
crates/senzor-core/src/project/builtin.toml
Cargo.toml
wrangler.jsonc
```

To check what the Worker would serve before deploying, build the site and run `npx wrangler dev` from the repository root; it serves `website/out` the way the Worker does.

### Custom domain

The domain's DNS has to be on Cloudflare (`kankaj.cz` is). In the Worker, **Settings** → **Domains & Routes** → **Add** → **Custom Domain**, and enter the domain, for example `senzor.kankaj.cz`. Cloudflare creates the DNS record and the certificate. A hostname that already has a `CNAME` record cannot be used; delete that record first.

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
