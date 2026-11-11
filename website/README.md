# Senzor documentation site

The documentation for Senzor, built with [Fumadocs](https://fumadocs.dev) on Next.js and exported as a static site.

## Requirements

Node.js and [pnpm](https://pnpm.io), in the versions `mise.toml` in the repository root pins; CI
uses the same. With [mise](https://mise.jdx.dev), run `mise trust` once and then `mise install` in
the repository.

## Run it

```bash
pnpm install
pnpm dev      # development server on http://localhost:3000
pnpm build    # static export into out/
pnpm types:check
pnpm check:links   # internal links, #anchors, images and screenshots in content/docs
pnpm test          # the site's own logic (lib/*.test.ts, Node's test runner)
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

The site is published as a Cloudflare Worker that only serves files: [`wrangler.jsonc`](../wrangler.jsonc) at the repository root names the Worker (`senzor-docs`) and points it at `website/out`; unknown paths get `404.html`. [`public/_headers`](public/_headers) gives `/api/search`, the search index, a JSON content type, which its name without an extension does not. Every docs page has a link-preview image, rendered at build time by `app/og/docs/[...slug]/route.tsx` into `out/og/docs/<page>/image.png`; the pages point to it with an absolute URL on `siteUrl` in `lib/shared.ts`, so change that when the domain changes. Cloudflare's Git integration builds and deploys it.

The production site shows the docs of the released version: Cloudflare's production branch is `release`, which only the release workflow moves, to each release's tag (`.github/workflows/publish-docs.yml`; a pre-release leaves it alone). Every other branch, `main` above all, gets a preview with the docs of the next version: a banner says they are not released yet and the version badge reads "(unreleased)". The build tells the two apart by `WORKERS_CI_BRANCH`, which Workers Builds sets to the branch's name (`lib/channel.ts`); a build elsewhere counts as released. To see the preview locally: `DOCS_CHANNEL=next pnpm build`, or `DOCS_CHANNEL=next pnpm dev`.

### Versions

Every released minor version keeps its docs at `/<minor>/docs/` (`/1.0/docs/`); `/docs/` is the latest release, and the app's help links open the folder of the version that is running. `public/versions.json` lists the released minor versions, newest first, each with the release whose docs it holds; `dev/release.sh` adds each release to it. For each release, the release workflow builds the docs with `DOCS_BASE_PATH=/<minor>` (`scripts/docs-archive.sh`, in `.github/workflows/docs-archive.yml`) and attaches them as `senzor-docs-<minor>.tar.gz` with a `.sha256` to the GitHub Release. The production build (the `release` branch) then unpacks every listed version's archive into `out/<minor>/` (`scripts/versions.mjs`, run by `pnpm build`), checking each checksum, and fails if one is missing: a deployment never loses a version. To try it locally, with archives in a folder instead of GitHub: `DOCS_VERSIONS=fetch DOCS_ARCHIVES=<folder> pnpm build`.

The **Version** menu above the sidebar (`components/version-menu.tsx`) and the 404 page read `/versions.json` from the site's root in the browser, so the docs of an older version offer the versions released after it too. Picking a version opens the same page there. The 404 page (`app/not-found.tsx`) sends a page that a version lacks to that version's start page, and a version folder that does not exist (a development build of an unreleased version links to one) to the same page of the latest docs. Set `previewUrl` in `lib/shared.ts` to offer the preview as "next" in the menu. Cloudflare marks workers.dev URLs `noindex`, so search engines keep to the production site.

In the Cloudflare dashboard, **Workers & Pages** → **Create application**, connect the GitHub repository, and on **Set up your application** set:

| Setting                        | Value                                                                            |
| ------------------------------ | -------------------------------------------------------------------------------- |
| Project name                   | `senzor-docs` (the same as `name` in `wrangler.jsonc`)                           |
| Build command                  | `cd website && pnpm install --frozen-lockfile && pnpm check:links && pnpm build` |
| Deploy command                 | `npx wrangler deploy` (the default)                                              |
| Preview command                | `npx wrangler preview` (the default)                                             |
| Enable Preview builds          | on                                                                               |
| Production branch              | `release` (**Settings** → **Build** → **Branch control**)                        |
| Protect with Cloudflare Access | off (the docs are public)                                                        |

The build runs from the repository root, where `wrangler.jsonc` is: the pages include files from outside `website/` (`examples/`, the built-in templates in `crates/senzor-core/`) and the version badge reads `Cargo.toml`.

Cloudflare builds with the Node.js and pnpm that `mise.toml` pins for development and CI, from two files that repeat them: `.node-version` at the top of the repository (Cloudflare reads it in the folder the build starts in) and `packageManager` in `website/package.json` (Corepack reads it where pnpm runs, ahead of any `PNPM_VERSION` build variable). `pnpm test` fails when either differs from `mise.toml`, so change all three together. Set no `NODE_VERSION` or `PNPM_VERSION` build variable, for production or for previews: the files decide. `pnpm-workspace.yaml` still lists `packages`, so that an older pnpm, such as the build image's own, can install too.

Cloudflare cannot install a Node.js release that came out only days ago (a build with Node.js 24.22.0, released the day before, failed at "Installing nodejs"), so move to a new Node.js only once it is a couple of weeks old.

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
