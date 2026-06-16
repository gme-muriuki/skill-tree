# Running tests

## Unit tests

```bash
cargo test
```

Runs all unit tests inside the `skill-tree` crate. No network access required.
No `GITHUB_TOKEN` required.

## Integration tests

```bash
cargo test --test integration
```

Runs the integration test suite in `tests/integration.rs`. Uses
`skill-tree-testlib` fixture builders exclusively. No network access required.
No `GITHUB_TOKEN` required.

## All tests

```bash
cargo test --workspace
```

Runs unit tests and integration tests across all crates in the workspace.

## Checking DOT output determinism

```bash
cargo test dot_output_is_valid_digraph
cargo test dot_output_contains_all_nodes
```

These tests assert byte-level properties of the DOT output. If you change
anything in `render/mod.rs`, run these first.