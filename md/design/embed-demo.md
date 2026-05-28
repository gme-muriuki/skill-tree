# Embed demo (dogfood)

Live `embed` output from `rust-lang/59`, loaded into an `<iframe>` so the
widget runs as its own document — no scope-leak with the surrounding
mdbook chrome and no CommonMark blank-line surprises in the inlined
CSS/JS.

Regenerate before serving (gitignored; mdbook copies it through to
`book/design/embed-demo.html` on the next build):

```bash
$env:GITHUB_TOKEN = & gh auth token
cargo run -- embed --output md/design/embed-widget.html
```

<iframe src="embed-widget.html" width="100%" height="640" frameborder="0" style="border: 1px solid #d8dce0; border-radius: 12px;"></iframe>
