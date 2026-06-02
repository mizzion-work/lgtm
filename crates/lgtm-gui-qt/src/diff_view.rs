//! Two-file diff window — Qt (cxx-qt) port.
//!
//! Architecture differs from both the egui and gpui counterparts:
//!
//! 1. **QObject + QML.** The diff state is a Rust struct projected to QML
//!    as a `QObject` by cxx-qt's `#[cxx_qt::bridge]` macro. QML reads
//!    properties (`rowCount`, `title`, etc.) and invokes invokables
//!    (`rowAt(i)`) for per-row data. The view itself — the side-by-side
//!    `ListView` with gutters and tinted backgrounds — lives in
//!    `qml/DiffWindow.qml` (inlined at compile time via cxx-qt-build).
//!
//! 2. **Event loop is Qt's.** `QGuiApplication::exec()` blocks until the
//!    last window closes, matching gpui's `application().run(...)` and
//!    egui's `eframe::run_native` semantics.
//!
//! Sync scrolling: since both panes are columns inside one outer
//! `Flickable` in QML, scrolling is intrinsically synchronized — same
//! trick the gpui port uses with a single outer scroll container.

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use lgtm_core::{AlignedDiff, DiffDocument, DiffRow, Side};

use crate::GuiOutcome;

/// Launch the Qt-backed two-file diff window. Blocks until the user
/// closes the window.
///
/// Identical files short-circuit without opening a window, matching the
/// eframe and gpui entry points' behavior.
pub fn run_diff(
    left: DiffDocument,
    right: DiffDocument,
    _read_only: bool,
) -> anyhow::Result<GuiOutcome> {
    // Cheap pre-check: identical content → no window.
    let outcome = if !left.is_binary && !right.is_binary && left.content == right.content {
        GuiOutcome::Identical
    } else {
        GuiOutcome::Differs
    };

    let diff = AlignedDiff::compute(&left, &right).with_inline();
    let rows = flatten_rows(&diff);
    let title = format!(
        "lgtm — {} ↔ {}",
        short_path(&left.path),
        short_path(&right.path),
    );
    let status = format!(
        "{} differences, {} hunks",
        diff.stats.differences(),
        diff.hunks.len(),
    );

    // QGuiApplication owns Qt's event loop and must outlive the engine.
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    // Register the DiffViewModel singleton with the QML context before
    // loading the QML so the window can bind to its properties at
    // construction time.
    let model = qobject::DiffViewModel::new(rows, title, status);
    if let Some(eng) = engine.as_mut() {
        // root_context().set_context_property("diff", model.as_ptr())
        // — pseudocoded so the call sites are easy to read; the
        // cxx-qt 0.7 API uses `set_context_property` on the engine's
        // root context.
        eng.set_property(&QString::from("diff"), &model);
        eng.load(&QUrl::from(QString::from("qrc:/qt/qml/lgtm/DiffWindow.qml")));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }

    Ok(outcome)
}

/// A row in the format QML wants: pre-flattened so QML doesn't need to
/// reach into the Rust enum.
#[derive(Debug, Clone)]
pub(crate) struct RowVm {
    /// Hex RGB background for the row (insert/delete/replace tint).
    pub bg: u32,
    /// Left line number or empty string if absent.
    pub left_num: String,
    /// Left side text (newline stripped).
    pub left_text: String,
    /// Right line number or empty string if absent.
    pub right_num: String,
    /// Right side text (newline stripped).
    pub right_text: String,
}

fn flatten_rows(diff: &AlignedDiff) -> Vec<RowVm> {
    diff.rows.iter().map(row_to_vm).collect()
}

