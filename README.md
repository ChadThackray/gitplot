# gitplot

A small desktop app that plots a git repository's lines-of-code over time.
Open a folder, watch the chart appear, then tick file extensions on or off
to slice the line by what you care about (e.g. exclude `.json` fixtures).

Built in Rust with [iced](https://iced.rs/) for the UI,
[git2](https://docs.rs/git2) for history walking,
[tokei](https://docs.rs/tokei) for code-line counting (skips blanks and
comments), and [plotters-iced](https://docs.rs/plotters-iced) for the chart.

## Build & run

```sh
cargo run --release
```

The first build compiles libgit2 from source (via `git2`'s
`vendored-libgit2` feature) so it takes a few minutes; subsequent builds
are fast.

## Usage

1. Click **Open repo** and pick any local git repository.
2. The status bar shows progress as commits are processed
   (`Walking 412/1893`).
3. When the walk finishes, the chart renders total LOC per day across the
   whole HEAD history.
4. The left sidebar lists every file extension found in the latest commit
   with its current LOC; toggle a checkbox to add or remove that extension
   from the chart. Toggles re-aggregate the cached per-day data instantly,
   no re-walk.

## What it counts

For every calendar day on `HEAD`, the last commit of that day is
selected. For each blob in that commit's tree:

- The language is inferred from the file extension (via tokei).
- If tokei recognises it, only **code lines** are counted (blanks and
  comments are skipped).
- If tokei doesn't recognise the extension, raw newlines are counted as a
  fallback. These show up in a `(no ext)` or `.<ext>` bucket and can be
  toggled off in the sidebar.
- Binary blobs are skipped.

## Development

```sh
cargo test                                      # integration tests
cargo clippy --all-targets                      # lint
cargo run --example walk_summary -- <repo>      # CLI dump of a repo's analysis
```
