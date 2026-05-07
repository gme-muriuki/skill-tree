# Common issues

## `dot` binary not found

`skill-tree render --format svg` shells out to the system `dot` binary.
If Graphviz is not installed, the command fails with an actionable message:

```
Graphviz `dot` binary not found.
Install it with:
  macOS:  brew install graphviz
  Ubuntu: apt install graphviz
  Windows: https://graphviz.org/download/
```

DOT output (`--format dot`) does not require Graphviz and always works.

## `GITHUB_TOKEN` not set

```
GITHUB_TOKEN environment variable is not set.
Create a token at https://github.com/settings/tokens and run:
export GITHUB_TOKEN=<your token>
```

The token needs `read:project` and `repo` scopes.

## `github-name` does not match the field in GitHub Projects

The `github-name` in every `[[field]]` block and in `[colors]` must
match the field name exactly as it appears in GitHub Projects,
including case. If the field is named `"Status"` in GitHub and your
config says `"status"`, skill-tree will find no values for that field
and all nodes will render with the default gray color.

To find the exact name: open your GitHub Project, click the field
header, and copy the name character for character into your config.

## Node colors not appearing

If nodes render gray even though status values are set in GitHub,
check two things:

1. `[colors] github-name` matches a field declared in `[[field]]`
   with the same `github-name`.
2. The keys in `[colors.values]` match the option names in the
   GitHub Projects single-select field exactly, including case
   and spacing.

## Pagination silently truncating results

If your project has more than 100 issues and some are missing from
the output, check that the fetch loop is following `hasNextPage`
correctly. The `fetch_project_items` function in `github/projects.rs`
is responsible for this. Run with `--verbose` to see how many pages
were fetched.

## Cycle detected on valid graph

If `validate` reports a cycle that does not exist, check whether the
same issue number appears as both a blocking issue and a sub-issue.
The graph builder in `graph/mod.rs` treats both as edges. A
self-referencing sub-issue will produce a false cycle report.