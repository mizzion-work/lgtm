# lgtm

**the diff tool that lets you say lgtm with confidence.**

`lgtm` is a cross-platform GUI diff and merge tool inspired by Meld, with
first-class support for use as `git difftool` and `git mergetool`.

> from wtf to lgtm

## Status

Work in progress. v1 is being built top-down per the plan in the repo's
session prompt. This README will be filled out as features land
(install instructions, screenshots, troubleshooting).

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
