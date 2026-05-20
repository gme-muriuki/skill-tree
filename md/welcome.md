# Contributing to skill-tree

Welcome! This section is for people who want to work on skill-tree itself. If you're a user of skill-tree, see the [getting started guide](../introduction.md) instead.

## Building and testing

skill-tree is a standard Cargo project with a library and a binary:

```bash
cargo check              # type-check
cargo test               # run the test suite
cargo run -- render      # run locally: render command
```

Tests use snapshot assertions via the `expect-test` crate. If a snapshot changes, run with `UPDATE_EXPECT=1` to update it:

```bash
UPDATE_EXPECT=1 cargo test
```

## Development setup

You'll need:

- Rust (latest stable)
- `graphviz` package (for the `dot` binary, used by render tests)

On macOS:
```bash
brew install graphviz
```

On Ubuntu:
```bash
apt install graphviz
```

## Logging and debugging

skill-tree uses `eprintln!` for diagnostic output during development. Errors use the `thiserror` crate for structured error types.

To debug a command:
```bash
RUST_BACKTRACE=1 cargo run -- render --verbose
```

For unit tests:
```bash
cargo test -- --nocapture
```

## Code style

Follow the established patterns from the codebase:

- Comments explain the "why", not the "what"
- Code is self-documenting; avoid redundant comments
- Each error type knows its own exit code
- Use `match` statements instead of `matches!` when binding multiple variables
- Tests are organized by concern (config tests in config.rs, GitHub client tests in github/mod.rs)

## What to read next

- [Key modules](./01-module-structure.md) — the major pieces of skill-tree
- [Important flows](./02-important-flows.md) — how each command works
- [Common issues](./03-common-issues.md) — known limitations and gotchas
- [Design docs](../design/) — architecture decisions and design philosophy

## Getting started on a task

1. Pick an issue or identify something to improve
2. Read the relevant design doc (e.g., [GitHub client design](../design/02-github-client.md))
3. Check the [key modules](./01-module-structure.md) to understand the code structure
4. Write a failing test first, then implement
5. Run `cargo test` to make sure everything passes
6. Open a PR with a clear description of what changed and why

## What we're building

skill-tree turns a GitHub Project into a visual dependency graph. It's a tool for thinking about project roadmaps—seeing at a glance which issues are blocking others, which are ready to start, and how the work flows together.

The codebase is intentionally kept small and focused. We ship features that work well, not everything that could be imagined.