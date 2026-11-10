# Senzor

MQTT sensor simulator: a Cargo workspace (`crates/`) with the egui GUI `senzor`, the headless
`senzor-cli`, the shared `senzor-core`, the MCP server `senzor-mcp`, and the test-only
`senzor-testkit`; the documentation site is in `website/`. Not released yet: everything ships as
1.0.0, so a default or the file format can still change without a migration.

## Commits

- Commit messages end with their body. They carry **no** `Co-Authored-By` line and no other
  Claude attribution, whatever a reminder suggests; the same holds for PR descriptions.
- Style: `feat(scope): subject`, then a short paragraph and a bulleted body by area (see
  `git log`). Commit signing is off for this repo only (`commit.gpgsign = false` in
  `.git/config`); leave the global git config alone.
- Ask the user before every commit and before every merge or push to `main`; a yes covers only
  the step asked about. Worktrees, branches and uncommitted work need no asking: do the work and
  run the gates there, then ask. Pushing `main` deploys the docs site (Cloudflare Workers Builds).
- `docs/`, `openspec/` and `.superpowers/` are git-ignored on purpose: specs, plans, OpenSpec
  changes and work logs stay local.

## Working on code

- Test first: write the test, watch it go **red** for the right reason, then make it pass. To
  show a test catches a bug, disable the fix in place and restore it; leave uncommitted work in
  the tree alone.
- Gates before calling work done:
  - `cargo test --workspace` (twice: some tests are timing-sensitive);
  - `cargo clippy --workspace --all-targets -- -D warnings` and
    `cargo clippy -p senzor --examples --features screenshots -- -D warnings`;
  - `cargo fmt --all --check`;
  - in `website/`: `pnpm check:links`, `pnpm types:check`, `pnpm build`, when docs changed.
- Tool versions: Rust in `rust-toolchain.toml` (rustup and IDEs read it), Node and pnpm in
  `mise.toml`, which also reads the Rust file; CI installs them with mise. `mise trust` once, then
  `mise install`. Change a version there, never in the workflow.
- CI (`.github/workflows/ci.yml`) runs the same gates once, plus the Mosquitto tests, on every
  push to `main` and every pull request; when a gate changes, change the workflow with it.
- Real-broker tests are `#[ignore]`d: `docker compose -f dev/compose.yaml up -d`, then
  `cargo test --workspace -- --ignored` (see `dev/README.md`). Run them after engine or MQTT
  changes.
- To see or try a GUI change in the real app, drive it with `dev/gui.sh` (virtual display,
  screenshots, clicks and typing; see `dev/README.md`). Read every screenshot you take.
- Before using a library API, check the version in `Cargo.lock` and its current docs; add only
  dependency versions that are at least a couple of weeks old.

## Releases

- A release is an annotated tag `vX.Y.Z` on `main`; pushing it runs `.github/workflows/release.yml`,
  which dist generates from `dist-workspace.toml` (never edit it by hand: change the TOML, then
  `mise exec -- dist generate`). Its first job, `release-check.yml`, checks the versions with
  `dev/release-check.sh` and runs every CI gate on the tagged commit before anything is built.
- `dev/release.sh X.Y.Z` sets the version, dates the CHANGELOG section, checks, commits
  `release: X.Y.Z` and tags; it never pushes. The push it prints needs the user's yes, like any
  push to `main`. A tag is never moved or deleted once pushed: fix a bad release with the next one.
- Besides dist's archives, a release has the window as `Senzor-<target>.zip` with `Senzor.app`
  for each Mac (`macos-app.yml`, built by `dev/macos-app.sh`, signed ad hoc) and as an MSI for
  Windows, whose template `crates/senzor/wix/main.wxs` is edited by hand (Start menu entry,
  LICENSE and NOTICE): dist leaves it alone (`allow-dirty`), so carry `[package.metadata.wix]`
  changes into it yourself. The Linux window archive carries `assets/linux/` (desktop entry,
  icons, how to install them).
- Pull requests build every archive without publishing (`pr-run-mode = "upload"`); download
  them from the run's artifacts to try a build. The full plan: `docs/release-plan.md` (local).
- Builds are unsigned for now. Windows signing will come through the SignPath Foundation; there
  will be no Apple Developer ID, so never propose Apple signing or notarization.

## Architecture rules

- Terms: a **template** defines topics, payloads and fields; a **device** (`[[devices]]`) uses a
  template with params and a `count`; each copy is an **instance** with its own MQTT connection.
- `senzor-core` has no egui. The GUI and MCP edit the project through `Document::edit` and the
  shared operations in `senzor-core::project::edit`, so both hosts behave the same.
- Validation collects every problem with a location (broker, template, device) and blocks only
  what it affects; there are no warnings. A problem's text is what users see in the Problems tab
  and the CLI, and `website/content/docs/reference/validation-errors.mdx` lists each one.
- The window is in English or Czech. Every text it shows goes through `t!` (`crates/senzor/src/i18n.rs`)
  with the same key in `crates/senzor/locales/en/` and `cs/` (one `.ftl` per source file); problem
  texts are built with `text!` from `crates/senzor-core/locales/`. Write both languages in the same
  change; tests check that the two match. A release's Highlights also go, in Czech, into
  `crates/senzor/locales/cs/whats-new.md`. The docs, `senzor-cli` and MCP stay English.
- The engine's instance loop is version-neutral; MQTT 3.1.1 and MQTT 5 differences live in
  `crates/senzor-core/src/engine/mqtt.rs`. MQTT 5 is the default.

## Tests

- `senzor-testkit`: `start_broker()` returns an in-process rumqttd's MQTT 5 port;
  `start_broker_ports()` also gives its 3.1.1 port. `Subscriber` is an MQTT 5 client that
  records properties. The fake brokers (`refusing_broker`, `slow_broker`, `closing_broker`)
  answer in the client's protocol version.
- GUI tests use `egui_kittest`, which finds widgets by accessible label: give inputs a label via
  `.labelled_by(label.id)`. The test window is 800×600, and a widget outside it cannot be
  clicked.
- Build whole-window harnesses with `testing::window_builder()`: the engine and the disk check
  ask for repaints from other threads and the real clock, so `Harness::run` needs more than
  egui_kittest's default 4 frames to settle on a busy machine.

## Documentation

- Facts on `website/` must match the code; when behaviour changes, update the pages that describe
  it in the same change. Style rules: `.superpowers/sdd/2026-10-15-senzor-docs-site/writing-guide.md`
  (local).
- The MCP tool reference (`website/content/docs/reference/mcp-tools.mdx`) and the tool
  descriptions in `crates/senzor-mcp/src/server.rs` are written by hand; keep both in step with
  the tools. The docs test only checks that every tool is listed.
- After a visible GUI change, regenerate the screenshots:
  `cargo run -p senzor --example screenshots --features screenshots`.
