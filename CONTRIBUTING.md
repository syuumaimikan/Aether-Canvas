# Contributing

## Getting started

```sh
cargo test --workspace     # everything, including the end-to-end tests
cargo clippy --workspace --all-targets
cargo fmt --all
cargo run -p aether-desktop
```

## What "done" means here

A change is finished when the feature actually works. Concretely:

- **No placeholder UI.** If a button exists, clicking it does the thing. A
  feature that is not ready does not get a disabled control or a "coming soon"
  label — it gets left out and listed in [ROADMAP.md](ROADMAP.md).
- **No mock implementations.** Do not stub a function with `todo!()` and wire it
  into the UI.
- **Tests come with the code.** Especially for serialisation, undo/redo, layer
  operations, blending and anything with off-by-one risk.

## Code conventions

**Module size.** Keep files focused; split before a module becomes a grab bag.
No 2000-line `main.rs`.

**Errors.** Library code does not `unwrap()` or `expect()`. Return
`aether_core::Result` and give the message enough context to show a user —
`"layer 'Background' is locked"` beats `"invalid state"`. `expect()` is fine in
tests, where a panic *is* the failure report.

**Documentation.** Every public item gets a doc comment. Module-level docs
should explain *why* the module is shaped the way it is, not restate the type
names. Comments in the body are for decisions and non-obvious constraints, not
narration of the next line.

**User-visible strings.** Never inline. Add a key to both tables in
`aether-ui/src/i18n.rs`; a test enforces that they stay sorted and in sync.

**Editing the document.** Every user-triggered mutation goes through a
`Command`, so it can be undone and so it is reachable from a test without a
window. If you find yourself mutating `Document` directly from a widget, that
is the bug.

**Layer order.** Children are stored bottom-to-top. The layer panel shows them
top-to-bottom. Getting this backwards is the most common mistake in this
codebase; the tree tests will catch it.

## Testing conventions

- Name tests as sentences about behaviour: `clipping_layers_are_limited_to_the_base_alpha`.
- Assert on what a user would notice — pixel values, layer counts, what a second
  undo restores — rather than on internal fields.
- Include the failing value in the message when a bare assertion would be
  cryptic.
- Cross-crate behaviour belongs in `apps/desktop/tests/end_to_end.rs`.

## Pull requests

Explain what changed and why, note anything you deliberately left out, and say
how you verified it. If a phase in the roadmap is now further along, update it
in the same change.
