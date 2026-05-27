# lgtm-gui-gpui — gpui migration scaffold

A second GUI backend for `lgtm` using [gpui](https://gpui.rs), Zed's
GPU-accelerated UI framework, sitting alongside the existing eframe-based
`lgtm-gui`. This crate is **excluded from the default workspace** (see
`workspace.exclude` in the repo-root `Cargo.toml`) because gpui doesn't
currently resolve as a standalone crates.io dependency — see _Building_
below.

## Migration status

Ported on this branch:

- `run_diff(left, right, read_only)` — the entry point matching
  `lgtm_gui::run_diff`'s shape (without the `repo` / `editor` plumbing
  yet — those land later).
- `DiffView` — a single-window read-only side-by-side diff with
  monospace text, line numbers in the gutter, and diff-status background
  tints (insert green / delete red / replace yellow). Sync scrolling is
  free since both columns live inside one outer scroll container.
- `GuiOutcome { Identical, Differs }` — same shape as the eframe
  backend so the CLI dispatch in `crates/lgtm-cli/src/main.rs` is a
  one-liner match.

Not yet ported (each is its own follow-up patch):

| Feature | Status |
|---|---|
| Edit mode + debounced re-diff | TODO |
| Find bar (with regex / case toggles) | TODO |
| Blame on hover | TODO |
| Per-hunk Copy → / ← buttons | TODO |
| Hunk navigation (`n` / `p`, minimap) | TODO |
| Open in editor (`e`) | TODO |
| Menubar (File / Edit / View / Help) | TODO |
| Three-way merge view | TODO |
| Folder-diff view | TODO |
| Git graph drawer | TODO |
| Settings persistence + theme switch | TODO |
| Syntax highlighting (`syntect` integration) | TODO |
| Drag-and-drop, large-file confirm | TODO |

The data model in `lgtm-core` is reused verbatim — both GUI backends
consume the same `AlignedDiff`, `BlameCache`, `FindState`, `Settings`,
`Graph`, etc., so this port is strictly a presentation-layer rewrite.

## Why gpui is excluded from the default workspace

The `gpui` 0.2.2 release on crates.io declares a sub-dependency on
`gpui_platform`, which Zed publishes only inside their own workspace.
Pulling `gpui` via a git dep from `zed-industries/zed` succeeds at
fetching the sub-crate but trips an unresolvable version conflict
during cargo's dependency resolution:

```
error: failed to select a version for `core-foundation`.
    ... required by package `cocoa v0.26.0`
    ... which satisfies dependency `cocoa = "=0.26.0"` of package `gpui`
versions that meet the requirements `^0.10` (locked to 0.10.1) are: 0.10.1
  previously selected package `core-foundation v0.10.0`
    ... which satisfies dependency `core-foundation = "=0.10.0"` of package `gpui`
```

The root cause: zed's workspace pins `core-foundation = "=0.10.0"` and
`cocoa = "=0.26.0"`. Inside zed's workspace those resolve fine. Pulled
out, cocoa's own crates.io Cargo.toml requires `core-foundation ^0.10`
which cargo locks to the latest patch (0.10.1), and the `=0.10.0` pin
from gpui's manifest is irreconcilable.

This is a macOS-platform pin even though the failure also blocks Linux
builds, since cargo resolves all-target dep graphs eagerly.

Workarounds (none yet applied here):

1. **Replicate zed's `[patch.crates-io]` block** for all transitively
   conflicting crates. Practical but invasive — would patch cocoa,
   core-foundation, and likely several others — and would need to be
   refreshed every time we bump the gpui rev.
2. **Vendor gpui sources** as a `path` dep into the workspace. Works,
   but pulls ~150k LOC into the repo for what should be a single
   dependency.
3. **Wait for gpui to ship a clean crates.io publish.** The gpui team
   has signaled (via README and `gpui.rs`) that this is the plan; not
   on a published timeline yet.

## Building this crate today

You need a checkout of `zed-industries/zed` to act as a path-dep source.

```bash
# 1. Clone zed alongside the lgtm checkout.
git clone --depth 1 https://github.com/zed-industries/zed.git ../zed

# 2. Edit the repo-root Cargo.toml: re-include the crate, swap the
#    gpui / gpui_platform git deps for paths.
#
#    [workspace]
#    members = [
#        "crates/lgtm-core",
#        "crates/lgtm-gui",
#        "crates/lgtm-gui-gpui",   <-- re-add
#        "crates/lgtm-cli",
#    ]
#    # remove the exclude
#
#    [workspace.dependencies]
#    gpui          = { path = "../zed/crates/gpui",          default-features = false, features = ["x11", "wayland"] }
#    gpui_platform = { path = "../zed/crates/gpui_platform", default-features = false, features = ["wayland", "x11"] }

# 3. Re-wire the CLI dispatch — see the commented-out match-arm
#    in crates/lgtm-cli/src/main.rs (search for "GuiBackend::Gpui").

# 4. Build.
cargo build -p lgtm-gui-gpui
LGTM_LOG=debug ./target/debug/lgtm --backend gpui ./a.txt ./b.txt
```

The first build pulls a large transitive dep set (gpui itself; cosmic-text;
blade-graphics; wayland / x11 / xkbcommon; live_kit_client + its
WebRTC bindings) — expect ~3-5 minutes on a warm machine and ~1.5 GB of
cargo cache.

## API shape

`run_diff(left, right, read_only) -> Result<GuiOutcome>` mirrors the
eframe entry point. The `DiffView` is a long-lived gpui `Entity`; its
`Render` impl returns a tree of styled `div()`s.

Key gpui idioms used here:

- `gpui_platform::application().run(|cx: &mut App| { ... })` — the
  top-level event loop, called from `run_diff`.
- `cx.open_window(WindowOptions, |window, cx| { cx.new(|_| view) })`
  — spawn the window with our root view.
- `Render for DiffView` — the per-frame layout function returning
  `impl IntoElement`.
- `div().flex().flex_col().bg(rgb(...)).child(...)` — Tailwind-like
  styling chain.
- `.children(rows)` — children take an `IntoIterator` of elements.

Compared to egui's immediate-mode `update(ctx)`-per-frame model, gpui
is retained-mode: views live as model entities, and the framework
invalidates and re-renders only when state changes. The diff doesn't
change frame-to-frame, so this is a noticeably better fit for a
diff/merge tool than egui's "render everything every frame" pattern —
once the rest of the surface is ported.

## Next steps

The migration plan order, in roughly increasing scope:

1. Sync the diff view with `Settings.font_size` + `Settings.app_theme`.
2. Wire find (just the bar UI, search already lives in `lgtm-core`).
3. Port the per-hunk Copy buttons.
4. Port edit mode — gpui's `Editor` widget (from `editor` crate inside
   Zed) is the obvious building block, but it's not on crates.io either.
   May require a smaller `EditableText` shim.
5. Menubar — gpui has menus built in, mapping should be direct.
6. Blame on hover — re-use the existing `BlameCache`.
7. Merge / folder views.
8. Settings persistence + theme switching (already in `lgtm-core`).
