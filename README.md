# lgtm

**the diff tool that lets you say lgtm with confidence.**

`lgtm` is a cross-platform GUI diff and merge tool inspired by Meld, with
first-class support for use as `git difftool` and `git mergetool`.

> from wtf to lgtm

```
  ╦  ╔═╗╔╦╗╔╦╗
  ║  ║ ╦ ║ ║║║
  ╩═╝╚═╝ ╩ ╩ ╩
  from wtf to lgtm
```

## What it does

- **Two-file diff** with side-by-side synchronized scrolling, word-level
  highlighting inside replaced lines, hunk navigation, and a clickable
  minimap of all changes.
- **Live editing** of either pane with a 150 ms debounced re-diff;
  Ctrl+S saves back to disk preserving the original encoding and line
  endings.
- **Three-way merge** built for `git mergetool`. Each conflict gets
  Take LOCAL / Take REMOTE / Take BOTH (either order) / Take BASE
  buttons and an editable per-conflict resolution. Saving with
  unresolved conflicts requires explicit confirmation.
- **Folder diff** that walks both trees respecting `.gitignore` by
  default, with status badges (`= ~ < > ! b`) and filters.
- **`--no-gui` mode** prints a unified diff to stdout — handy for
  scripting or piping to `less`.

## Install

```
cargo install --path crates/lgtm-cli
```

After install, `lgtm --version` should print the banner.

## Use as `git difftool`

```bash
git config --global diff.tool lgtm
git config --global difftool.lgtm.cmd 'lgtm "$LOCAL" "$REMOTE"'
git config --global difftool.prompt false
```

Now `git difftool HEAD~1` opens `lgtm` for each changed file.

For directory mode (`git difftool -d`) lgtm detects two directory
arguments automatically and opens its folder-diff view.

## Use as `git mergetool`

```bash
git config --global merge.tool lgtm
git config --global mergetool.lgtm.cmd 'lgtm --merge "$LOCAL" "$BASE" "$REMOTE" --output "$MERGED"'
git config --global mergetool.lgtm.trustExitCode true
```

`trustExitCode true` lets `lgtm` tell git whether the merge succeeded
(exit 0) or was aborted (exit 1). Without it, git falls back to
inspecting the merged file's mtime, which is less reliable.

`just setup-git` prints the same snippet so you can copy it out.

## CLI

```
lgtm <LEFT> <RIGHT>                                      # two-file or two-folder diff (auto-detect)
lgtm --merge <LOCAL> <BASE> <REMOTE> --output <MERGED>   # three-way merge
lgtm --read-only <LEFT> <RIGHT>                          # diff without edit
lgtm --no-gui <LEFT> <RIGHT>                             # print unified diff to stdout
lgtm --dir  <LEFT> <RIGHT>                               # force folder mode
lgtm --file <LEFT> <RIGHT>                               # force file mode
lgtm --quiet ...                                         # suppress "LGTM ✓" on success
```

### Exit codes

| Code | Meaning                                                            |
|------|--------------------------------------------------------------------|
| 0    | files identical, or user saved merge result successfully           |
| 1    | files differ (diff mode), or user quit merge without saving        |
| 2    | error (file not found, permission denied, invalid arguments, etc.) |

When `lgtm` exits 0 because everything matched (or because the merge
saved), it prints `LGTM ✓` to stderr unless `--quiet` is passed.

## Keyboard shortcuts (diff view)

| Key                 | Action                                |
|---------------------|---------------------------------------|
| `n` / `p`           | next / previous hunk                  |
| `Ctrl+Home` / `End` | first / last hunk                     |
| `Ctrl+S`            | save modified panes                   |
| `Esc` / `q`         | close window (confirms if dirty)      |

## Headless merge (test / CI)

`lgtm` supports a non-interactive merge mode for end-to-end testing of
the `git mergetool` integration without a display. Set
`LGTM_HEADLESS_MERGE` to one of:

- `auto` — save only if every conflict is already auto-resolved
- `take-local` — resolve every conflict by taking LOCAL
- `take-remote` — resolve every conflict by taking REMOTE
- `abort` — exit 1 without writing the output

The test suite uses this mode to drive `git mergetool` against a real
git repo from inside `cargo test`.

## Workspace layout

```
lgtm/
├── Cargo.toml
├── justfile
├── crates/
│   ├── lgtm-core/   diff engine, file I/O, folder walking — no UI deps
│   ├── lgtm-gui/    egui app, all view code
│   └── lgtm-cli/    binary; arg parsing, dispatches to gui or stdout
└── tests/integration/
```

## Building & testing

```
just build    # release build of the lgtm binary
just test     # workspace tests, including end-to-end git mergetool tests
just lint     # cargo fmt --check + cargo clippy -D warnings
just install  # cargo install --path crates/lgtm-cli
just setup-git
```

## Troubleshooting difftool / mergetool

- **`git mergetool` always says "merged successfully" even when I aborted.**
  You probably forgot `mergetool.lgtm.trustExitCode true`. Without it,
  git inspects mtime instead of exit code and a no-op `lgtm` will look
  like success.

- **Paths with spaces don't open.**
  Make sure the `cmd` is single-quoted as shown above and that `$LOCAL`
  / `$REMOTE` / `$BASE` / `$MERGED` stay in double quotes inside the
  command. On Windows / PowerShell, use the equivalent quoting for your
  shell.

- **Windows path quoting.**
  PowerShell needs backtick escapes; cmd.exe needs `"%LOCAL%"`. Use a
  `.cmd` shim if your git config doesn't expand the variables the way
  you want.

- **`lgtm` reports "Binary files differ" for a text file.**
  We detect binary content via `content_inspector`, which trips on any
  NUL byte in the first 1 KB. If your file has a NUL byte but is really
  text, that's the cause. We don't currently have a flag to force-text;
  open an issue.

- **Huge files are slow.**
  Files larger than 50 MB get a warning; files larger than 500 MB are
  refused outright. If you need to diff something larger, consider
  pre-filtering with `head` / `grep` and diffing the slices.

## CI

A GitHub Actions matrix like the following is enough to get cross-platform
coverage:

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]
runs-on: ${{ matrix.os }}
steps:
  - uses: actions/checkout@v4
  - uses: dtolnay/rust-toolchain@stable
  - run: cargo test --workspace --all-targets
  - run: cargo clippy --workspace --all-targets -- -D warnings
  - run: cargo fmt --all -- --check
```

The end-to-end `git mergetool` tests require `git` on PATH; all GitHub
runners ship it, so no extra setup is needed.

## Screenshot

_(placeholder — drop a PNG here once you build the binary; the merge
view in particular looks dramatically less abstract with a screenshot)._

## License

Dual-licensed under MIT or Apache-2.0.
