//! `lgtm` — entry point.
//!
//! Parses arguments, dispatches to the GUI or to stdout, and maps the
//! result to a process exit code per the CLI contract documented in README.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use lgtm_core::{
    DiffDocument, FolderDiff, FolderDiffOptions, SOFT_SIZE_LIMIT, ThreeWayMerge, unified_diff,
};
use lgtm_gui::GuiOutcome;

const EXIT_IDENTICAL: u8 = 0;
const EXIT_DIFFERS: u8 = 1;
const EXIT_ERROR: u8 = 2;

const VERSION_BANNER: &str = "  ╦  ╔═╗╔╦╗╔╦╗
  ║  ║ ╦ ║ ║║║
  ╩═╝╚═╝ ╩ ╩ ╩
  from wtf to lgtm — v";

/// `lgtm` — from wtf to lgtm.
#[derive(Debug, Parser)]
#[command(name = "lgtm", version, about, long_about = None)]
struct Cli {
    /// Left side (or LOCAL in `--merge` mode).
    left: PathBuf,
    /// Right side (or BASE in `--merge` mode).
    right: Option<PathBuf>,
    /// REMOTE — only used with `--merge`.
    remote: Option<PathBuf>,

    /// Three-way merge mode. Requires three positional args and `--output`.
    #[arg(long)]
    merge: bool,
    /// Where to write the merged result (only with `--merge`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Open files without allowing edits.
    #[arg(long)]
    read_only: bool,
    /// Print a unified diff to stdout and exit.
    #[arg(long)]
    no_gui: bool,
    /// Force folder mode (otherwise auto-detected).
    #[arg(long)]
    dir: bool,
    /// Force file mode (otherwise auto-detected).
    #[arg(long)]
    file: bool,

    /// Suppress the "LGTM ✓" banner on successful exit.
    #[arg(long)]
    quiet: bool,

    /// Working-tree root for blame lookups and editor-target resolution.
    /// Typically `$(git rev-parse --show-toplevel)` in the git-difftool
    /// config snippet. When set, lgtm uses it to look up blame for
    /// difftool temp files and to open the real working-tree file
    /// (not the temp snapshot) when the user presses `e`.
    #[arg(long)]
    repo: Option<PathBuf>,

    /// Editor command for the `e` keybinding. Overrides $LGTM_EDITOR,
    /// $VISUAL, and $EDITOR. Use a quoted string for flags, e.g.
    /// `--editor "open -t"`.
    #[arg(long)]
    editor: Option<String>,

