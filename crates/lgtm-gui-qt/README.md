# lgtm-gui-qt — Qt migration scaffold

A third GUI backend for `lgtm` using [Qt 6](https://www.qt.io) via the
[`cxx-qt`](https://github.com/KDAB/cxx-qt) Rust bindings, sitting
alongside the existing eframe-based `lgtm-gui` and the gpui scaffold
`lgtm-gui-gpui`. This crate is **excluded from the default workspace**
(see `workspace.exclude` in the repo-root `Cargo.toml`) because Qt is a
large C++ runtime that has to be supplied by the host system (or a
pre-built SDK) — it isn't a normal cargo dep. See _Building_ below.

## Why cxx-qt (over qmetaobject)?

The two viable Rust → Qt bindings today are:

- **`cxx-qt`** (KDAB, current). Built on the `cxx` FFI framework, targets
  Qt 6, actively maintained, ships its own `cxx-qt-build` driver, has a
  recent crates.io release, and projects Rust types as first-class
  `QObject`s with QML registration via attribute macros.
- **`qmetaobject`** (woboq, older). Predates `cxx`, targets Qt 5 +
  earlier Qt 6, smaller surface area, slower-moving maintenance.

We picked **cxx-qt**: Qt 6 is the current LTS line; cxx-qt is what KDAB
recommends for new projects; the `#[cxx_qt::bridge]` macro keeps the
"this is a QObject" boilerplate inside the Rust source instead of a
separate `.h`/`.cpp` shim; and the build story (`cxx-qt-build` invoked
from `build.rs`) is the closest a Rust+Qt project gets to "just cargo
build" given the C++ runtime.

## Migration status

Ported on this branch:

- `run_diff(left, right, read_only)` — the entry point matching
  `lgtm_gui::run_diff`'s shape (without the `repo` / `editor` plumbing
  yet — those land later), mirroring what `lgtm-gui-gpui` already did.
- `DiffView` — a single-window read-only side-by-side diff with
  monospace text, line numbers in the gutter, and diff-status background
  tints (insert green / delete red / replace yellow). Implemented as a
  cxx-qt `QObject` projected to a QML `ListView` for the rows. Sync
  scrolling is free since both columns live inside a single outer
  `Flickable`.
- `GuiOutcome { Identical, Differs }` — same shape as the eframe and
  gpui backends so the CLI dispatch in `crates/lgtm-cli/src/main.rs` is
  a one-liner match.

Not yet ported (each is its own follow-up patch — same list as gpui):

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

The data model in `lgtm-core` is reused verbatim — all three GUI
backends consume the same `AlignedDiff`, `BlameCache`, `FindState`,
`Settings`, `Graph`, etc., so this port is strictly a presentation-layer
rewrite.

## Why Qt is excluded from the default workspace

Qt is a C++ runtime, not a crate. `cxx-qt-build` invokes Qt's `moc`,
`rcc`, and `qmlcachegen` tools and links against `QtCore`, `QtGui`,
`QtQml`, and `QtQuick` (≥ 6.5). None of that ships with cargo. The
sandbox CI and most contributors don't have a Qt 6 install on their
PATH, so adding this crate to `workspace.members` would break
`cargo check --workspace` for everyone who hasn't run the Qt installer.

Workarounds (none yet applied here):

1. **System packages.** `apt install qt6-base-dev qt6-declarative-dev`
   on Debian/Ubuntu; equivalents on Fedora (`qt6-qtbase-devel`,
   `qt6-qtdeclarative-devel`), Arch (`qt6-base`, `qt6-declarative`),
   Homebrew (`brew install qt`), or the official online installer on
   Windows. Practical for individual contributors; opaque for CI.
2. **Pre-built Qt SDK + `QMAKE` env var.** Point `cxx-qt-build` at a
   vendored Qt by exporting `QMAKE=/path/to/qt6/bin/qmake6`. Reliable
   but the SDK download is multi-hundred-MB.
3. **Wait for a fully cargo-installable Qt build.** Not on any
   timeline; conceptually unlikely given Qt's size and the LGPL/dynamic
   linking story.

## Building this crate today

```bash
# 1. Install Qt 6 (≥ 6.5) with QML.
#    Debian / Ubuntu:
sudo apt install qt6-base-dev qt6-declarative-dev qml6-module-qtquick \
                 qml6-module-qtquick-controls qml6-module-qtquick-layouts
#    macOS (Homebrew):
brew install qt
#    Or use the official online installer from qt.io.

# 2. If qmake6 isn't on PATH (typical for Homebrew / the online
#    installer), point cxx-qt-build at it:
export QMAKE=/opt/homebrew/opt/qt/bin/qmake6
# Some cxx-qt-build versions also honor QT_INCLUDE_PATH and
# QT_LIBRARY_PATH for non-standard layouts.

# 3. Edit the repo-root Cargo.toml: re-include the crate, drop the
#    exclude.
#
#    [workspace]
#    members = [
#        "crates/lgtm-core",
#        "crates/lgtm-gui",
#        "crates/lgtm-gui-qt",   # <-- re-add
#        "crates/lgtm-cli",
#    ]
#    exclude = ["crates/lgtm-gui-gpui"]   # remove the qt entry

# 4. Re-wire the CLI dispatch — see the commented-out match-arm in
#    crates/lgtm-cli/src/main.rs (search for "GuiBackend::Qt").

# 5. Build.
cargo build -p lgtm-gui-qt
LGTM_LOG=debug ./target/debug/lgtm --backend qt ./a.txt ./b.txt
```

The first build pulls cxx, cxx-qt, cxx-qt-lib, cxx-qt-build and a
modest cmake/moc step — expect ~1-2 minutes on a warm machine. The
finished binary dynamically links against the system Qt 6, so end
users also need Qt installed (or a bundled Qt deployment via
`windeployqt` / `macdeployqt` / `linuxdeployqt`).

## API shape

`run_diff(left, right, read_only) -> Result<GuiOutcome>` mirrors the
eframe and gpui entry points. Internally it:

1. Pre-flattens `AlignedDiff::rows` into a `Vec<RowVm>` — color, line
   numbers, and stripped text per row.
2. Constructs a cxx-qt `DiffViewModel` QObject wrapping those rows and
   the title/status banners.
3. Spawns a `QGuiApplication` + `QQmlApplicationEngine`, binds the
   model as a context property called `diff`, and loads
   `qml/DiffWindow.qml`. (The QML resource lives under `qrc:/qt/qml/lgtm/`
   once `cxx-qt-build` registers it; see cxx-qt's `qml_module` docs.)
4. Runs the Qt event loop until the window closes.

Key cxx-qt idioms used:

- `#[cxx_qt::bridge]` module — declares the `extern "RustQt"` block
  exposing the QObject + its `qproperty`s + `qinvokable`s. `cxx-qt-build`
  generates the C++ shell and runs `moc` over it.
- `#[qml_element]` — auto-registers the QObject as a QML type so QML
  can refer to it without a separate `qmlRegisterType` call.
- `Self::default_boxed()` + `.rust_mut()` — the cxx-qt 0.7 pattern for
  populating non-`qproperty` Rust fields after construction.

Compared to the egui frontend (immediate-mode redraw every frame) and
the gpui frontend (retained-mode element tree re-rendered on
invalidation), Qt is retained-mode with property bindings: QML re-runs
only the bindings whose dependencies changed. For a diff view that's
mostly static after load this is the cheapest of the three at idle.

## Next steps

The migration plan order, in roughly increasing scope:

1. Sync the diff view with `Settings.font_size` + `Settings.app_theme`.
2. Wire find (just the QML overlay UI, search already lives in
   `lgtm-core`).
3. Port the per-hunk Copy buttons (`Button` in QML, signal → Rust
   `qinvokable`).
4. Port edit mode — Qt's `TextEdit` / `QQuickTextDocument` is a direct
   building block; round-trip diff through `lgtm-core`'s editor module.
5. Menubar — QML `MenuBar` maps directly to the eframe menubar layout.
6. Blame on hover — re-use the existing `BlameCache`; show via a QML
   `ToolTip` bound to row hover.
7. Merge / folder views — second + third top-level `.qml` windows
   driven by analogous `QObject` view-models.
8. Settings persistence + theme switching (already in `lgtm-core`).
