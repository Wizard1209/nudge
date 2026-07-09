# Nudge — Electron prototype

This directory is the **prototyping harness** for Nudge. It is a full,
feature-parity implementation of the same spec as the Rust app, built on
Electron + Svelte because that stack is fast to iterate on and easy to inspect
visually. It is a workbench, not a second product.

## Workflow

1. **Prototype here.** Try a new feature or a design change in this Electron app
   first — it is quick to build, quick to run, and easy to eyeball.
2. **Capture it in the spec.** Once the behavior is settled, write it into the
   shared spec under [`../docs/`](../docs) (`design-spec.md`, `journal-spec.md`).
   The spec is the contract both implementations follow.
3. **Port to Rust.** The Rust app (the repo root) is then updated to match, from
   the spec plus this prototype as the working reference.

One commit is one spec state: the two implementations and the spec move together.
`package.json`'s `version` tracks the Rust `Cargo.toml` version.

## Non-goals

This code intentionally holds a **low quality bar**. It carries no requirements
for performance, memory footprint, stability, code structure, or documentation
depth, and it is not gated by CI. It is fully rewritable from the spec at any
time. The Rust binary at the repo root is the released product and the one that
holds the quality bar.

## Stack

- **Electron** main process + **Svelte 5** renderer, bundled with **Vite**.
- Tailwind for styling; packaged for Windows with **electron-builder**.

## Running it

Requires Node (with [pnpm](https://pnpm.io); `corepack enable` will provide it)
and, for the browser/native test tiers, a display (WSLg/X11).

```sh
pnpm install          # install dependencies

pnpm dev              # run the renderer in a Vite dev server
pnpm dev:electron     # run the full Electron app against the dev server

pnpm dist             # build a Windows installer + portable .exe (release/)
```

## Tests

```sh
pnpm typecheck        # tsc --noEmit
pnpm test:unit        # pure logic (node, sandbox-friendly)
pnpm test:e2e         # renderer behavior via Puppeteer (needs a display)
pnpm test:native      # real Electron app via Playwright (needs a display, built assets)
```

The testing tiers, when to use each, and the manual native checklist are
described in [`docs/testing.md`](docs/testing.md) and
[`docs/manual-verification.md`](docs/manual-verification.md).
