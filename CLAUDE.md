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
- Work on a branch and fast-forward `main` when asked. Pushing `main` deploys the docs site
  (Cloudflare Workers Builds), so push only when asked.
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
- Real-broker tests are `#[ignore]`d: `docker compose -f dev/compose.yaml up -d`, then
  `cargo test --workspace -- --ignored` (see `dev/README.md`). Run them after engine or MQTT
  changes.
- To see or try a GUI change in the real app, drive it with `dev/gui.sh` (virtual display,
  screenshots, clicks and typing; see `dev/README.md`). Read every screenshot you take.
- Before using a library API, check the version in `Cargo.lock` and its current docs; add only
  dependency versions that are at least a couple of weeks old.

## Architecture rules

- Terms: a **template** defines topics, payloads and fields; a **device** (`[[devices]]`) uses a
  template with params and a `count`; each copy is an **instance** with its own MQTT connection.
- `senzor-core` has no egui. The GUI and MCP edit the project through `Document::edit` and the
  shared operations in `senzor-core::project::edit`, so both hosts behave the same.
- Validation collects every problem with a location (broker, template, device) and blocks only
  what it affects; there are no warnings. A problem's text is what users see in the Problems tab
  and the CLI, and `website/content/docs/reference/validation-errors.mdx` lists each one.
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

## Documentation

- Facts on `website/` must match the code; when behaviour changes, update the pages that describe
  it in the same change. Style rules: `.superpowers/sdd/2026-10-15-senzor-docs-site/writing-guide.md`
  (local).
- The MCP tool reference (`website/content/docs/reference/mcp-tools.mdx`) and the tool
  descriptions in `crates/senzor-mcp/src/server.rs` are written by hand; keep both in step with
  the tools. The docs test only checks that every tool is listed.
- After a visible GUI change, regenerate the screenshots:
  `cargo run -p senzor --example screenshots --features screenshots`.