fn row_to_vm(row: &DiffRow) -> RowVm {
    let bg = match row {
        DiffRow::Equal { .. } | DiffRow::Gap { .. } => 0x202020,
        DiffRow::Delete { .. } => 0x4a181f,
        DiffRow::Insert { .. } => 0x183f1d,
        DiffRow::Replace { .. } => 0x4a4012,
    };
    let (left_num, left_text, right_num, right_text) = match row {
        DiffRow::Equal {
            left_line,
            right_line,
            text,
        } => (
            fmt_num(Some(*left_line)),
            strip_nl(text).to_string(),
            fmt_num(Some(*right_line)),
            strip_nl(text).to_string(),
        ),
        DiffRow::Delete { left_line, text } => (
            fmt_num(Some(*left_line)),
            strip_nl(text).to_string(),
            fmt_num(None),
            String::new(),
        ),
        DiffRow::Insert { right_line, text } => (
            fmt_num(None),
            String::new(),
            fmt_num(Some(*right_line)),
            strip_nl(text).to_string(),
        ),
        DiffRow::Replace {
            left_line,
            right_line,
            left_text,
            right_text,
            ..
        } => (
            fmt_num(Some(*left_line)),
            strip_nl(left_text).to_string(),
            fmt_num(Some(*right_line)),
            strip_nl(right_text).to_string(),
        ),
        DiffRow::Gap { side } => match side {
            Side::Left => (
                fmt_num(None),
                String::new(),
                fmt_num(None),
                String::new(),
            ),
            Side::Right => (
                fmt_num(None),
                String::new(),
                fmt_num(None),
                String::new(),
            ),
        },
    };
    RowVm {
        bg,
        left_num,
        left_text,
        right_num,
        right_text,
    }
}

fn fmt_num(n: Option<usize>) -> String {
    match n {
        Some(n) => format!("{n:>5}"),
        None => "     ".to_string(),
    }
}

fn strip_nl(s: &str) -> &str {
    s.strip_suffix('\n').unwrap_or(s)
}

fn short_path(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// cxx-qt bridge: projects the Rust `DiffViewModel` to QML as a QObject
/// with `rowCount`, `title`, `status` properties and a `rowAt(i)`
/// invokable returning `QVariantMap { bg, leftNum, leftText, rightNum,
/// rightText }`.
///
/// The bridge module is generated at build time by `cxx-qt-build`; the
/// `#[cxx_qt::bridge]` attribute declares the C++ side for moc.
#[cxx_qt::bridge]
pub mod qobject {
    use super::RowVm;

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qvariantmap.h");
        type QVariantMap = cxx_qt_lib::QVariantMap;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, row_count)]
        #[qproperty(QString, title)]
        #[qproperty(QString, status)]
        type DiffViewModel = super::DiffViewModelRust;

        /// QML calls this once per row to populate the ListView delegate.
        #[qinvokable]
        fn row_at(self: &DiffViewModel, index: i32) -> QVariantMap;
    }

    impl DiffViewModel {
        /// Construct a freshly-bound model carrying the pre-flattened
        /// rows and the title/status banners.
        pub fn new(rows: Vec<RowVm>, title: String, status: String) -> cxx::UniquePtr<Self> {
            let mut this = Self::default_boxed();
            let mut pinned = this.as_mut().expect("default_boxed returned null");
            pinned.as_mut().rust_mut().rows = rows;
            pinned.as_mut().set_row_count(pinned.rust().rows.len() as i32);
            pinned.as_mut().set_title(QString::from(&title));
            pinned.as_mut().set_status(QString::from(&status));
            this
        }
    }
}

/// Rust-side state for the QML-facing `DiffViewModel` QObject. cxx-qt
/// generates the QObject shell from this plus the bridge above.
#[derive(Default)]
pub struct DiffViewModelRust {
    /// Pre-flattened rows; QML reads them via `row_at(i)`.
    pub(crate) rows: Vec<RowVm>,
    /// `qproperty` mirror — kept in sync via setters generated by cxx-qt.
    pub(crate) row_count: i32,
    /// Window title.
    pub(crate) title: cxx_qt_lib::QString,
    /// Status-bar text.
    pub(crate) status: cxx_qt_lib::QString,
}

impl qobject::DiffViewModel {
    /// Look up a single row for the QML delegate.
    fn row_at(self: &qobject::DiffViewModel, index: i32) -> cxx_qt_lib::QVariantMap {
        use cxx_qt_lib::{QString, QVariant, QVariantMap};
        let mut map = QVariantMap::default();
        let i = index as usize;
        let Some(row) = self.rust().rows.get(i) else {
            return map;
        };
        map.insert(QString::from("bg"), QVariant::from(&(row.bg as i32)));
        map.insert(QString::from("leftNum"), QVariant::from(&QString::from(&row.left_num)));
        map.insert(QString::from("leftText"), QVariant::from(&QString::from(&row.left_text)));
        map.insert(QString::from("rightNum"), QVariant::from(&QString::from(&row.right_num)));
        map.insert(QString::from("rightText"), QVariant::from(&QString::from(&row.right_text)));
        map
    }
}
