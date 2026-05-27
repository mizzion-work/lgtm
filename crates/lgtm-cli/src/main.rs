//! `lgtm` — entry point.
//!
//! Parses arguments, dispatches to the GUI or to stdout, and maps the
//! result to a process exit code per the CLI contract documented in README.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use lgtm_core::{unified_diff, DiffDocument, ThreeWayMerge};
use lgtm_gui::GuiOutcome;

const EXIT_IDENTICAL: u8 = 0;
const EXIT_DIFFERS: u8 = 1;
const EXIT_ERROR: u8 = 2;

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
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LGTM_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

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
        return dispatch_folder(&cli.left, &right_path);
    }

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

    let outcome = lgtm_gui::run_diff(left, right, cli.read_only)?;
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

fn dispatch_folder(left: &Path, right: &Path) -> anyhow::Result<u8> {
    let _ = (left, right);
    anyhow::bail!("folder mode is not implemented yet (lands in step 9)")
}

fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn emit_lgtm(quiet: bool) {
    if !quiet {
        let _ = writeln!(std::io::stderr(), "LGTM ✓");
    }
}