    /// Follow symbolic links when walking folders (with cycle detection).
    /// Only relevant in folder mode. Off by default.
    #[arg(long)]
    follow_symlinks: bool,
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LGTM_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    // Custom --version banner. We pre-parse before clap so the user sees
    // the lgtm ASCII art instead of the default crate-version line.
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{VERSION_BANNER}{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::from(0);
    }

    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("lgtm: error: {e:#}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn dispatch(cli: Cli) -> anyhow::Result<u8> {
    if cli.merge {
        return dispatch_merge(cli);
    }

    let right_path = cli
        .right
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing RIGHT argument"))?;

    let left_is_dir = is_dir(&cli.left);
    let right_is_dir = is_dir(&right_path);
    let folder_mode = if cli.dir {
        true
    } else if cli.file {
        false
    } else {
        left_is_dir && right_is_dir
    };

    if folder_mode {
        return dispatch_folder(
            &cli.left,
            &right_path,
            cli.no_gui,
            cli.quiet,
            cli.follow_symlinks,
        );
    }

    warn_if_large(&cli.left);
    warn_if_large(&right_path);
    let left = DiffDocument::load(&cli.left)?;
    let right = DiffDocument::load(&right_path)?;

    if cli.no_gui {
        return dispatch_no_gui(&left, &right);
    }

    let identical_at_load = !left.is_binary
        && !right.is_binary
        && left.content == right.content
        && left.is_binary == right.is_binary;

    if identical_at_load {
        emit_lgtm(cli.quiet);
        return Ok(EXIT_IDENTICAL);
    }

    let outcome = lgtm_gui::run_diff(
        left,
        right,
        cli.read_only,
        cli.repo.clone(),
        cli.editor.as_deref(),
    )?;
    match outcome {
        GuiOutcome::Identical => {
            emit_lgtm(cli.quiet);
            Ok(EXIT_IDENTICAL)
        }
        GuiOutcome::Differs => Ok(EXIT_DIFFERS),
    }
}

fn dispatch_no_gui(left: &DiffDocument, right: &DiffDocument) -> anyhow::Result<u8> {
    if left.is_binary || right.is_binary {
        if left.is_binary && right.is_binary && left.size_bytes == right.size_bytes {
            // best-effort: same size MIGHT mean identical, but we don't slurp
            // twice; for --no-gui, just say they differ if either is binary.
            println!(
                "Binary files {} and {} differ",
                left.path.display(),
                right.path.display()
            );
            return Ok(EXIT_DIFFERS);
        }
        println!(
            "Binary files {} and {} differ",
            left.path.display(),
            right.path.display()
        );
        return Ok(EXIT_DIFFERS);
    }

    if left.content == right.content {
        return Ok(EXIT_IDENTICAL);
    }

    let out = unified_diff(
        &left.content,
        &right.content,
        &left.path.display().to_string(),
        &right.path.display().to_string(),
        3,
    );
    let stdout = std::io::stdout();
    stdout.lock().write_all(out.as_bytes())?;
    Ok(EXIT_DIFFERS)
}

fn dispatch_merge(cli: Cli) -> anyhow::Result<u8> {
    let base_path = cli
        .right
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--merge requires LOCAL BASE REMOTE positional args"))?;
    let remote_path = cli
        .remote
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--merge requires LOCAL BASE REMOTE positional args"))?;
    let output = cli
        .output
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--merge requires --output"))?;

    let local = DiffDocument::load(&cli.left)?;
    let base = DiffDocument::load(&base_path)?;
    let remote = DiffDocument::load(&remote_path)?;
    let merge = ThreeWayMerge::compute(local, base, remote)?;

    // Headless mode for end-to-end git tests: bypass the GUI.
    // LGTM_HEADLESS_MERGE=auto|take-local|take-remote|abort
    if let Ok(mode) = std::env::var("LGTM_HEADLESS_MERGE") {
        return dispatch_merge_headless(merge, output, &mode, cli.quiet);
    }

    let outcome = lgtm_gui::run_merge(merge, output)?;
    match outcome {
        GuiOutcome::Identical => {
            emit_lgtm(cli.quiet);
            Ok(EXIT_IDENTICAL)
        }
        GuiOutcome::Differs => Ok(EXIT_DIFFERS),
    }
}

fn dispatch_merge_headless(
    mut merge: ThreeWayMerge,
    output: PathBuf,
    mode: &str,
    quiet: bool,
) -> anyhow::Result<u8> {
    use lgtm_core::{MergeRegion, Side};
    match mode {
        "abort" => Ok(EXIT_DIFFERS),
        "auto" => {
            // Save only if all conflicts are already auto-resolved.
            if merge.unresolved_conflicts() > 0 {
                return Ok(EXIT_DIFFERS);
            }
            let text = merge.render()?;
            std::fs::write(&output, text)?;
            emit_lgtm(quiet);
            Ok(EXIT_IDENTICAL)
        }
        "take-local" | "take-remote" => {
            let take = if mode == "take-local" {
                Side::Left
            } else {
                Side::Right
            };
            for r in &mut merge.regions {
                if let MergeRegion::Conflict {
                    local,
                    remote,
                    resolution,
                    ..
                } = r
                {
                    *resolution = Some(if take == Side::Left {
                        local.clone()
                    } else {
                        remote.clone()
                    });
                }
            }
            let text = merge.render()?;
            std::fs::write(&output, text)?;
            emit_lgtm(quiet);
            Ok(EXIT_IDENTICAL)
        }
        other => anyhow::bail!(
            "LGTM_HEADLESS_MERGE: unknown mode {other:?} (expected auto|take-local|take-remote|abort)"
        ),
    }
}

fn dispatch_folder(
    left: &Path,
    right: &Path,
    no_gui: bool,
    quiet: bool,
    follow_symlinks: bool,
) -> anyhow::Result<u8> {
    let opts = FolderDiffOptions {
        follow_symlinks,
        respect_ignore: true,
    };
    let diff = FolderDiff::compute(left, right, &opts)?;

    let any_differ = diff
        .entries
        .iter()
        .any(|e| e.status != lgtm_core::FolderEntryStatus::Identical);

    if no_gui {
        use lgtm_core::FolderEntryStatus;
        for e in &diff.entries {
            let marker = match e.status {
                FolderEntryStatus::Identical => '=',
                FolderEntryStatus::Modified => '~',
                FolderEntryStatus::LeftOnly => '<',
                FolderEntryStatus::RightOnly => '>',
                FolderEntryStatus::TypeChanged => '!',
                FolderEntryStatus::BinaryDiffers => 'b',
            };
            if matches!(e.status, FolderEntryStatus::Identical) {
                continue;
            }
            println!("{marker} {}", e.relative_path.display());
        }
        return Ok(if any_differ {
            EXIT_DIFFERS
        } else {
            emit_lgtm(quiet);
            EXIT_IDENTICAL
        });
    }

    if !any_differ {
        emit_lgtm(quiet);
        return Ok(EXIT_IDENTICAL);
    }

    let outcome = lgtm_gui::run_folder(diff)?;
    Ok(match outcome {
        GuiOutcome::Identical => {
            emit_lgtm(quiet);
            EXIT_IDENTICAL
        }
        GuiOutcome::Differs => EXIT_DIFFERS,
    })
}

fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn warn_if_large(p: &Path) {
    if let Ok(m) = std::fs::metadata(p) {
        if m.is_file() && m.len() > SOFT_SIZE_LIMIT {
            eprintln!(
                "lgtm: warning: {} is {} MB; loading may take a while",
                p.display(),
                m.len() / (1024 * 1024)
            );
        }
    }
}

fn emit_lgtm(quiet: bool) {
    if !quiet {
        let _ = writeln!(std::io::stderr(), "LGTM ✓");
    }
}
