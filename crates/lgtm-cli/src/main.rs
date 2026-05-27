//! `lgtm` — entry point.
//!
//! Parses arguments, dispatches to the GUI or to stdout diff, and maps the
//! result to a process exit code per the CLI contract documented in README.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// `lgtm` — from wtf to lgtm.
#[derive(Debug, Parser)]
#[command(name = "lgtm", version, about, long_about = None)]
struct Cli {
    /// Left side (or LOCAL in --merge mode).
    left: PathBuf,
    /// Right side (or BASE in --merge mode).
    right: Option<PathBuf>,
    /// REMOTE — only used with --merge.
    remote: Option<PathBuf>,

    /// Three-way merge mode. Requires three positional args and --output.
    #[arg(long)]
    merge: bool,
    /// Where to write the merged result (only with --merge).
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
    // Real implementation lands in step 5.
    let _ = Cli::parse();
    ExitCode::from(2)
}
